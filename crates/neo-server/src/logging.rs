//! Log output of the command line: text lines on stderr, which systemd or
//! Docker collect.

use std::{fmt, io::IsTerminal};

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{
    EnvFilter,
    fmt::{FmtContext, FormatEvent, FormatFields, format::Writer},
    registry::LookupSpan,
};

/// Installs the global subscriber. `RUST_LOG`, when set, overrides `level`
/// for debugging.
pub fn init(level: Level) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level.as_str().to_ascii_lowercase()));
    let stderr = std::io::stderr();
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        // journald stores escape codes verbatim, so colour only on a terminal.
        .with_ansi(stderr.is_terminal())
        .event_format(Priority {
            inner: tracing_subscriber::fmt::format(),
            journal: stderr_is_journal(),
        })
        .init();
}

/// Prefixes each line with its syslog priority when stderr is the journal.
/// journald stores every line of a stream at info otherwise, and
/// `journalctl -p warning` would find nothing. systemd strips the prefix
/// (`SyslogLevelPrefix=`, on by default).
pub struct Priority<F> {
    pub inner: F,
    pub journal: bool,
}

impl<S, N, F> FormatEvent<S, N> for Priority<F>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    F: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        if !self.journal {
            return self.inner.format_event(ctx, writer, event);
        }
        // stderr is not a terminal here, so the line has no colour to keep.
        let mut text = String::new();
        self.inner
            .format_event(ctx, Writer::new(&mut text), event)?;
        writer.write_str(&prefixed(syslog_priority(*event.metadata().level()), &text))
    }
}

/// `text` for stderr at error priority, for what `main` prints outside the
/// subscriber.
pub fn error_lines(text: &str) -> String {
    if stderr_is_journal() {
        prefixed(3, text)
    } else {
        text.to_owned()
    }
}

/// journald files each line on its own, so a message that spans lines, a
/// TOML error with its snippet say, needs the prefix on every one.
fn prefixed(priority: u8, text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| format!("<{priority}>{line}"))
        .collect()
}

fn syslog_priority(level: Level) -> u8 {
    match level {
        Level::ERROR => 3,
        Level::WARN => 4,
        Level::INFO => 6,
        Level::DEBUG | Level::TRACE => 7,
    }
}

/// systemd sets `JOURNAL_STREAM` to the device and inode of the journal
/// stream. A child that inherited the variable but writes elsewhere, a
/// redirect to a file say, must not get the prefix.
#[cfg(unix)]
fn stderr_is_journal() -> bool {
    use std::os::fd::AsFd;

    let Ok(fd) = std::io::stderr().as_fd().try_clone_to_owned() else {
        return false;
    };
    let stream = std::env::var_os("JOURNAL_STREAM");
    is_journal(stream.as_deref(), &std::fs::File::from(fd))
}

#[cfg(not(unix))]
fn stderr_is_journal() -> bool {
    false
}

#[cfg(unix)]
fn is_journal(stream: Option<&std::ffi::OsStr>, file: &std::fs::File) -> bool {
    use std::os::unix::fs::MetadataExt;

    let (Some(stream), Ok(meta)) = (stream, file.metadata()) else {
        return false;
    };
    stream.to_str() == Some(&format!("{}:{}", meta.dev(), meta.ino()))
}

/// Log lines written during a test.
///
/// Each test thread reads only its own lines, through one global subscriber
/// for the whole test binary. Thread-local subscribers in parallel tests
/// race on tracing's global callsite cache and lose events now and then.
#[cfg(test)]
pub(crate) mod captured {
    use std::{
        cell::RefCell,
        io,
        sync::{Arc, Mutex, Once},
    };

    use tracing_subscriber::{
        filter::{Targets, dynamic_filter_fn},
        fmt::MakeWriter,
        prelude::*,
    };

    thread_local! {
        static LINES: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        /// Nothing is captured on a thread until its test asks.
        static FILTER: RefCell<Targets> = RefCell::new(Targets::new());
    }

    struct ThisThread;

    impl io::Write for ThisThread {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            LINES.with(|lines| lines.borrow_mut().extend_from_slice(buf));
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Installs the global subscriber. Tests that set their own subscriber
    /// for a scope call it too, so the cache never sees only theirs.
    pub(crate) fn install() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let layer = tracing_subscriber::fmt::layer()
                .with_writer(|| ThisThread)
                .with_ansi(false)
                .without_time()
                .with_filter(dynamic_filter_fn(|meta, _| {
                    FILTER.with(|f| f.borrow().would_enable(meta.target(), meta.level()))
                }));
            tracing_subscriber::registry().with(layer).init();
        });
    }

    /// This thread's lines from now on.
    pub(crate) struct Captured;

    impl Captured {
        pub(crate) fn lines(&self) -> Vec<String> {
            let bytes = LINES.with(|lines| lines.borrow().clone());
            String::from_utf8(bytes)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    /// Captures this thread's INFO and above as plain lines, without time
    /// or colour.
    pub(crate) fn start() -> Captured {
        start_filtered("info")
    }

    /// Like [`start`] with `RUST_LOG`-style directives.
    pub(crate) fn start_filtered(directives: &str) -> Captured {
        install();
        let targets = directives.parse().expect("valid directives");
        FILTER.with(|f| *f.borrow_mut() = targets);
        LINES.with(|lines| lines.borrow_mut().clear());
        Captured
    }

    /// A writer for a subscriber that a test sets for a scope.
    #[derive(Clone, Default)]
    pub(crate) struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Buffer {
        pub(crate) fn lines(&self) -> Vec<String> {
            let bytes = self.0.lock().unwrap().clone();
            String::from_utf8(bytes)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Buffer;

        fn make_writer(&'a self) -> Buffer {
            self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        captured::{self, Buffer},
        *,
    };

    fn lines(journal: bool) -> Vec<String> {
        captured::install();
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(Level::TRACE)
            .with_writer(buffer.clone())
            .with_ansi(false)
            .event_format(Priority {
                inner: tracing_subscriber::fmt::format().without_time(),
                journal,
            })
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!("e");
            tracing::warn!("w");
            tracing::info!("i");
            tracing::debug!("d");
            tracing::trace!("t");
        });
        buffer.lines()
    }

    #[test]
    fn journal_lines_carry_their_syslog_priority() {
        let prefixes: Vec<_> = lines(true)
            .iter()
            .map(|line| line[..3].to_owned())
            .collect();
        assert_eq!(prefixes, ["<3>", "<4>", "<6>", "<7>", "<7>"]);
    }

    #[test]
    fn every_line_of_a_multiline_message_has_the_prefix() {
        captured::install();
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buffer.clone())
            .with_ansi(false)
            .event_format(Priority {
                inner: tracing_subscriber::fmt::format().without_time(),
                journal: true,
            })
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!("first\nsecond\nthird");
        });
        let lines = buffer.lines();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines.iter().all(|line| line.starts_with("<4>")),
            "{lines:?}"
        );
        assert_eq!(prefixed(3, "a\nb\n"), "<3>a\n<3>b\n");
        assert_eq!(prefixed(3, "a"), "<3>a");
    }

    #[test]
    fn other_lines_have_no_prefix() {
        let lines = lines(false);
        assert_eq!(lines.len(), 5);
        assert!(lines.iter().all(|line| !line.starts_with('<')), "{lines:?}");
        assert!(lines[1].contains("WARN"), "{lines:?}");
    }

    #[cfg(unix)]
    #[test]
    fn journal_is_recognised_only_by_its_device_and_inode() {
        use std::{ffi::OsStr, os::unix::fs::MetadataExt};

        let file = tempfile::tempfile().unwrap();
        let other = tempfile::tempfile().unwrap();
        let meta = file.metadata().unwrap();
        let stream = format!("{}:{}", meta.dev(), meta.ino());
        assert!(is_journal(Some(OsStr::new(&stream)), &file));
        assert!(!is_journal(Some(OsStr::new(&stream)), &other));
        assert!(!is_journal(None, &file));
    }
}

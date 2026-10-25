//! The shell's log (book/src/desktop.md, "日志"). A release build on
//! Windows has no console, and an app opened from the macOS Finder has
//! nowhere for stderr to go, so the log is also written to daily files in
//! the system's log folder.

use std::{io::IsTerminal, path::Path};

use tracing_appender::rolling::{InitError, RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// One file per day the app runs. Opening the appender deletes the oldest
/// files beyond this many, so the folder never grows without end.
const KEEP_FILES: usize = 7;

/// Installs the global subscriber: stderr, plus the files in `dir` when it
/// can be opened. `RUST_LOG` overrides the default level, info.
pub fn init(dir: Option<&Path>) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let (files, failed) = match dir.map(appender) {
        Some(Ok(files)) => (Some(files), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let to_files = files.map(|files| fmt::layer().with_writer(files).with_ansi(false));
    let to_stderr = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal());
    tracing_subscriber::registry()
        .with(filter)
        .with(to_stderr)
        .with(to_files)
        .init();
    if let Some(e) = failed {
        tracing::warn!(error = %e, "cannot write the log files");
    } else if dir.is_none() {
        tracing::warn!("no log folder on this system, logging to stderr only");
    }
}

fn appender(dir: &Path) -> Result<RollingFileAppender, InitError> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("kikoeru-neo")
        .filename_suffix("log")
        .max_log_files(KEEP_FILES)
        .build(dir)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn writes_dated_files_and_keeps_only_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        for day in 1..=9 {
            let name = format!("kikoeru-neo.2026-01-0{day}.log");
            std::fs::write(dir.path().join(name), "old\n").unwrap();
        }
        let mut files = appender(dir.path()).unwrap();
        files.write_all(b"today\n").unwrap();
        files.flush().unwrap();
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names.len(), KEEP_FILES, "{names:?}");
        let today = names.last().unwrap();
        assert!(
            today.starts_with("kikoeru-neo.") && today.ends_with(".log"),
            "{names:?}"
        );
        let written = std::fs::read_to_string(dir.path().join(today)).unwrap();
        assert!(written.contains("today"), "{written:?}");
    }
}

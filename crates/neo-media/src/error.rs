use std::io;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("cannot start {program}: {source}")]
    Spawn {
        program: &'static str,
        source: io::Error,
    },
    #[error("{program} exited with {status}: {stderr}")]
    Failed {
        program: &'static str,
        status: String,
        /// The last part of stderr, for the log and the task message.
        stderr: String,
    },
    #[error("{0} timed out")]
    Timeout(&'static str),
    #[error("cancelled")]
    Cancelled,
    #[error("cannot read ffprobe output: {0}")]
    Parse(String),
    #[error("the output has no playable audio")]
    BadOutput,
    #[error("the source file changed while it was being processed")]
    SourceChanged,
    #[error("free disk space is below the configured minimum")]
    InsufficientStorage,
    #[error(transparent)]
    Io(#[from] io::Error),
}

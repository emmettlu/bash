use std::path::PathBuf;

/// Represents an error encountered while running or otherwise managing an interactive shell.
#[derive(thiserror::Error, Debug)]
pub enum ShellError {
    /// An error occurred with the embedded shell.
    #[error("{0}")]
    ShellError(#[from] crate::engine::Error),

    /// A generic I/O error occurred.
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Failed to create xtrace file.
    #[error("failed to create xtrace file '{0}': {1}")]
    FailedToCreateXtraceFile(PathBuf, std::io::Error),
}

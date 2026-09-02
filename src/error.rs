//! Error types for tailcat-node.

use std::io;
use thiserror::Error;

/// All fallible operations return [`Result<T, Error>`].
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("TOML decode error: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("TOML encode error: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Not initialized: {0}. Run `tailcat-node init` first.")]
    NotInitialized(String),

    #[error("Already initialized: {0}")]
    AlreadyInitialized(String),

    #[error("Peer not found: {0}")]
    PeerNotFound(String),

    #[error("Service not found: {0}")]
    ServiceNotFound(String),

    #[error("Identity error: {0}")]
    Identity(String),

    #[error("Daemon error: {0}")]
    Daemon(String),

    #[error("IPC error: {0}")]
    Ipc(String),

    #[error("Tailcat backend error: {0}")]
    Backend(String),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Not running: daemon not reachable at {0}")]
    NotRunning(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl Error {
    /// Exit code for the CLI to use when bubbling this error up.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::NotInitialized(_) => 2,
            Error::NotRunning(_) => 3,
            Error::PeerNotFound(_) | Error::ServiceNotFound(_) => 4,
            Error::InvalidArgument(_) => 5,
            _ => 1,
        }
    }
}

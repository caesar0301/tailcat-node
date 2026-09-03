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

    /// Return the inner message without the error-variant prefix.
    ///
    /// `Error::Backend("foo")` Display is "Tailcat backend error: foo".
    /// This returns just "foo", useful when serializing errors for IPC
    /// so the client doesn't see a double-wrapped prefix.
    pub fn inner_message(&self) -> String {
        match self {
            Error::Io(e) => e.to_string(),
            Error::TomlDe(e) => e.to_string(),
            Error::TomlSer(e) => e.to_string(),
            Error::Json(e) => e.to_string(),
            Error::NotInitialized(s) => s.clone(),
            Error::AlreadyInitialized(s) => s.clone(),
            Error::PeerNotFound(s) => s.clone(),
            Error::ServiceNotFound(s) => s.clone(),
            Error::Identity(s) => s.clone(),
            Error::Daemon(s) => s.clone(),
            Error::Ipc(s) => s.clone(),
            Error::Backend(s) => s.clone(),
            Error::InvalidArgument(s) => s.clone(),
            Error::NotRunning(s) => s.clone(),
            Error::Other(e) => e.to_string(),
        }
    }
}

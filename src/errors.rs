use std::io;

/// An operational failure while communicating with M2 or driving the terminal.
/// Syntax failures are not here: they are a verdict about the user's input
/// (a parser verdict), not a fault in the session.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    System(#[from] io::Error),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    #[error(transparent)]
    Theme(#[from] toml::de::Error),
    #[error(transparent)]
    Transport(#[from] jsonrpc::Error),
    #[error("M2 process exited: {0}")]
    Process(String),
    #[error("timed out after {0} seconds")]
    Timeout(u64),
}

/// The result type shared by Spectral's fallible operations.
pub(crate) type Result<T> = std::result::Result<T, Error>;

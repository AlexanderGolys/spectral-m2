use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;

#[derive(Debug)]
#[non_exhaustive]
#[expect(
    dead_code,
    reason = "the error taxonomy includes frontend and shell failures not emitted by the current backend yet"
)]
pub enum M2Error {
    SystemError(io::Error),
    SerializationError(serde_json::Error),
    TransmitionError(jsonrpc::Error),
    M2ProcessError { status: String, stderr: String },
    Timeout(u64),
    ParserError(String),
    SyntaxError(String),
    RuntimeError(String),
    ShellError(String),
    UnknownError(String),
}

impl Display for M2Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::SystemError(error) => write!(f, "SystemError: {error}"),
            Self::SerializationError(error) => write!(f, "SerializationError: {error}"),
            Self::TransmitionError(error) => write!(f, "TransmitionError: {error}"),
            Self::M2ProcessError { status, stderr } if stderr.is_empty() => {
                write!(f, "M2 process exited with code {status}")
            }
            Self::M2ProcessError { status, stderr } => {
                write!(f, "M2 process exited with code {status} ({stderr})")
            }
            Self::Timeout(seconds) => write!(f, "Timed out after {seconds} seconds"),
            Self::ParserError(message) => write!(f, "ParseError: {message}"),
            Self::SyntaxError(message) => write!(f, "SyntaxError: {message}"),
            Self::RuntimeError(message) => write!(f, "RuntimeError: {message}"),
            Self::ShellError(message) => write!(f, "ShellError: {message}"),
            Self::UnknownError(message) => write!(f, "UnknownError: {message}"),
        }
    }
}

impl Error for M2Error {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SystemError(error) => Some(error),
            Self::SerializationError(error) => Some(error),
            Self::TransmitionError(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for M2Error {
    fn from(error: io::Error) -> Self {
        Self::SystemError(error)
    }
}

impl From<serde_json::Error> for M2Error {
    fn from(error: serde_json::Error) -> Self {
        Self::SerializationError(error)
    }
}

impl From<jsonrpc::Error> for M2Error {
    fn from(error: jsonrpc::Error) -> Self {
        Self::TransmitionError(error)
    }
}

pub type M2Result<T> = Result<T, M2Error>;

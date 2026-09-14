use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum OrionRuntimeError {
    InvalidArgument(String),
    InvalidState(String),
    OutOfRange(String),
    Runtime(String),
    Io(std::io::Error),
    Json(serde_json::Error),
    Yaml(serde_yaml::Error),
}

// Display implementation for OrionRuntimeError
impl Display for OrionRuntimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgument(message)
            | Self::InvalidState(message)
            | Self::OutOfRange(message)
            | Self::Runtime(message) => formatter.write_str(message),
            Self::Io(error) => Display::fmt(error, formatter),
            Self::Json(error) => Display::fmt(error, formatter),
            Self::Yaml(error) => Display::fmt(error, formatter),
        }
    }
}

// Error implementation for OrionRuntimeError
// means that OrionRuntimeError satisfies Rust standard error trait/interface
// the Error trait requires us to impl Debug and Display, we've already done that
// with the Display implementation above and the #[derive(Debug)]
impl std::error::Error for OrionRuntimeError {}

impl From<std::io::Error> for OrionRuntimeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for OrionRuntimeError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<serde_yaml::Error> for OrionRuntimeError {
    fn from(error: serde_yaml::Error) -> Self {
        Self::Yaml(error)
    }
}

pub type Result<T> = std::result::Result<T, OrionRuntimeError>;

pub use OrionRuntimeError as Error;

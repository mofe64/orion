use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum Error {
    InvalidArgument(String),
    InvalidState(String),
    OutOfRange(String),
    Runtime(String),
    Io(std::io::Error),
    Json(serde_json::Error),
    Yaml(serde_yaml::Error),
}

impl Display for Error {
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

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<serde_yaml::Error> for Error {
    fn from(error: serde_yaml::Error) -> Self {
        Self::Yaml(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

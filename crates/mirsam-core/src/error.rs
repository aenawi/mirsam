//! Domain error type.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unsupported or malformed document: {0}")]
    Format(String),

    /// The path does not exist. Carries no path of its own: the caller adds it
    /// as context, and repeating it reads as a stutter.
    #[error("no such file")]
    NotFound,

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("refusing to overwrite the source document; choose a distinct output path")]
    WouldOverwriteSource,

    #[error("no adapter for extension {0:?}")]
    UnknownFormat(String),
}

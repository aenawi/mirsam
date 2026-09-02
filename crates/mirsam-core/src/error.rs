//! Domain error type.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unsupported or malformed document: {0}")]
    Format(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("refusing to overwrite the source document; choose a distinct output path")]
    WouldOverwriteSource,

    #[error("no adapter for extension {0:?}")]
    UnknownFormat(String),
}

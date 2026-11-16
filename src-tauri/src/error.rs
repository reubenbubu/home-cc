use serde::{Serialize, Serializer};

/// One error type for every command, so the screens always get a readable message.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("keychain error: {0}")]
    Keychain(#[from] keyring::Error),
    #[error("{0}")]
    Invalid(String),
    /// Scanning or pinging failed. The text is shown to the owner as it is.
    #[error("{0}")]
    Network(String),
    #[error("not found: {0}")]
    NotFound(String),
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

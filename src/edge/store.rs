use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, PartialEq, Eq)]
pub enum ObjectStoreError {
    Invalid(String),
    Conflict(String),
    Missing(String),
    Unavailable(String),
}

impl Display for ObjectStoreError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid object-store state: {message}"),
            Self::Conflict(message) => write!(formatter, "object-store conflict: {message}"),
            Self::Missing(message) => {
                write!(formatter, "object-store object is missing: {message}")
            }
            Self::Unavailable(message) => {
                write!(formatter, "object-store is unavailable: {message}")
            }
        }
    }
}

impl Error for ObjectStoreError {}

impl From<crate::xbf::XbfError> for ObjectStoreError {
    fn from(error: crate::xbf::XbfError) -> Self {
        Self::Invalid(error.to_string())
    }
}

impl From<serde_json::Error> for ObjectStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Invalid(format!("JSON encoding failed: {error}"))
    }
}

pub trait ObjectStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, ObjectStoreError>;
    fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<(), ObjectStoreError>;
    fn compare_and_swap(
        &self,
        key: &str,
        expected: Option<&[u8]>,
        replacement: &[u8],
    ) -> Result<(), ObjectStoreError>;
    fn delete(&self, key: &str) -> Result<(), ObjectStoreError>;
    fn list(&self, prefix: &str) -> Result<Vec<String>, ObjectStoreError>;
}

pub(super) fn validate_key(key: &str) -> Result<(), ObjectStoreError> {
    if key.is_empty() || key.contains('\0') {
        return Err(ObjectStoreError::Invalid(
            "object key must be non-empty and must not contain NUL".into(),
        ));
    }
    Ok(())
}

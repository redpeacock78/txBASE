use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, PartialEq, Eq)]
pub enum ObjectStoreError {
    Invalid(String),
    Conflict(String),
    Missing(String),
    Unavailable(String),
    Cancelled(String),
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
            Self::Cancelled(message) => {
                write!(formatter, "object-store operation was cancelled: {message}")
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

pub type AsyncObjectStoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

pub trait AsyncObjectStore {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Option<Vec<u8>>, ObjectStoreError>>;
    fn put_if_absent<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>>;
    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a [u8]>,
        replacement: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>>;
    fn delete<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>>;
    fn list<'a>(
        &'a self,
        prefix: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Vec<String>, ObjectStoreError>>;
}

#[derive(Clone)]
pub struct SyncObjectStoreAdapter<S> {
    inner: S,
}

impl<S> SyncObjectStoreAdapter<S> {
    pub fn new(inner: S) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: ObjectStore> AsyncObjectStore for SyncObjectStoreAdapter<S> {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Option<Vec<u8>>, ObjectStoreError>> {
        Box::pin(std::future::ready(self.inner.get(key)))
    }

    fn put_if_absent<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(std::future::ready(self.inner.put_if_absent(key, bytes)))
    }

    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a [u8]>,
        replacement: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(std::future::ready(self.inner.compare_and_swap(
            key,
            expected,
            replacement,
        )))
    }

    fn delete<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(std::future::ready(self.inner.delete(key)))
    }

    fn list<'a>(
        &'a self,
        prefix: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Vec<String>, ObjectStoreError>> {
        Box::pin(std::future::ready(self.inner.list(prefix)))
    }
}

pub(super) fn validate_key(key: &str) -> Result<(), ObjectStoreError> {
    if key.is_empty() || key.contains('\0') {
        return Err(ObjectStoreError::Invalid(
            "object key must be non-empty and must not contain NUL".into(),
        ));
    }
    Ok(())
}

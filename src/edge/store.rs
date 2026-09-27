use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::future::Future;
use std::pin::Pin;

use super::cancellation::CancellationToken;

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

    /// Read one object, dropping the read future after cancellation is polled.
    ///
    /// Whether this also stops underlying I/O depends on the store implementation.
    fn get_with_cancellation<'a>(
        &'a self,
        key: &'a str,
        cancellation: &'a CancellationToken,
    ) -> AsyncObjectStoreFuture<'a, Result<Option<Vec<u8>>, ObjectStoreError>> {
        if cancellation.is_cancelled() {
            return cancelled_future();
        }
        cancelable(self.get(key), cancellation)
    }

    /// Write one immutable object unless cancellation was already requested.
    ///
    /// Cancellation does not stop an accepted write; keep polling its future to observe its
    /// result.
    fn put_if_absent_with_cancellation<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
        cancellation: &'a CancellationToken,
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        if cancellation.is_cancelled() {
            return cancelled_future();
        }
        self.put_if_absent(key, bytes)
    }

    /// Compare and swap one object unless cancellation was already requested.
    ///
    /// Cancellation does not stop an accepted write; keep polling its future to observe its
    /// result.
    fn compare_and_swap_with_cancellation<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a [u8]>,
        replacement: &'a [u8],
        cancellation: &'a CancellationToken,
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        if cancellation.is_cancelled() {
            return cancelled_future();
        }
        self.compare_and_swap(key, expected, replacement)
    }

    /// Delete one object unless cancellation was already requested.
    ///
    /// Cancellation does not stop an accepted write; keep polling its future to observe its
    /// result.
    fn delete_with_cancellation<'a>(
        &'a self,
        key: &'a str,
        cancellation: &'a CancellationToken,
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        if cancellation.is_cancelled() {
            return cancelled_future();
        }
        self.delete(key)
    }

    /// List objects, dropping the list future after cancellation is polled.
    ///
    /// Whether this also stops underlying I/O depends on the store implementation.
    fn list_with_cancellation<'a>(
        &'a self,
        prefix: &'a str,
        cancellation: &'a CancellationToken,
    ) -> AsyncObjectStoreFuture<'a, Result<Vec<String>, ObjectStoreError>> {
        if cancellation.is_cancelled() {
            return cancelled_future();
        }
        cancelable(self.list(prefix), cancellation)
    }
}

pub(crate) struct CancellableObjectStore<'a, S: ?Sized> {
    inner: &'a S,
    cancellation: CancellationToken,
}

impl<'a, S: ?Sized> CancellableObjectStore<'a, S> {
    pub(crate) fn new(inner: &'a S, cancellation: CancellationToken) -> Self {
        Self {
            inner,
            cancellation,
        }
    }
}

impl<S: AsyncObjectStore + ?Sized> AsyncObjectStore for CancellableObjectStore<'_, S> {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Option<Vec<u8>>, ObjectStoreError>> {
        self.inner.get_with_cancellation(key, &self.cancellation)
    }

    fn put_if_absent<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        self.inner
            .put_if_absent_with_cancellation(key, bytes, &self.cancellation)
    }

    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a [u8]>,
        replacement: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        self.inner.compare_and_swap_with_cancellation(
            key,
            expected,
            replacement,
            &self.cancellation,
        )
    }

    fn delete<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        self.inner.delete_with_cancellation(key, &self.cancellation)
    }

    fn list<'a>(
        &'a self,
        prefix: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Vec<String>, ObjectStoreError>> {
        self.inner
            .list_with_cancellation(prefix, &self.cancellation)
    }
}

fn cancelable<'a, T, F>(
    future: F,
    cancellation: &CancellationToken,
) -> AsyncObjectStoreFuture<'a, Result<T, ObjectStoreError>>
where
    F: Future<Output = Result<T, ObjectStoreError>> + 'a,
    T: 'a,
{
    let future = cancellation.abortable(future);
    Box::pin(async move {
        match future.await {
            Ok(result) => result,
            Err(_) => Err(cancelled_error()),
        }
    })
}

fn cancelled_future<'a, T: 'a>() -> AsyncObjectStoreFuture<'a, Result<T, ObjectStoreError>> {
    Box::pin(std::future::ready(Err(cancelled_error())))
}

fn cancelled_error() -> ObjectStoreError {
    ObjectStoreError::Cancelled("operation cancelled".into())
}

pub(super) fn put_if_absent_or_matching<S: ObjectStore>(
    store: &S,
    key: &str,
    bytes: &[u8],
) -> Result<(), ObjectStoreError> {
    match store.put_if_absent(key, bytes) {
        Ok(()) => Ok(()),
        Err(error @ ObjectStoreError::Conflict(_)) => match store.get(key)? {
            Some(existing) if existing.as_slice() == bytes => Ok(()),
            _ => Err(error),
        },
        Err(error) => Err(error),
    }
}

pub(super) async fn put_if_absent_or_matching_async<S: AsyncObjectStore>(
    store: &S,
    key: &str,
    bytes: &[u8],
) -> Result<(), ObjectStoreError> {
    match store.put_if_absent(key, bytes).await {
        Ok(()) => Ok(()),
        Err(error @ ObjectStoreError::Conflict(_)) => match store.get(key).await? {
            Some(existing) if existing.as_slice() == bytes => Ok(()),
            _ => Err(error),
        },
        Err(error) => Err(error),
    }
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

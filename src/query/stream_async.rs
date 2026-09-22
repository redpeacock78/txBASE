use super::{QueryError, QuerySnapshotStream, QueryStream};
use serde_json::Value;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Runtime-neutral polling boundary for a long-lived query stream.
///
/// Implementations may return `Poll::Pending` while waiting for a host event,
/// but must arrange for the supplied waker to be called before polling again.
/// Dropping the pinned stream is the cancellation boundary.
pub trait AsyncQueryStream {
    type Item;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>>;
}

impl<'table, 'request> AsyncQueryStream for QueryStream<'table, 'request> {
    type Item = Result<Value, QueryError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.as_mut().get_mut().next())
    }
}

impl<'request> AsyncQueryStream for QuerySnapshotStream<'request> {
    type Item = Result<Value, QueryError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.as_mut().get_mut().next())
    }
}

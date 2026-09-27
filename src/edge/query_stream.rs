use super::{AsyncObjectStore, AsyncObjectTable, CancellationToken, ObjectStoreError};
use crate::query::{self, AsyncQueryStream, OwnedQuerySnapshotStream, QueryError, QueryRequest};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

type LoadFuture<'table> =
    Pin<Box<dyn Future<Output = Result<OwnedQuerySnapshotStream, QueryError>> + 'table>>;

enum State<'table> {
    Loading(LoadFuture<'table>),
    Ready(Box<OwnedQuerySnapshotStream>),
    Done,
}

/// Query stream that loads one selected committed object-store snapshot before yielding rows.
pub struct AsyncObjectQueryStream<'table> {
    state: State<'table>,
    cancellation: CancellationToken,
    cancellation_waiter: Option<Pin<Box<dyn Future<Output = ()> + 'table>>>,
}

impl<'table> AsyncObjectQueryStream<'table> {
    fn new<S: AsyncObjectStore>(
        table: &'table AsyncObjectTable<S>,
        request: QueryRequest,
        generation: Option<u64>,
    ) -> Result<Self, QueryError> {
        query::validate_stream_request(&request)?;
        let cancellation = CancellationToken::new();
        let waiter_token = cancellation.clone();
        let cancellation_waiter = Box::pin(async move { waiter_token.cancelled().await });
        let cancellable_table = table.with_cancellation(cancellation.clone());
        let load: LoadFuture<'table> =
            Box::pin(
                async move { load_query_snapshot(&cancellable_table, request, generation).await },
            );
        Ok(Self {
            state: State::Loading(load),
            cancellation,
            cancellation_waiter: Some(cancellation_waiter),
        })
    }

    /// Request cancellation of pending reads and subsequent storage operations.
    ///
    /// An accepted mutation is allowed to resolve before the stream ends.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Return a clone of this stream's cancellation token.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

impl Drop for AsyncObjectQueryStream<'_> {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

async fn load_query_snapshot<S: AsyncObjectStore>(
    table: &AsyncObjectTable<S>,
    request: QueryRequest,
    generation: Option<u64>,
) -> Result<OwnedQuerySnapshotStream, QueryError> {
    let snapshot = match generation {
        Some(generation) => table
            .read_at(generation)
            .await
            .map_err(storage_query_error)?
            .ok_or_else(|| {
                QueryError::Invalid(format!(
                    "async object table has no retained snapshot at generation {generation}"
                ))
            })?,
        None => table
            .read()
            .await
            .map_err(storage_query_error)?
            .ok_or_else(|| {
                QueryError::Invalid("async object table has no committed snapshot".into())
            })?,
    };
    query::stream_xbf_snapshot_owned(snapshot, request)
}

fn storage_query_error(error: ObjectStoreError) -> QueryError {
    QueryError::Invalid(format!("async object query storage failed: {error}"))
}

impl AsyncQueryStream for AsyncObjectQueryStream<'_> {
    type Item = Result<Value, QueryError>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            let cancellation_notified = this
                .cancellation_waiter
                .as_mut()
                .is_some_and(|waiter| waiter.as_mut().poll(context).is_ready());
            if cancellation_notified {
                this.cancellation_waiter = None;
            }
            let cancellation_requested = cancellation_notified || this.cancellation.is_cancelled();
            if cancellation_requested && !matches!(&this.state, State::Loading(_)) {
                this.state = State::Done;
                return Poll::Ready(None);
            }
            match &mut this.state {
                State::Loading(load) => match load.as_mut().poll(context) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Ok(stream)) => this.state = State::Ready(Box::new(stream)),
                    Poll::Ready(Err(error)) => {
                        this.state = State::Done;
                        if this.cancellation.is_cancelled() {
                            return Poll::Ready(None);
                        }
                        return Poll::Ready(Some(Err(error)));
                    }
                },
                State::Ready(stream) => return Pin::new(stream.as_mut()).poll_next(context),
                State::Done => return Poll::Ready(None),
            }
        }
    }
}

impl<S: AsyncObjectStore> AsyncObjectTable<S> {
    /// Start a stream over one recovered, committed snapshot.
    pub fn query_stream(
        &self,
        request: QueryRequest,
    ) -> Result<AsyncObjectQueryStream<'_>, QueryError> {
        AsyncObjectQueryStream::new(self, request, None)
    }

    /// Start a stream over one retained committed snapshot generation.
    pub fn query_stream_at(
        &self,
        generation: u64,
        request: QueryRequest,
    ) -> Result<AsyncObjectQueryStream<'_>, QueryError> {
        AsyncObjectQueryStream::new(self, request, Some(generation))
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn query_stream_owned(
        &self,
        generation: Option<u64>,
        request: QueryRequest,
    ) -> Result<OwnedQuerySnapshotStream, QueryError> {
        query::validate_stream_request(&request)?;
        load_query_snapshot(self, request, generation).await
    }
}

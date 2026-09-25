use super::{AsyncObjectStore, AsyncObjectTable, ObjectStoreError};
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
}

impl<'table> AsyncObjectQueryStream<'table> {
    fn new<S: AsyncObjectStore>(
        table: &'table AsyncObjectTable<S>,
        request: QueryRequest,
        generation: Option<u64>,
    ) -> Result<Self, QueryError> {
        query::validate_stream_request(&request)?;
        let load: LoadFuture<'table> = Box::pin(async move {
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
            let table = snapshot.to_dbf().map_err(|error| {
                QueryError::Invalid(format!("cannot query XBF snapshot: {error}"))
            })?;
            query::stream_query_snapshot_owned(&table, &request)
        });
        Ok(Self {
            state: State::Loading(load),
        })
    }
}

fn storage_query_error(error: ObjectStoreError) -> QueryError {
    QueryError::Invalid(format!("async object query storage failed: {error}"))
}

impl AsyncQueryStream for AsyncObjectQueryStream<'_> {
    type Item = Result<Value, QueryError>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match &mut this.state {
                State::Loading(load) => match load.as_mut().poll(context) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Ok(stream)) => this.state = State::Ready(Box::new(stream)),
                    Poll::Ready(Err(error)) => {
                        this.state = State::Done;
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
}

use futures::future::{AbortHandle, Abortable, Aborted};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};

/// Runtime-neutral, one-way signal for cancelling asynchronous object-store operations.
///
/// Clones share the same cancellation state. Cancellation wakes active futures, which
/// stop when they are polled again.
#[derive(Clone, Default)]
pub struct CancellationToken {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    cancelled: bool,
    next_operation: u64,
    operations: HashMap<u64, AbortHandle>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation and wake tasks polling cancellation-aware operations.
    pub fn cancel(&self) {
        let operations = {
            let mut state = self.state();
            if state.cancelled {
                return;
            }
            state.cancelled = true;
            std::mem::take(&mut state.operations)
        };
        for operation in operations.into_values() {
            operation.abort();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state().cancelled
    }

    /// Resolve when this token is cancelled.
    pub async fn cancelled(&self) {
        let _ = self.abortable(std::future::pending::<()>()).await;
    }

    pub(crate) fn abortable<F: Future>(&self, future: F) -> CancellationFuture<F> {
        let (handle, registration) = AbortHandle::new_pair();
        let operation_id = {
            let mut state = self.state();
            if state.cancelled {
                None
            } else {
                let id = state.next_operation;
                state.next_operation = state.next_operation.wrapping_add(1);
                state.operations.insert(id, handle.clone());
                Some(id)
            }
        };
        if operation_id.is_none() {
            handle.abort();
        }

        CancellationFuture {
            future: Box::pin(Abortable::new(future, registration)),
            token: self.clone(),
            operation_id,
        }
    }

    fn unregister(&self, operation_id: u64) {
        self.state().operations.remove(&operation_id);
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub(crate) struct CancellationFuture<F> {
    future: Pin<Box<Abortable<F>>>,
    token: CancellationToken,
    operation_id: Option<u64>,
}

impl<F> Unpin for CancellationFuture<F> {}

impl<F: Future> Future for CancellationFuture<F> {
    type Output = Result<F::Output, Aborted>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let result = this.future.as_mut().poll(context);
        if result.is_ready() {
            if let Some(operation_id) = this.operation_id.take() {
                this.token.unregister(operation_id);
            }
        }
        result
    }
}

impl<F> Drop for CancellationFuture<F> {
    fn drop(&mut self) {
        if let Some(operation_id) = self.operation_id.take() {
            self.token.unregister(operation_id);
        }
    }
}

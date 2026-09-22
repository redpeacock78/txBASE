use super::stream::{stream_query_snapshot, validate_stream_request};
use super::{AsyncQueryStream, QueryError, QueryRequest};
use serde_json::Value;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, mpsc::SyncSender};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};

/// Native host adapter for a bounded query stream.
///
/// The query runs on one worker thread and `poll_next` only performs a
/// non-blocking channel read. A full channel applies backpressure to the
/// worker, and dropping this stream cancels and joins that worker.
pub struct ThreadedQueryStream {
    receiver: Option<Receiver<Result<Value, QueryError>>>,
    cancelled: Arc<AtomicBool>,
    waker: Arc<Mutex<Option<Waker>>>,
    worker: Option<JoinHandle<()>>,
}

/// Start a native bounded query stream that implements [`AsyncQueryStream`].
pub fn stream_query_threaded(
    table: &super::DbfTable,
    request: &QueryRequest,
    capacity: usize,
) -> Result<ThreadedQueryStream, QueryError> {
    validate_stream_request(request)?;
    if capacity == 0 {
        return Err(QueryError::Invalid(
            "threaded stream capacity must be positive".into(),
        ));
    }

    let table = table.clone();
    let request = request.clone();
    let (sender, receiver) = sync_channel(capacity);
    let cancelled = Arc::new(AtomicBool::new(false));
    let waker = Arc::new(Mutex::new(None));
    let worker_cancelled = Arc::clone(&cancelled);
    let worker_waker = Arc::clone(&waker);
    let worker = thread::Builder::new()
        .name("txbase-query-stream".into())
        .spawn(move || {
            produce(table, request, sender, worker_cancelled, worker_waker);
        })
        .map_err(|error| QueryError::Invalid(format!("failed to spawn query stream: {error}")))?;

    Ok(ThreadedQueryStream {
        receiver: Some(receiver),
        cancelled,
        waker,
        worker: Some(worker),
    })
}

fn produce(
    table: super::DbfTable,
    request: QueryRequest,
    sender: SyncSender<Result<Value, QueryError>>,
    cancelled: Arc<AtomicBool>,
    waker: Arc<Mutex<Option<Waker>>>,
) {
    let stream = match stream_query_snapshot(&table, &request) {
        Ok(stream) => stream,
        Err(error) => {
            let _ = sender.send(Err(error));
            wake(&waker);
            return;
        }
    };

    for item in stream {
        if cancelled.load(Ordering::Acquire) || sender.send(item).is_err() {
            return;
        }
        wake(&waker);
    }
    wake(&waker);
}

fn lock_waker(waker: &Mutex<Option<Waker>>) -> MutexGuard<'_, Option<Waker>> {
    match waker.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn wake(waker: &Mutex<Option<Waker>>) {
    if let Some(waker) = lock_waker(waker).take() {
        waker.wake();
    }
}

fn register(waker: &Mutex<Option<Waker>>, context: &Context<'_>) {
    *lock_waker(waker) = Some(context.waker().clone());
}

fn clear(waker: &Mutex<Option<Waker>>) {
    let _ = lock_waker(waker).take();
}

impl ThreadedQueryStream {
    fn poll_receiver(&mut self, context: &Context<'_>) -> Poll<Option<Result<Value, QueryError>>> {
        let Some(receiver) = self.receiver.as_ref() else {
            return Poll::Ready(None);
        };

        match receiver.try_recv() {
            Ok(item) => {
                clear(&self.waker);
                Poll::Ready(Some(item))
            }
            Err(TryRecvError::Disconnected) => {
                clear(&self.waker);
                Poll::Ready(None)
            }
            Err(TryRecvError::Empty) => {
                register(&self.waker, context);
                match receiver.try_recv() {
                    Ok(item) => {
                        clear(&self.waker);
                        Poll::Ready(Some(item))
                    }
                    Err(TryRecvError::Disconnected) => {
                        clear(&self.waker);
                        Poll::Ready(None)
                    }
                    Err(TryRecvError::Empty) => Poll::Pending,
                }
            }
        }
    }
}

impl AsyncQueryStream for ThreadedQueryStream {
    type Item = Result<Value, QueryError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.as_mut().get_mut().poll_receiver(context)
    }
}

impl Drop for ThreadedQueryStream {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.receiver.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

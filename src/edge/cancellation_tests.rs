use super::tests::block_on;
use super::{
    AsyncObjectStore, AsyncObjectStoreFuture, AsyncObjectTable, CancellationToken,
    MemoryObjectStore, ObjectStore, ObjectStoreError,
};
use crate::query::{AsyncQueryStream, parse};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

struct PendingOperation<T> {
    dropped: Arc<AtomicUsize>,
    output: PhantomData<fn() -> T>,
}

impl<T> PendingOperation<T> {
    fn new(dropped: Arc<AtomicUsize>) -> Self {
        Self {
            dropped,
            output: PhantomData,
        }
    }
}

impl<T> Unpin for PendingOperation<T> {}

impl<T> Future for PendingOperation<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl<T> Drop for PendingOperation<T> {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone)]
struct PendingStore {
    dropped: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
}

impl PendingStore {
    fn pending<'a, T: 'a>(&'a self) -> AsyncObjectStoreFuture<'a, T> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(PendingOperation::new(self.dropped.clone()))
    }
}

impl AsyncObjectStore for PendingStore {
    fn get<'a>(
        &'a self,
        _key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Option<Vec<u8>>, ObjectStoreError>> {
        self.pending()
    }

    fn put_if_absent<'a>(
        &'a self,
        _key: &'a str,
        _bytes: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        self.pending()
    }

    fn compare_and_swap<'a>(
        &'a self,
        _key: &'a str,
        _expected: Option<&'a [u8]>,
        _replacement: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        self.pending()
    }

    fn delete<'a>(
        &'a self,
        _key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        self.pending()
    }

    fn list<'a>(
        &'a self,
        _prefix: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Vec<String>, ObjectStoreError>> {
        self.pending()
    }
}

#[derive(Clone)]
struct RecoveryStore {
    inner: MemoryObjectStore,
    fail_first_delete: Arc<AtomicBool>,
    delay_wal_delete: Arc<AtomicBool>,
    wal_delete_started: Arc<AtomicBool>,
}

impl AsyncObjectStore for RecoveryStore {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Option<Vec<u8>>, ObjectStoreError>> {
        Box::pin(async move { self.inner.get(key) })
    }

    fn put_if_absent<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(async move { self.inner.put_if_absent(key, bytes) })
    }

    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a [u8]>,
        replacement: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(async move { self.inner.compare_and_swap(key, expected, replacement) })
    }

    fn delete<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        if self.fail_first_delete.swap(false, Ordering::SeqCst) {
            return Box::pin(std::future::ready(Err(ObjectStoreError::Unavailable(
                "injected WAL cleanup failure".into(),
            ))));
        }
        if key == "users/wal/0.json" && self.delay_wal_delete.swap(false, Ordering::SeqCst) {
            return Box::pin(DelayedDelete {
                inner: self.inner.clone(),
                key: key.to_owned(),
                started: false,
                wal_delete_started: self.wal_delete_started.clone(),
            });
        }
        Box::pin(async move { self.inner.delete(key) })
    }

    fn list<'a>(
        &'a self,
        prefix: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Vec<String>, ObjectStoreError>> {
        Box::pin(async move { self.inner.list(prefix) })
    }
}

struct DelayedDelete {
    inner: MemoryObjectStore,
    key: String,
    started: bool,
    wal_delete_started: Arc<AtomicBool>,
}

impl Future for DelayedDelete {
    type Output = Result<(), ObjectStoreError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.started {
            self.started = true;
            self.wal_delete_started.store(true, Ordering::SeqCst);
            context.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(self.inner.delete(&self.key))
    }
}

#[derive(Default)]
struct CountWake(AtomicUsize);

impl Wake for CountWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn cancelling_a_pending_query_wakes_polling_and_drops_storage_work() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let table = AsyncObjectTable::new(
        PendingStore {
            dropped: dropped.clone(),
            calls: Arc::new(AtomicUsize::new(0)),
        },
        "users",
    )
    .unwrap();
    let mut stream = Box::pin(table.query_stream(parse(b"{}").unwrap()).unwrap());
    let cancellation = stream.cancellation_token();
    let wake_count = Arc::new(CountWake::default());
    let waker = Waker::from(wake_count.clone());
    let mut context = Context::from_waker(&waker);

    assert!(matches!(
        stream.as_mut().poll_next(&mut context),
        Poll::Pending
    ));
    cancellation.cancel();
    assert!(wake_count.0.load(Ordering::SeqCst) > 0);
    assert!(matches!(
        stream.as_mut().poll_next(&mut context),
        Poll::Ready(None)
    ));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn dropping_a_pending_query_cancels_its_token_and_storage_work() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let table = AsyncObjectTable::new(
        PendingStore {
            dropped: dropped.clone(),
            calls: Arc::new(AtomicUsize::new(0)),
        },
        "users",
    )
    .unwrap();
    let mut stream = Box::pin(table.query_stream(parse(b"{}").unwrap()).unwrap());
    let cancellation = stream.cancellation_token();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(matches!(
        stream.as_mut().poll_next(&mut context),
        Poll::Pending
    ));
    drop(stream);

    assert!(cancellation.is_cancelled());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_waiter_wakes_when_the_token_is_cancelled() {
    let cancellation = CancellationToken::new();
    let wake_count = Arc::new(CountWake::default());
    let waker = Waker::from(wake_count.clone());
    let mut context = Context::from_waker(&waker);
    let mut waiter = Box::pin(cancellation.cancelled());

    assert!(matches!(waiter.as_mut().poll(&mut context), Poll::Pending));
    cancellation.cancel();
    assert!(wake_count.0.load(Ordering::SeqCst) > 0);
    assert!(matches!(
        waiter.as_mut().poll(&mut context),
        Poll::Ready(())
    ));
}

#[test]
fn cancelling_before_first_poll_skips_storage() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let table = AsyncObjectTable::new(
        PendingStore {
            dropped,
            calls: calls.clone(),
        },
        "users",
    )
    .unwrap();
    let mut stream = Box::pin(table.query_stream(parse(b"{}").unwrap()).unwrap());
    stream.cancel();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(matches!(
        stream.as_mut().poll_next(&mut context),
        Poll::Ready(None)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_before_first_operation_poll_skips_store_methods() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let store = PendingStore {
        dropped,
        calls: calls.clone(),
    };
    let cancellation = CancellationToken::new();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    let mut read = Box::pin(store.get_with_cancellation("key", &cancellation));
    let mut write = Box::pin(store.put_if_absent_with_cancellation("key", b"value", &cancellation));
    let mut compare_and_swap =
        Box::pin(store.compare_and_swap_with_cancellation("key", None, b"value", &cancellation));
    let mut delete = Box::pin(store.delete_with_cancellation("key", &cancellation));
    let mut list = Box::pin(store.list_with_cancellation("users/", &cancellation));

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    cancellation.cancel();

    assert!(matches!(
        read.as_mut().poll(&mut context),
        Poll::Ready(Err(ObjectStoreError::Cancelled(_)))
    ));
    assert!(matches!(
        write.as_mut().poll(&mut context),
        Poll::Ready(Err(ObjectStoreError::Cancelled(_)))
    ));
    assert!(matches!(
        compare_and_swap.as_mut().poll(&mut context),
        Poll::Ready(Err(ObjectStoreError::Cancelled(_)))
    ));
    assert!(matches!(
        delete.as_mut().poll(&mut context),
        Poll::Ready(Err(ObjectStoreError::Cancelled(_)))
    ));
    assert!(matches!(
        list.as_mut().poll(&mut context),
        Poll::Ready(Err(ObjectStoreError::Cancelled(_)))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_drops_a_pending_list_operation() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let store = PendingStore {
        dropped: dropped.clone(),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let cancellation = CancellationToken::new();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut list = Box::pin(store.list_with_cancellation("users/", &cancellation));

    assert!(matches!(list.as_mut().poll(&mut context), Poll::Pending));
    cancellation.cancel();
    assert!(matches!(
        list.as_mut().poll(&mut context),
        Poll::Ready(Err(ObjectStoreError::Cancelled(_)))
    ));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_drops_all_registered_read_operations() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let store = PendingStore {
        dropped: dropped.clone(),
        calls: calls.clone(),
    };
    let cancellation = CancellationToken::new();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut get = Box::pin(store.get_with_cancellation("key", &cancellation));
    let mut list = Box::pin(store.list_with_cancellation("users/", &cancellation));

    assert!(matches!(get.as_mut().poll(&mut context), Poll::Pending));
    assert!(matches!(list.as_mut().poll(&mut context), Poll::Pending));
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    cancellation.cancel();

    assert!(matches!(
        get.as_mut().poll(&mut context),
        Poll::Ready(Err(ObjectStoreError::Cancelled(_)))
    ));
    assert!(matches!(
        list.as_mut().poll(&mut context),
        Poll::Ready(Err(ObjectStoreError::Cancelled(_)))
    ));
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
}

#[test]
fn cancellation_does_not_abort_accepted_mutations() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let store = PendingStore {
        dropped: dropped.clone(),
        calls: calls.clone(),
    };
    let cancellation = CancellationToken::new();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut put = Box::pin(store.put_if_absent_with_cancellation("key", b"value", &cancellation));
    let mut compare_and_swap =
        Box::pin(store.compare_and_swap_with_cancellation("key", None, b"value", &cancellation));
    let mut delete = Box::pin(store.delete_with_cancellation("key", &cancellation));

    assert!(matches!(put.as_mut().poll(&mut context), Poll::Pending));
    assert!(matches!(
        compare_and_swap.as_mut().poll(&mut context),
        Poll::Pending
    ));
    assert!(matches!(delete.as_mut().poll(&mut context), Poll::Pending));
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    cancellation.cancel();

    assert!(matches!(put.as_mut().poll(&mut context), Poll::Pending));
    assert!(matches!(
        compare_and_swap.as_mut().poll(&mut context),
        Poll::Pending
    ));
    assert!(matches!(delete.as_mut().poll(&mut context), Poll::Pending));
    assert_eq!(dropped.load(Ordering::SeqCst), 0);

    drop(put);
    drop(compare_and_swap);
    drop(delete);
    assert_eq!(dropped.load(Ordering::SeqCst), 3);
}

#[test]
fn cancelling_a_query_finishes_an_accepted_recovery_write() {
    let store = RecoveryStore {
        inner: MemoryObjectStore::new(),
        fail_first_delete: Arc::new(AtomicBool::new(true)),
        delay_wal_delete: Arc::new(AtomicBool::new(false)),
        wal_delete_started: Arc::new(AtomicBool::new(false)),
    };
    let table = AsyncObjectTable::new(store.clone(), "users").unwrap();
    let snapshot = crate::xbf::XbfTable {
        generation: 0,
        fields: vec![crate::xbf::XbfField {
            name: "NAME".into(),
            ty: crate::xbf::XbfType::String,
            nullable: true,
            primary_key: false,
            unique: false,
        }],
        records: vec![crate::xbf::XbfRecord {
            deleted: false,
            values: vec![crate::xbf::XbfValue::String("Alice".into())],
        }],
    };

    assert!(matches!(
        block_on(table.commit(&snapshot)),
        Err(ObjectStoreError::Unavailable(_))
    ));
    store.delay_wal_delete.store(true, Ordering::SeqCst);

    let mut stream = Box::pin(table.query_stream(parse(b"{}").unwrap()).unwrap());
    let wake_count = Arc::new(CountWake::default());
    let waker = Waker::from(wake_count.clone());
    let mut context = Context::from_waker(&waker);
    assert!(matches!(
        stream.as_mut().poll_next(&mut context),
        Poll::Pending
    ));
    assert!(store.wal_delete_started.load(Ordering::SeqCst));

    stream.cancel();

    assert!(wake_count.0.load(Ordering::SeqCst) > 0);
    assert!(matches!(
        stream.as_mut().poll_next(&mut context),
        Poll::Ready(None)
    ));
    assert!(store.inner.get("users/wal/0.json").unwrap().is_none());
}

#[test]
fn cancelling_one_query_does_not_cancel_another_query() {
    let first_dropped = Arc::new(AtomicUsize::new(0));
    let first_table = AsyncObjectTable::new(
        PendingStore {
            dropped: first_dropped.clone(),
            calls: Arc::new(AtomicUsize::new(0)),
        },
        "first",
    )
    .unwrap();
    let second_dropped = Arc::new(AtomicUsize::new(0));
    let second_table = AsyncObjectTable::new(
        PendingStore {
            dropped: second_dropped.clone(),
            calls: Arc::new(AtomicUsize::new(0)),
        },
        "second",
    )
    .unwrap();
    let mut first = Box::pin(first_table.query_stream(parse(b"{}").unwrap()).unwrap());
    let mut second = Box::pin(second_table.query_stream(parse(b"{}").unwrap()).unwrap());
    let first_cancellation = first.cancellation_token();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(matches!(
        first.as_mut().poll_next(&mut context),
        Poll::Pending
    ));
    assert!(matches!(
        second.as_mut().poll_next(&mut context),
        Poll::Pending
    ));
    first_cancellation.cancel();
    assert!(matches!(
        first.as_mut().poll_next(&mut context),
        Poll::Ready(None)
    ));
    assert!(matches!(
        second.as_mut().poll_next(&mut context),
        Poll::Pending
    ));
    assert_eq!(first_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(second_dropped.load(Ordering::SeqCst), 0);
}

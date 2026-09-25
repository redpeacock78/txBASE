use super::{
    AsyncObjectStore, AsyncObjectStoreFuture, AsyncObjectTable, MemoryObjectStore, ObjectStore,
    ObjectStoreError, SyncObjectStoreAdapter,
};
use crate::query::{AsyncQueryStream, QueryRequest, parse};
use crate::xbf::{XbfField, XbfRecord, XbfTable, XbfType, XbfValue};
use serde_json::json;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = Box::pin(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

fn table(generation: u64, names: &[&str]) -> XbfTable {
    XbfTable {
        generation,
        fields: vec![XbfField {
            name: "NAME".into(),
            ty: XbfType::String,
            nullable: true,
            primary_key: false,
            unique: false,
        }],
        records: names
            .iter()
            .map(|name| XbfRecord {
                deleted: false,
                values: vec![XbfValue::String((*name).into())],
            })
            .collect(),
    }
}

struct PendingOnce<T> {
    value: Option<T>,
    pending: bool,
}

impl<T> Unpin for PendingOnce<T> {}

impl<T> std::future::Future for PendingOnce<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.pending {
            self.pending = false;
            context.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(
            self.value
                .take()
                .expect("pending future polled after completion"),
        )
    }
}

#[derive(Clone)]
struct PendingStore {
    inner: MemoryObjectStore,
}

impl AsyncObjectStore for PendingStore {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Option<Vec<u8>>, ObjectStoreError>> {
        Box::pin(PendingOnce {
            value: Some(self.inner.get(key)),
            pending: true,
        })
    }

    fn put_if_absent<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(PendingOnce {
            value: Some(self.inner.put_if_absent(key, bytes)),
            pending: true,
        })
    }

    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a [u8]>,
        replacement: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(PendingOnce {
            value: Some(self.inner.compare_and_swap(key, expected, replacement)),
            pending: true,
        })
    }

    fn delete<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(PendingOnce {
            value: Some(self.inner.delete(key)),
            pending: true,
        })
    }

    fn list<'a>(
        &'a self,
        prefix: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Vec<String>, ObjectStoreError>> {
        Box::pin(PendingOnce {
            value: Some(self.inner.list(prefix)),
            pending: true,
        })
    }
}

#[test]
fn async_object_query_stream_loads_one_snapshot_and_reuses_query_semantics() {
    let store = SyncObjectStoreAdapter::new(MemoryObjectStore::new());
    let object_table = AsyncObjectTable::new(store, "users").unwrap();
    let snapshot = table(0, &["Alice", "Bob"]);
    block_on(object_table.commit(&snapshot)).unwrap();

    let request =
        parse(br#"{"filter":{"NAME":{"$gte":"B"}},"projection":{"NAME":1},"limit":1}"#).unwrap();
    let mut stream = object_table.query_stream(request).unwrap();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    let first = AsyncQueryStream::poll_next(Pin::new(&mut stream), &mut context);
    assert!(matches!(first, Poll::Ready(Some(Ok(value))) if value == json!({"NAME": "Bob"})));
    assert!(matches!(
        AsyncQueryStream::poll_next(Pin::new(&mut stream), &mut context),
        Poll::Ready(None)
    ));
}

#[test]
fn async_object_query_stream_reads_a_retained_generation() {
    let store = SyncObjectStoreAdapter::new(MemoryObjectStore::new());
    let object_table = AsyncObjectTable::new(store, "users").unwrap();
    block_on(object_table.commit(&table(0, &["Alice"]))).unwrap();
    block_on(object_table.commit(&table(1, &["Bob"]))).unwrap();

    let mut stream = object_table
        .query_stream_at(0, QueryRequest::default())
        .unwrap();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(matches!(
        AsyncQueryStream::poll_next(Pin::new(&mut stream), &mut context),
        Poll::Ready(Some(Ok(value))) if value == json!({"NAME": "Alice"})
    ));
    assert!(matches!(
        AsyncQueryStream::poll_next(Pin::new(&mut stream), &mut context),
        Poll::Ready(None)
    ));
}

#[test]
fn async_object_query_stream_reports_an_unretained_generation_once() {
    let store = SyncObjectStoreAdapter::new(MemoryObjectStore::new());
    let object_table = AsyncObjectTable::new(store, "users").unwrap();
    block_on(object_table.commit(&table(0, &["Alice"]))).unwrap();
    block_on(object_table.commit(&table(1, &["Bob"]))).unwrap();
    block_on(object_table.retain_generations(1)).unwrap();

    let mut stream = object_table
        .query_stream_at(0, QueryRequest::default())
        .unwrap();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(matches!(
        AsyncQueryStream::poll_next(Pin::new(&mut stream), &mut context),
        Poll::Ready(Some(Err(crate::query::QueryError::Invalid(message))))
            if message.contains("no retained snapshot at generation 0")
    ));
    assert!(matches!(
        AsyncQueryStream::poll_next(Pin::new(&mut stream), &mut context),
        Poll::Ready(None)
    ));
}

#[test]
fn async_object_query_stream_reports_a_missing_snapshot_once() {
    let store = SyncObjectStoreAdapter::new(MemoryObjectStore::new());
    let object_table = AsyncObjectTable::new(store, "users").unwrap();
    let mut stream = object_table.query_stream(QueryRequest::default()).unwrap();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert!(matches!(
        AsyncQueryStream::poll_next(Pin::new(&mut stream), &mut context),
        Poll::Ready(Some(Err(crate::query::QueryError::Invalid(message))))
            if message.contains("no committed snapshot")
    ));
    assert!(matches!(
        AsyncQueryStream::poll_next(Pin::new(&mut stream), &mut context),
        Poll::Ready(None)
    ));
}

#[test]
fn async_object_query_stream_rejects_blocking_controls_before_loading() {
    let store = SyncObjectStoreAdapter::new(MemoryObjectStore::new());
    let object_table = AsyncObjectTable::new(store, "users").unwrap();
    let request = parse(br#"{"sort":{"NAME":1}}"#).unwrap();

    assert!(matches!(
        object_table.query_stream(request),
        Err(crate::query::QueryError::Invalid(message))
            if message.contains("filter, projection, skip, and limit")
    ));
}

#[test]
fn async_object_query_stream_preserves_pending_storage_future_boundary() {
    let inner = MemoryObjectStore::new();
    let writer =
        AsyncObjectTable::new(SyncObjectStoreAdapter::new(inner.clone()), "users").unwrap();
    block_on(writer.commit(&table(0, &["Alice"]))).unwrap();

    let object_table = AsyncObjectTable::new(PendingStore { inner }, "users").unwrap();
    let mut stream = object_table.query_stream(QueryRequest::default()).unwrap();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut saw_pending = false;
    let mut result = None;
    for _ in 0..8 {
        let poll = AsyncQueryStream::poll_next(Pin::new(&mut stream), &mut context);
        if matches!(poll, Poll::Pending) {
            saw_pending = true;
            continue;
        }
        result = Some(poll);
        break;
    }

    assert!(saw_pending);
    assert!(matches!(
        result,
        Some(Poll::Ready(Some(Ok(value)))) if value == json!({"NAME": "Alice"})
    ));
}

#[test]
fn async_object_query_stream_queries_non_dbf_xbf_values() {
    let store = SyncObjectStoreAdapter::new(MemoryObjectStore::new());
    let object_table = AsyncObjectTable::new(store, "users").unwrap();
    let fields = [
        ("BOOL", XbfType::Boolean),
        ("I32", XbfType::Signed32),
        ("I64", XbfType::Signed64),
        ("U64", XbfType::Unsigned64),
        ("F32", XbfType::Float32),
        ("F32_SPECIAL", XbfType::Float32),
        ("F64", XbfType::Float64),
        ("F64_SPECIAL", XbfType::Float64),
        ("STRING", XbfType::String),
        ("BYTES", XbfType::Bytes),
        ("DATE", XbfType::Date),
        ("DATE_OUTSIDE", XbfType::Date),
        ("TIMESTAMP", XbfType::Timestamp),
        ("TIMESTAMP_OUTSIDE", XbfType::Timestamp),
        ("UUID", XbfType::Uuid),
        ("JSON", XbfType::Json),
        ("I64_SHORT", XbfType::Signed64),
        ("NULL", XbfType::String),
    ]
    .into_iter()
    .map(|(name, ty)| XbfField {
        name: name.into(),
        ty,
        nullable: true,
        primary_key: false,
        unique: false,
    })
    .collect();
    let long_text = "x".repeat(300);
    let snapshot = XbfTable {
        generation: 0,
        fields,
        records: vec![XbfRecord {
            deleted: false,
            values: vec![
                XbfValue::Boolean(true),
                XbfValue::Signed32(-32),
                XbfValue::Signed64(-64),
                XbfValue::Unsigned64(u64::MAX),
                XbfValue::Float32(1.5),
                XbfValue::Float32(f32::from_bits(0x7fc0_0042)),
                XbfValue::Float64(-2.5),
                XbfValue::Float64(f64::NEG_INFINITY),
                XbfValue::String(long_text.clone()),
                XbfValue::Bytes(vec![0xab; 300]),
                XbfValue::Date(0),
                XbfValue::Date(i32::MAX),
                XbfValue::Timestamp(0),
                XbfValue::Timestamp(i64::MAX),
                XbfValue::Uuid(std::array::from_fn(|index| index as u8)),
                XbfValue::Json(json!({"nested": [true, 7]})),
                XbfValue::Signed32(64),
                XbfValue::Null,
            ],
        }],
    };
    assert!(snapshot.to_dbf().is_err());
    block_on(object_table.commit(&snapshot)).unwrap();

    let request = parse(br#"{"filter":{"UUID":{"$eq":"00010203-0405-0607-0809-0a0b0c0d0e0f"},"U64":{"$eq":18446744073709551615},"JSON":{"$eq":{"nested":[true,7]}}}}"#).unwrap();
    let mut stream = object_table.query_stream(request).unwrap();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let expected = json!({
        "BOOL": true,
        "I32": -32,
        "I64": -64,
        "U64": u64::MAX,
        "F32": 1.5,
        "F32_SPECIAL": {"$txbaseFloat32Bits": "7fc00042"},
        "F64": -2.5,
        "F64_SPECIAL": {"$txbaseFloat64Bits": "fff0000000000000"},
        "STRING": long_text,
        "BYTES": "ab".repeat(300),
        "DATE": "19700101",
        "DATE_OUTSIDE": i32::MAX,
        "TIMESTAMP": "8c3d250000000000",
        "TIMESTAMP_OUTSIDE": i64::MAX,
        "UUID": "00010203-0405-0607-0809-0a0b0c0d0e0f",
        "JSON": {"nested": [true, 7]},
        "I64_SHORT": 64,
        "NULL": null,
    });
    assert!(matches!(
        AsyncQueryStream::poll_next(Pin::new(&mut stream), &mut context),
        Poll::Ready(Some(Ok(value))) if value == expected
    ));
    assert!(matches!(
        AsyncQueryStream::poll_next(Pin::new(&mut stream), &mut context),
        Poll::Ready(None)
    ));
}

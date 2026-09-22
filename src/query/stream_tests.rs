use super::*;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{Receiver, Sender, channel};
#[cfg(not(target_arch = "wasm32"))]
use std::task::Wake;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

fn poll_ready<S>(stream: &mut S, context: &mut Context<'_>) -> Option<Result<Value, QueryError>>
where
    S: AsyncQueryStream<Item = Result<Value, QueryError>> + Unpin,
{
    match AsyncQueryStream::poll_next(Pin::new(stream), context) {
        Poll::Ready(item) => item,
        Poll::Pending => panic!("in-memory query stream unexpectedly returned Pending"),
    }
}

fn table_with_two_active_records() -> DbfTable {
    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    DbfTable::from_bytes(&bytes).unwrap()
}

#[cfg(not(target_arch = "wasm32"))]
struct SignalWaker(Sender<()>);

#[cfg(not(target_arch = "wasm32"))]
impl Wake for SignalWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let _ = self.0.send(());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn poll_threaded(
    stream: &mut ThreadedQueryStream,
    context: &mut Context<'_>,
    wakes: &Receiver<()>,
) -> Option<Result<Value, QueryError>> {
    loop {
        match AsyncQueryStream::poll_next(Pin::new(stream), context) {
            Poll::Ready(item) => return item,
            Poll::Pending => wakes
                .recv_timeout(Duration::from_secs(1))
                .expect("threaded stream must wake a pending poll"),
        }
    }
}

#[test]
fn streams_filtered_projected_records_with_bounded_controls() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "filter": {"AGE": {"$gte": 7}},
            "projection": {"NAME": 1},
            "skip": 1,
            "limit": 1
        }"#,
    )
    .unwrap();

    let records = stream_query(&table, &request)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(records, vec![serde_json::json!({"NAME": "Bob"})]);
}

#[test]
fn streaming_rejects_blocking_and_resume_controls() {
    let table = table_with_two_active_records();
    for request in [
        parse(br#"{"sort":{"AGE":1}}"#).unwrap(),
        parse(br#"{"aggregate":[{"$group":{"_id":null,"count":{"$count":{}}}}]}"#).unwrap(),
        parse(br#"{"page_size":1}"#).unwrap(),
    ] {
        assert!(stream_query(&table, &request).is_err());
    }
}

#[test]
fn snapshot_stream_is_independent_of_later_table_mutations() {
    let mut table = table_with_two_active_records();
    let request = parse(br#"{"projection":{"NAME":1}}"#).unwrap();
    let stream = stream_query_snapshot(&table, &request).unwrap();

    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "Changed"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();

    assert_eq!(
        stream.collect::<Result<Vec<_>, _>>().unwrap(),
        vec![
            serde_json::json!({"NAME": "Alice"}),
            serde_json::json!({"NAME": "Bob"})
        ]
    );
}

#[test]
fn bounded_snapshot_stream_keeps_a_fixed_snapshot() {
    let mut table = table_with_two_active_records();
    let request = parse(br#"{"projection":{"NAME":1}}"#).unwrap();
    let stream = stream_query_bounded(&table, &request, 1).unwrap();

    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "Changed"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();

    assert_eq!(
        stream.collect::<Result<Vec<_>, _>>().unwrap(),
        vec![
            serde_json::json!({"NAME": "Alice"}),
            serde_json::json!({"NAME": "Bob"})
        ]
    );
}

#[test]
fn bounded_stream_requires_positive_capacity() {
    let table = table_with_two_active_records();
    let request = parse(br#"{"projection":{"NAME":1}}"#).unwrap();
    assert!(stream_query_bounded(&table, &request, 0).is_err());
}

#[test]
fn in_memory_streams_implement_the_runtime_neutral_async_boundary() {
    let table = table_with_two_active_records();
    let request = parse(br#"{"projection":{"NAME":1}}"#).unwrap();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    let mut borrowed = stream_query(&table, &request).unwrap();
    assert_eq!(
        poll_ready(&mut borrowed, &mut context).unwrap().unwrap(),
        serde_json::json!({"NAME": "Alice"})
    );

    let mut snapshot = stream_query_snapshot(&table, &request).unwrap();
    assert_eq!(
        poll_ready(&mut snapshot, &mut context).unwrap().unwrap(),
        serde_json::json!({"NAME": "Alice"})
    );
    assert_eq!(
        poll_ready(&mut snapshot, &mut context).unwrap().unwrap(),
        serde_json::json!({"NAME": "Bob"})
    );
    assert!(poll_ready(&mut snapshot, &mut context).is_none());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn threaded_stream_polls_without_blocking_and_keeps_a_snapshot() {
    let mut table = table_with_two_active_records();
    let request = parse(br#"{"projection":{"NAME":1}}"#).unwrap();
    let mut stream = stream_query_threaded(&table, &request, 1).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "Changed"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();

    let (wake_sender, wake_receiver) = channel();
    let waker = Waker::from(Arc::new(SignalWaker(wake_sender)));
    let mut context = Context::from_waker(&waker);
    let mut records = Vec::new();
    while let Some(item) = poll_threaded(&mut stream, &mut context, &wake_receiver) {
        records.push(item.unwrap());
    }

    assert_eq!(
        records,
        vec![
            serde_json::json!({"NAME": "Alice"}),
            serde_json::json!({"NAME": "Bob"})
        ]
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn threaded_stream_requires_positive_capacity() {
    let table = table_with_two_active_records();
    let request = parse(br#"{"projection":{"NAME":1}}"#).unwrap();
    assert!(stream_query_threaded(&table, &request, 0).is_err());
}

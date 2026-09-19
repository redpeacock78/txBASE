use super::*;

fn table_with_two_active_records() -> DbfTable {
    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    DbfTable::from_bytes(&bytes).unwrap()
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

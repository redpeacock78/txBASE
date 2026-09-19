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
fn paginates_in_physical_record_order() {
    let table = table_with_two_active_records();
    let first = execute_query_page(&table, &parse(br#"{"page_size":1}"#).unwrap()).unwrap();
    assert_eq!(first.records.len(), 1);
    assert_eq!(first.next_cursor.as_deref(), Some("1"));

    let next =
        execute_query_page(&table, &parse(br#"{"page_size":1,"cursor":"1"}"#).unwrap()).unwrap();
    assert_eq!(next.records.len(), 1);
    assert_eq!(next.next_cursor, None);
}

#[test]
fn paginates_sorted_records_with_a_keyset_cursor() {
    let table = table_with_two_active_records();
    let request = parse(br#"{"page_size":1,"sort":{"AGE":1}}"#).unwrap();
    let first = execute_query_page(&table, &request).unwrap();
    assert_eq!(first.records[0]["NAME"], "Bob");
    let cursor = first.next_cursor.clone().expect("sorted page has a cursor");

    let next_request = parse(
        serde_json::json!({
            "page_size": 1,
            "sort": {"AGE": 1},
            "cursor": cursor,
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let next = execute_query_page(&table, &next_request).unwrap();
    assert_eq!(next.records[0]["NAME"], "Alice");
    assert_eq!(next.next_cursor, None);
}

#[test]
fn paginates_descending_sorted_records_with_a_keyset_cursor() {
    let table = table_with_two_active_records();
    let request = parse(br#"{"page_size":1,"sort":{"AGE":-1}}"#).unwrap();
    let first = execute_query_page(&table, &request).unwrap();
    assert_eq!(first.records[0]["NAME"], "Alice");
    let cursor = first.next_cursor.unwrap();
    let next_request = parse(
        serde_json::json!({
            "page_size": 1,
            "sort": {"AGE": -1},
            "cursor": cursor,
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let next = execute_query_page(&table, &next_request).unwrap();
    assert_eq!(next.records[0]["NAME"], "Bob");
    assert_eq!(next.next_cursor, None);
}

#[test]
fn rejects_a_sorted_cursor_for_a_different_sort() {
    let table = table_with_two_active_records();
    let request = parse(br#"{"page_size":1,"sort":{"AGE":1}}"#).unwrap();
    let cursor = execute_query_page(&table, &request)
        .unwrap()
        .next_cursor
        .unwrap();
    let error = parse(
        serde_json::json!({
            "page_size": 1,
            "sort": {"AGE": -1},
            "cursor": cursor,
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not match the sort definition")
    );
}

#[test]
fn limit_caps_physical_cursor_without_advertising_an_extra_page() {
    let table = table_with_two_active_records();
    let page =
        execute_query_page(&table, &parse(br#"{"page_size":1,"limit":1}"#).unwrap()).unwrap();

    assert_eq!(page.records.len(), 1);
    assert_eq!(page.next_cursor, None);

    let request = parse(br#"{"page_size":1,"filter":{"ID":1}}"#).unwrap();
    assert_eq!(
        explain_query_at("missing-index-source.dbf", &request).unwrap(),
        QueryPlan::TableScan
    );
}

#[test]
fn rejects_ambiguous_cursor_boundaries() {
    for body in [
        br#"{"cursor":"1"}"#.as_slice(),
        br#"{"page_size":1,"skip":1}"#.as_slice(),
        br#"{"page_size":0}"#.as_slice(),
        br#"{"page_size":1,"cursor":"first"}"#.as_slice(),
    ] {
        assert!(parse(body).is_err(), "expected rejection for {body:?}");
    }
}

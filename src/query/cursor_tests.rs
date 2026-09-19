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
fn rejects_ambiguous_cursor_boundaries() {
    for body in [
        br#"{"cursor":"1"}"#.as_slice(),
        br#"{"page_size":1,"sort":{"AGE":1}}"#.as_slice(),
        br#"{"page_size":1,"skip":1}"#.as_slice(),
        br#"{"page_size":0}"#.as_slice(),
        br#"{"page_size":1,"cursor":"first"}"#.as_slice(),
    ] {
        assert!(parse(body).is_err(), "expected rejection for {body:?}");
    }
}

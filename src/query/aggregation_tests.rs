use super::*;
use serde_json::json;

fn table_with_two_active_records() -> DbfTable {
    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    DbfTable::from_bytes(&bytes).unwrap()
}

#[test]
fn groups_filtered_records_with_count_and_integer_sum() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "filter": {"AGE": {"$gte": 0}},
            "aggregate": [{"$group": {
                "_id": null,
                "count": {"$count": {}},
                "total_age": {"$sum": "$AGE"}
            }}]
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![json!({"_id": null, "count": 2, "total_age": 36})]
    );
}

#[test]
fn groups_comparable_extremes_and_returns_null_for_missing_values() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "aggregate": [{"$group": {
                "_id": null,
                "youngest": {"$min": "$AGE"},
                "oldest": {"$max": "$AGE"},
                "missing_min": {"$min": "$MISSING"}
            }}]
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![json!({"_id": null, "youngest": 7, "oldest": 29, "missing_min": null})]
    );
}

#[test]
fn missing_group_fields_share_the_null_group() {
    let table = table_with_two_active_records();
    let request =
        parse(br#"{"aggregate":[{"$group":{"_id":"$MISSING","count":{"$count":{}}}}]}"#).unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![json!({"_id": null, "count": 2})]
    );
}

#[test]
fn applies_match_stages_before_grouping() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "aggregate": [
                {"$match": {"AGE": {"$gte": 20}}},
                {"$match": {"ACTIVE": true}},
                {"$group": {
                    "_id": null,
                    "count": {"$count": {}},
                    "total_age": {"$sum": "$AGE"}
                }}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![json!({"_id": null, "count": 1, "total_age": 29})]
    );
}

#[test]
fn sorts_group_output_after_grouping() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "aggregate": [
                {"$group": {
                    "_id": "$AGE",
                    "count": {"$count": {}}
                }},
                {"$sort": {"_id": -1}}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![
            json!({"_id": 29, "count": 1}),
            json!({"_id": 7, "count": 1})
        ]
    );
}

#[test]
fn rejects_unsupported_aggregation_combinations() {
    for body in [
        br#"{"aggregate":[]}"#.as_slice(),
        br#"{"sort":{"AGE":1},"aggregate":[{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$count":"total"}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null,"total":{"$sum":"AGE"}}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$match":{}}]}"#.as_slice(),
        br#"{"aggregate":[{"$sort":{"_id":1}},{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$sort":{}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$sort":{"_id":2}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$sort":{"_id":1}},{"$sort":{"_id":1}}]}"#
            .as_slice(),
    ] {
        assert!(parse(body).is_err(), "expected rejection for {body:?}");
    }
}

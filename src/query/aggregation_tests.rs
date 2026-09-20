use super::*;
use crate::dbf::DbfRecord;
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
fn sums_fractional_and_integer_numbers() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"AMOUNT": 1.5}).as_object().unwrap().clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"AMOUNT": 2}).as_object().unwrap().clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "total": {"$sum": "$AMOUNT"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        super::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({"_id": null, "total": 3.5})]
    );
}

#[test]
fn sums_numeric_literals_for_each_input_record() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"VALUE": 10}).as_object().unwrap().clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"VALUE": 20}).as_object().unwrap().clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "fixed": {"$sum": 2},
                "fractional": {"$sum": 0.5}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        super::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({"_id": null, "fixed": 4, "fractional": 1.0})]
    );
}

#[test]
fn counts_filtered_records_with_a_count_stage() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "aggregate": [
                {"$match": {"AGE": {"$gte": 20}}},
                {"$count": "total"}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![json!({"total": 1})]
    );

    let empty = parse(
        br#"{
            "aggregate": [
                {"$match": {"AGE": {"$gt": 100}}},
                {"$count": "total"}
            ]
        }"#,
    )
    .unwrap();
    assert_eq!(
        execute_query(&table, &empty).unwrap(),
        vec![json!({"total": 0})]
    );
}

#[test]
fn returns_distinct_values_after_matching() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "aggregate": [
                {"$match": {"AGE": {"$gte": 0}}},
                {"$distinct": "$ACTIVE"}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![json!(false), json!(true)]
    );
}

#[test]
fn groups_numeric_average_and_returns_null_for_missing_values() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "aggregate": [{"$group": {
                "_id": null,
                "average_age": {"$avg": "$AGE"},
                "missing_average": {"$avg": "$MISSING"}
            }}]
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![json!({"_id": null, "average_age": 18.0, "missing_average": null})]
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
fn groups_first_and_last_values_in_physical_order() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"VALUE": "first"}).as_object().unwrap().clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"VALUE": "last"}).as_object().unwrap().clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "first_value": {"$first": "$VALUE"},
                "last_value": {"$last": "$VALUE"},
                "missing_value": {"$first": "$MISSING"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        super::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({
            "_id": null,
            "first_value": "first",
            "last_value": "last",
            "missing_value": null
        })]
    );
}

#[test]
fn groups_pushed_values_and_deduplicates_set_values() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"VALUE": "first"}).as_object().unwrap().clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"VALUE": "first"}).as_object().unwrap().clone(),
    };
    let third = DbfRecord {
        number: 3,
        deleted: false,
        values: json!({}).as_object().unwrap().clone(),
    };
    let records = [&first, &second, &third];
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "all_values": {"$push": "$VALUE"},
                "unique_values": {"$addToSet": "$VALUE"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        super::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({
            "_id": null,
            "all_values": ["first", "first", null],
            "unique_values": ["first", null]
        })]
    );
}

#[test]
fn bounds_materialized_group_values() {
    let records = (0..=super::aggregation::MAX_COLLECTED_VALUES)
        .map(|number| DbfRecord {
            number,
            deleted: false,
            values: json!({"VALUE": number}).as_object().unwrap().clone(),
        })
        .collect::<Vec<_>>();
    let references = records.iter().collect::<Vec<_>>();
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "values": {"$push": "$VALUE"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    let error = super::aggregation::execute(&references, &stages).unwrap_err();
    assert!(error.to_string().contains("collected value count"));
}

#[test]
fn aggregation_restores_physical_order_after_index_candidates() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "aggregate": [{"$group": {
                "_id": null,
                "first_value": {"$first": "$NAME"},
                "last_value": {"$last": "$NAME"}
            }}]
        }"#,
    )
    .unwrap();

    let page = super::execute_query_with_records(&table, &request, Some(vec![2, 1]), 0).unwrap();

    assert_eq!(
        page.records,
        vec![json!({"_id": null, "first_value": "Alice", "last_value": "Bob"})]
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
fn filters_group_output_before_projection_and_sorting() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "aggregate": [
                {"$group": {
                    "_id": "$AGE",
                    "count": {"$count": {}}
                }},
                {"$match": {"_id": {"$gte": 20}}},
                {"$project": {"_id": 1}},
                {"$sort": {"_id": -1}}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![json!({"_id": 29})]
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
fn limits_sorted_group_output() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "aggregate": [
                {"$group": {
                    "_id": "$AGE",
                    "count": {"$count": {}}
                }},
                {"$sort": {"_id": -1}},
                {"$limit": 1}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![json!({"_id": 29, "count": 1})]
    );
}

#[test]
fn projects_group_output_before_sorting() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "aggregate": [
                {"$group": {
                    "_id": "$AGE",
                    "count": {"$count": {}}
                }},
                {"$project": {"_id": 1}},
                {"$sort": {"_id": -1}}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![json!({"_id": 29}), json!({"_id": 7})]
    );
}

#[test]
fn rejects_unsupported_aggregation_combinations() {
    for body in [
        br#"{"aggregate":[]}"#.as_slice(),
        br#"{"sort":{"AGE":1},"aggregate":[{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$count":{}}]}"#.as_slice(),
        br#"{"aggregate":[{"$count":""}]}"#.as_slice(),
        br#"{"aggregate":[{"$count":"total.value"}]}"#.as_slice(),
        br#"{"aggregate":[{"$count":"total"},{"$match":{}}]}"#.as_slice(),
        br#"{"aggregate":[{"$count":"total"},{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$distinct":"ACTIVE"}]}"#.as_slice(),
        br#"{"aggregate":[{"$distinct":"$ACTIVE"},{"$limit":1}]}"#.as_slice(),
        br#"{"aggregate":[{"$distinct":"$ACTIVE"},{"$distinct":"$AGE"}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null,"total":{"$sum":"AGE"}}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null,"total":{"$sum":true}}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null,"average":{"$avg":"AGE"}}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$project":{"_id":1}},{"$match":{}}]}"#
            .as_slice(),
        br#"{"aggregate":[{"$sort":{"_id":1}},{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$sort":{}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$sort":{"_id":2}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$sort":{"_id":1}},{"$sort":{"_id":1}}]}"#
            .as_slice(),
        br#"{"aggregate":[{"$limit":1},{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$limit":-1}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$limit":1},{"$limit":1}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$limit":1},{"$sort":{"_id":1}}]}"#.as_slice(),
        br#"{"aggregate":[{"$project":{"_id":1}},{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$project":{"_id":1,"count":0}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$sort":{"_id":1}},{"$project":{"_id":1}}]}"#
            .as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$project":{"_id":1}},{"$project":{"_id":1}}]}"#
            .as_slice(),
    ] {
        assert!(parse(body).is_err(), "expected rejection for {body:?}");
    }
}

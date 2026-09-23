use super::table_with_two_active_records;
use crate::dbf::DbfRecord;
use serde_json::json;

#[test]
fn groups_filtered_records_with_count_and_integer_sum() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
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
        crate::query::execute_query(&table, &request).unwrap(),
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
        crate::query::aggregation::execute(&records, &stages).unwrap(),
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
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({"_id": null, "fixed": 4, "fractional": 1.0})]
    );
}

#[test]
fn evaluates_bounded_numeric_accumulator_expressions() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"AGE": 10, "DELTA": -2}).as_object().unwrap().clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"AGE": 20, "DELTA": 4}).as_object().unwrap().clone(),
    };
    let third = DbfRecord {
        number: 3,
        deleted: false,
        values: json!({"AGE": "not-a-number", "DELTA": null})
            .as_object()
            .unwrap()
            .clone(),
    };
    let records = [&first, &second, &third];
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "total": {"$sum": {"$add": ["$AGE", 1]}},
                "average_delta": {"$avg": {"$abs": "$DELTA"}},
                "doubled": {"$sum": {"$multiply": ["$AGE", 2]}}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({"_id": null, "total": 32, "average_delta": 3.0, "doubled": 60})]
    );
}

#[test]
fn counts_filtered_records_with_a_count_stage() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
        br#"{
            "aggregate": [
                {"$match": {"AGE": {"$gte": 20}}},
                {"$count": "total"}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        crate::query::execute_query(&table, &request).unwrap(),
        vec![json!({"total": 1})]
    );

    let empty = crate::query::parse(
        br#"{
            "aggregate": [
                {"$match": {"AGE": {"$gt": 100}}},
                {"$count": "total"}
            ]
        }"#,
    )
    .unwrap();
    assert_eq!(
        crate::query::execute_query(&table, &empty).unwrap(),
        vec![json!({"total": 0})]
    );
}

#[test]
fn returns_distinct_values_after_matching() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
        br#"{
            "aggregate": [
                {"$match": {"AGE": {"$gte": 0}}},
                {"$distinct": "$ACTIVE"}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        crate::query::execute_query(&table, &request).unwrap(),
        vec![json!(false), json!(true)]
    );
}

#[test]
fn groups_numeric_average_and_returns_null_for_missing_values() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
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
        crate::query::execute_query(&table, &request).unwrap(),
        vec![json!({"_id": null, "average_age": 18.0, "missing_average": null})]
    );
}

#[test]
fn groups_comparable_extremes_and_returns_null_for_missing_values() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
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
        crate::query::execute_query(&table, &request).unwrap(),
        vec![json!({"_id": null, "youngest": 7, "oldest": 29, "missing_min": null})]
    );
}

#[test]
fn groups_by_a_dotted_field_reference() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"PROFILE": {"COUNTRY": "JP"}})
            .as_object()
            .unwrap()
            .clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"PROFILE": {"COUNTRY": "US"}})
            .as_object()
            .unwrap()
            .clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({
            "$group": {
                "_id": "$PROFILE.COUNTRY",
                "count": {"$count": {}}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![
            json!({"_id": "JP", "count": 1}),
            json!({"_id": "US", "count": 1})
        ]
    );
}

#[test]
fn rejects_incomparable_extreme_values() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"VALUE": 1}).as_object().unwrap().clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"VALUE": "one"}).as_object().unwrap().clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "minimum": {"$min": "$VALUE"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    let error = crate::query::aggregation::execute(&records, &stages).unwrap_err();
    assert!(error.to_string().contains("incomparable values"));
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
        crate::query::aggregation::execute(&records, &stages).unwrap(),
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
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({
            "_id": null,
            "all_values": ["first", "first", null],
            "unique_values": ["first", null]
        })]
    );
}

#[test]
fn bounds_materialized_group_values() {
    let records = (0..=crate::query::aggregation::MAX_COLLECTED_VALUES)
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

    let error = crate::query::aggregation::execute(&references, &stages).unwrap_err();
    assert!(error.to_string().contains("collected value count"));
}

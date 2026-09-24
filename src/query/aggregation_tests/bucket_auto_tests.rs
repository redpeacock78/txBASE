use crate::dbf::DbfRecord;
use serde_json::{Value, json};

fn record(number: u32, value: Value) -> DbfRecord {
    DbfRecord {
        number: number as usize,
        deleted: false,
        values: value.as_object().cloned().unwrap_or_default(),
    }
}

fn execute(
    records: &[DbfRecord],
    stages: &[Value],
) -> Result<Vec<Value>, crate::query::QueryError> {
    let stages = stages
        .iter()
        .map(|stage| stage.as_object().expect("stage object").clone())
        .collect::<Vec<_>>();
    crate::query::aggregation::execute(&records.iter().collect::<Vec<_>>(), &stages)
}

#[test]
fn bucket_auto_uses_sorted_numeric_ranges_and_default_count() {
    let records = [
        record(1, json!({"AGE": 7})),
        record(2, json!({"AGE": 18})),
        record(3, json!({"AGE": 29})),
        record(4, json!({"AGE": 45})),
    ];

    assert_eq!(
        execute(
            &records,
            &[json!({"$bucketAuto": {"groupBy": "$AGE", "buckets": 2}})],
        )
        .unwrap(),
        vec![
            json!({"_id": {"min": 7, "max": 29}, "count": 2}),
            json!({"_id": {"min": 29, "max": 45}, "count": 2}),
        ]
    );
}

#[test]
fn bucket_auto_preserves_input_order_for_accumulators_and_output_skip() {
    let records = [
        record(1, json!({"AGE": 7, "NAME": "Alice"})),
        record(2, json!({"AGE": 18, "NAME": "Bob"})),
        record(3, json!({"AGE": 29, "NAME": "Carol"})),
        record(4, json!({"AGE": 45, "NAME": "Dave"})),
    ];

    assert_eq!(
        execute(
            &records,
            &[
                json!({"$bucketAuto": {
                    "groupBy": "$AGE",
                    "buckets": 2,
                    "output": {
                        "names": {"$push": "$NAME"},
                        "count": {"$count": {}}
                    }
                }}),
                json!({"$skip": 1}),
            ],
        )
        .unwrap(),
        vec![json!({
            "_id": {"min": 29, "max": 45},
            "names": ["Carol", "Dave"],
            "count": 2
        })]
    );
}

#[test]
fn bucket_auto_emits_fewer_buckets_for_duplicate_values() {
    let records = [
        record(1, json!({"VALUE": 1})),
        record(2, json!({"VALUE": 1})),
        record(3, json!({"VALUE": 2})),
    ];

    assert_eq!(
        execute(
            &records,
            &[json!({"$bucketAuto": {"groupBy": "$VALUE", "buckets": 5}})],
        )
        .unwrap(),
        vec![
            json!({"_id": {"min": 1, "max": 2}, "count": 2}),
            json!({"_id": {"min": 2, "max": 2}, "count": 1}),
        ]
    );
}

#[test]
fn bucket_auto_rejects_unsupported_or_non_numeric_input() {
    let records = [record(1, json!({"VALUE": "one"}))];
    for stage in [
        json!({"$bucketAuto": {"groupBy": "$VALUE", "buckets": 0}}),
        json!({"$bucketAuto": {"groupBy": "$VALUE", "buckets": 1, "granularity": "R10"}}),
        json!({"$bucketAuto": {"groupBy": "$VALUE", "buckets": 1, "output": {}}}),
    ] {
        assert!(execute(&records, &[stage]).is_err());
    }

    let stages = [json!({
        "$bucketAuto": {"groupBy": "$VALUE", "buckets": 1}
    })];
    assert!(execute(&records, &stages).is_err());
}

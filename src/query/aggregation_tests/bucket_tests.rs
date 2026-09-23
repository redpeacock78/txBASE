use crate::dbf::DbfRecord;
use serde_json::{Value, json};

fn record(number: u32, value: Value) -> DbfRecord {
    DbfRecord {
        number: number as usize,
        deleted: false,
        values: value.as_object().cloned().unwrap_or_default(),
    }
}

#[test]
fn buckets_numeric_ranges_and_default_values() {
    let records = [
        record(1, json!({"AGE": 7, "SCORE": 10})),
        record(2, json!({"AGE": 18, "SCORE": 20})),
        record(3, json!({"AGE": 29, "SCORE": 30})),
        record(4, json!({"AGE": 45, "SCORE": 40})),
        record(5, json!({"AGE": "unknown", "SCORE": 50})),
        record(6, json!({"SCORE": 60})),
    ];
    let stages = vec![
        json!({
            "$bucket": {
                "groupBy": "$AGE",
                "boundaries": [0, 20, 40],
                "default": "other",
                "output": {
                    "count": {"$count": {}},
                    "average_score": {"$avg": "$SCORE"}
                }
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records.iter().collect::<Vec<_>>(), &stages).unwrap(),
        vec![
            json!({"_id": 0, "count": 2, "average_score": 15.0}),
            json!({"_id": 20, "count": 1, "average_score": 30.0}),
            json!({"_id": "other", "count": 3, "average_score": 50.0}),
        ]
    );
}

#[test]
fn bucket_defaults_to_count_and_supports_group_output_stages() {
    let records = [
        record(1, json!({"AGE": 7})),
        record(2, json!({"AGE": 29})),
        record(3, json!({"AGE": 31})),
    ];
    let stages = vec![
        json!({
            "$bucket": {
                "groupBy": "$AGE",
                "boundaries": [0, 20, 40]
            }
        })
        .as_object()
        .unwrap()
        .clone(),
        json!({"$match": {"count": {"$gte": 2}}})
            .as_object()
            .unwrap()
            .clone(),
        json!({"$sort": {"_id": -1}}).as_object().unwrap().clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records.iter().collect::<Vec<_>>(), &stages).unwrap(),
        vec![json!({"_id": 20, "count": 2})]
    );
}

#[test]
fn rejects_invalid_bucket_boundaries_and_unmatched_values() {
    let records = [record(1, json!({"AGE": 7}))];
    let invalid_stages = [
        json!({
            "$bucket": {"groupBy": "$AGE", "boundaries": [20, 20, 40]}
        }),
        json!({
            "$bucket": {"groupBy": "$AGE", "boundaries": [0, "20"]}
        }),
        json!({
            "$bucket": {"groupBy": "$AGE", "boundaries": [0, 20], "output": {}}
        }),
    ];
    for stage in invalid_stages {
        let stages = vec![stage.as_object().unwrap().clone()];
        assert!(
            crate::query::aggregation::execute(&records.iter().collect::<Vec<_>>(), &stages)
                .is_err()
        );
    }

    let stages = vec![
        json!({"$bucket": {"groupBy": "$AGE", "boundaries": [0, 20]}})
            .as_object()
            .unwrap()
            .clone(),
    ];
    let outside = [record(1, json!({"AGE": 20}))];
    assert!(
        crate::query::aggregation::execute(&outside.iter().collect::<Vec<_>>(), &stages).is_err()
    );
}

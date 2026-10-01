use crate::dbf::DbfRecord;
use serde_json::{Map, Value, json};

fn record(number: usize, values: Value) -> DbfRecord {
    DbfRecord {
        number,
        deleted: false,
        values: values.as_object().cloned().unwrap_or_default(),
    }
}

fn execute(records: &[DbfRecord], stage: Value) -> Result<Vec<Value>, crate::query::QueryError> {
    let stages = vec![stage.as_object().expect("stage object").clone()];
    crate::query::aggregation::execute(&records.iter().collect::<Vec<_>>(), &stages)
}

#[test]
fn merges_objects_for_group_bucket_and_bucket_auto_accumulators() {
    let records = [
        record(1, json!({"KEY": 2, "OBJECT": {"a": 1, "b": 2}})),
        record(2, json!({"KEY": 1, "OBJECT": {"a": 3, "c": null}})),
        record(3, json!({"KEY": 4, "OBJECT": null})),
        record(4, json!({"KEY": 3})),
    ];
    let expected = json!({"a": 3, "b": 2, "c": null});
    let stages = [
        json!({
            "$group": {
                "_id": null,
                "merged": {"$mergeObjects": "$OBJECT"}
            }
        }),
        json!({
            "$bucket": {
                "groupBy": "$KEY",
                "boundaries": [0, 5],
                "output": {"merged": {"$mergeObjects": "$OBJECT"}}
            }
        }),
        json!({
            "$bucketAuto": {
                "groupBy": "$KEY",
                "buckets": 1,
                "output": {"merged": {"$mergeObjects": "$OBJECT"}}
            }
        }),
    ];

    for stage in stages {
        let output = execute(&records, stage).unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0]["merged"], expected);
    }
}

#[test]
fn merge_objects_ignores_only_null_or_missing_inputs_and_accepts_literal_documents() {
    let records = [record(1, json!({"OBJECT": null})), record(2, json!({}))];

    assert_eq!(
        execute(
            &records,
            json!({
                "$group": {
                    "_id": null,
                    "merged": {"$mergeObjects": "$OBJECT"},
                    "literal": {
                        "$mergeObjects": {"$literal": {"source": "literal"}}
                    }
                }
            }),
        )
        .unwrap(),
        vec![json!({
            "_id": null,
            "merged": {},
            "literal": {"source": "literal"}
        })]
    );
}

#[test]
fn merge_objects_rejects_non_document_values_and_bounds_retained_fields() {
    let stage = json!({"$group": {"_id": null, "merged": {"$mergeObjects": "$OBJECT"}}});
    let invalid = [record(1, json!({"OBJECT": 1}))];
    let error = execute(&invalid, stage.clone()).unwrap_err();
    assert!(error.to_string().contains("$mergeObjects"));

    let mut object = serde_json::Map::new();
    for field in 0..=crate::query::aggregation::MAX_COLLECTED_VALUES {
        object.insert(field.to_string(), json!(field));
    }
    let oversized = [record(2, json!({"OBJECT": Value::Object(object)}))];
    let error = execute(&oversized, stage).unwrap_err();
    assert!(error.to_string().contains("collected value count"));
}

#[test]
fn merge_objects_shares_retained_field_budget_across_groups() {
    let records = (0..=crate::query::aggregation::MAX_COLLECTED_VALUES)
        .map(|field| {
            let mut object = Map::new();
            object.insert(field.to_string(), json!(field));
            record(field + 1, json!({"KEY": field % 2, "OBJECT": object}))
        })
        .collect::<Vec<_>>();
    let stage = json!({
        "$group": {
            "_id": "$KEY",
            "merged": {"$mergeObjects": "$OBJECT"}
        }
    });

    let error = execute(&records, stage).unwrap_err();
    assert!(error.to_string().contains("collected value count"));
}

#[test]
fn merge_objects_overwrite_does_not_consume_another_retained_value() {
    let mut initial = Map::new();
    for field in 0..crate::query::aggregation::MAX_COLLECTED_VALUES {
        initial.insert(field.to_string(), json!(field));
    }
    let records = [
        record(1, json!({"KEY": 1, "OBJECT": initial})),
        record(2, json!({"KEY": 1, "OBJECT": {"0": "replacement"}})),
    ];
    let stage = json!({
        "$group": {
            "_id": "$KEY",
            "merged": {"$mergeObjects": "$OBJECT"}
        }
    });

    let output = execute(&records, stage).unwrap();
    assert_eq!(output[0]["merged"]["0"], "replacement");
}

#[test]
fn merge_objects_shares_budget_with_value_accumulators() {
    let mut object = Map::new();
    for field in 0..crate::query::aggregation::MAX_COLLECTED_VALUES {
        object.insert(field.to_string(), json!(field));
    }
    let records = [record(1, json!({"OBJECT": object, "VALUE": 1}))];
    let stage = json!({
        "$group": {
            "_id": null,
            "merged": {"$mergeObjects": "$OBJECT"},
            "values": {"$push": "$VALUE"}
        }
    });

    let error = execute(&records, stage).unwrap_err();
    assert!(error.to_string().contains("collected value count"));
}

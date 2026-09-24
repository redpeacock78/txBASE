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
fn sort_by_count_groups_missing_values_and_sorts_descending() {
    let records = [
        record(1, json!({"COUNTRY": "JP"})),
        record(2, json!({"COUNTRY": "JP"})),
        record(3, json!({"COUNTRY": "JP"})),
        record(4, json!({"COUNTRY": "US"})),
        record(5, json!({"COUNTRY": "US"})),
        record(6, json!({})),
    ];
    let stages = vec![
        json!({"$sortByCount": "$COUNTRY"})
            .as_object()
            .unwrap()
            .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records.iter().collect::<Vec<_>>(), &stages).unwrap(),
        vec![
            json!({"_id": "JP", "count": 3}),
            json!({"_id": "US", "count": 2}),
            json!({"_id": null, "count": 1}),
        ]
    );
}

#[test]
fn sort_by_count_applies_input_and_group_output_stages() {
    let records = [
        record(1, json!({"ACTIVE": true, "COUNTRY": "JP"})),
        record(2, json!({"ACTIVE": true, "COUNTRY": "JP"})),
        record(3, json!({"ACTIVE": true, "COUNTRY": "US"})),
        record(4, json!({"ACTIVE": false, "COUNTRY": "US"})),
    ];
    let stages = vec![
        json!({"$match": {"ACTIVE": true}})
            .as_object()
            .unwrap()
            .clone(),
        json!({"$sortByCount": "$COUNTRY"})
            .as_object()
            .unwrap()
            .clone(),
        json!({"$match": {"count": {"$gte": 2}}})
            .as_object()
            .unwrap()
            .clone(),
        json!({"$project": {"_id": 1, "count": 1}})
            .as_object()
            .unwrap()
            .clone(),
        json!({"$limit": 1}).as_object().unwrap().clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records.iter().collect::<Vec<_>>(), &stages).unwrap(),
        vec![json!({"_id": "JP", "count": 2})]
    );
}

#[test]
fn rejects_unsupported_sort_by_count_forms_and_combinations() {
    let records = [record(1, json!({"COUNTRY": "JP"}))];
    for stages in [
        vec![
            json!({"$sortByCount": "COUNTRY"})
                .as_object()
                .unwrap()
                .clone(),
        ],
        vec![
            json!({"$sortByCount": "$COUNTRY"})
                .as_object()
                .unwrap()
                .clone(),
            json!({"$sortByCount": "$COUNTRY"})
                .as_object()
                .unwrap()
                .clone(),
        ],
        vec![
            json!({"$sortByCount": "$COUNTRY"})
                .as_object()
                .unwrap()
                .clone(),
            json!({"$group": {"_id": null}})
                .as_object()
                .unwrap()
                .clone(),
        ],
    ] {
        assert!(
            crate::query::aggregation::execute(&records.iter().collect::<Vec<_>>(), &stages)
                .is_err(),
            "expected rejection for {stages:?}"
        );
    }
}

use crate::dbf::DbfRecord;
use serde_json::json;

#[test]
fn array_and_let_expressions_are_shared_by_set_and_group_stages() {
    let records = [DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"CATEGORY": "cold", "ITEMS": [1, 2, 3]})
            .as_object()
            .unwrap()
            .clone(),
    }];
    let references = records.iter().collect::<Vec<_>>();
    let stages = vec![
        json!({"$set": {
            "DOUBLED": {"$map": {
                "input": "$ITEMS",
                "as": "item",
                "in": {"$multiply": ["$$item", 2]}
            }},
            "TOTAL": {"$reduce": {
                "input": "$ITEMS",
                "initialValue": 0,
                "in": {"$add": ["$$value", "$$this"]}
            }}
        }})
        .as_object()
        .unwrap()
        .clone(),
        json!({"$group": {
            "_id": {"$let": {
                "vars": {"category": "$CATEGORY"},
                "in": "$$category"
            }},
            "doubled": {"$first": "$DOUBLED"},
            "total": {"$first": "$TOTAL"}
        }})
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&references, &stages).unwrap(),
        vec![json!({"_id": "cold", "doubled": [2, 4, 6], "total": 6})]
    );
}

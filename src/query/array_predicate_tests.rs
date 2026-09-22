use super::matches_filter;
use serde_json::json;

#[test]
fn matches_all_and_exact_array_size() {
    let values = json!({
        "TAGS": ["db", "rust", "wasm"],
        "EMPTY": [],
        "SCALAR": "db"
    });
    let values = values.as_object().unwrap();

    assert!(
        matches_filter(
            values,
            json!({"TAGS": {"$all": ["rust", "wasm"]}})
                .as_object()
                .unwrap()
        )
        .unwrap()
    );
    assert!(
        !matches_filter(
            values,
            json!({"TAGS": {"$all": ["rust", "sql"]}})
                .as_object()
                .unwrap()
        )
        .unwrap()
    );
    assert!(matches_filter(values, json!({"TAGS": {"$size": 3}}).as_object().unwrap()).unwrap());
    assert!(!matches_filter(values, json!({"SCALAR": {"$size": 1}}).as_object().unwrap()).unwrap());
    assert!(!matches_filter(values, json!({"EMPTY": {"$all": []}}).as_object().unwrap()).unwrap());
}

#[test]
fn elem_match_binds_all_conditions_to_one_array_element() {
    let values = json!({
        "SCORES": [82, 85, 88],
        "RESULTS": [
            {"product": "abc", "score": 7},
            {"product": "xyz", "score": 8}
        ]
    });
    let values = values.as_object().unwrap();

    assert!(
        matches_filter(
            values,
            json!({"SCORES": {"$elemMatch": {"$gte": 80, "$lt": 85}}})
                .as_object()
                .unwrap()
        )
        .unwrap()
    );
    assert!(
        !matches_filter(
            values,
            json!({"SCORES": {"$elemMatch": {"$gt": 82, "$lt": 85}}})
                .as_object()
                .unwrap()
        )
        .unwrap()
    );
    assert!(
        matches_filter(
            values,
            json!({"RESULTS": {"$elemMatch": {"product": "xyz", "score": {"$gte": 8}}}})
                .as_object()
                .unwrap()
        )
        .unwrap()
    );
    assert!(
        matches_filter(
            values,
            json!({
                "RESULTS": {
                    "$all": [
                        {"$elemMatch": {"product": "abc"}},
                        {"$elemMatch": {"score": {"$gte": 8}}}
                    ]
                }
            })
            .as_object()
            .unwrap()
        )
        .unwrap()
    );
}

#[test]
fn rejects_malformed_array_predicates() {
    assert!(crate::query::parse(br#"{"filter":{"TAGS":{"$all":"rust"}}}"#).is_err());
    assert!(crate::query::parse(br#"{"filter":{"TAGS":{"$elemMatch":true}}}"#).is_err());
    assert!(crate::query::parse(br#"{"filter":{"TAGS":{"$size":-1}}}"#).is_err());
    assert!(crate::query::parse(br#"{"filter":{"TAGS":{"$size":1.5}}}"#).is_err());
    assert!(crate::query::parse(br#"{"filter":{"TAGS":{"$all":[{"$gt":"rust"}]}}}"#).is_err());
}

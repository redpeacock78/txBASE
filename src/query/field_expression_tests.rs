use super::*;
use serde_json::json;

#[test]
fn compares_two_fields_with_expr() {
    let values = json!({
        "LEFT": 4,
        "RIGHT": 2,
        "NESTED": {"VALUE": 4}
    });
    let filter = json!({
        "$expr": {"$gt": ["$NESTED.VALUE", "$RIGHT"]}
    });

    assert!(matches_filter(values.as_object().unwrap(), filter.as_object().unwrap()).unwrap());
}

#[test]
fn missing_expr_operand_does_not_match() {
    let values = json!({"LEFT": 4});
    let filter = json!({"$expr": {"$eq": ["$LEFT", "$MISSING"]}});

    assert!(!matches_filter(values.as_object().unwrap(), filter.as_object().unwrap()).unwrap());
}

#[test]
fn rejects_unsupported_or_malformed_expr() {
    assert!(parse(br#"{"filter":{"$expr":{"$regex":["$A","x"]}}}"#).is_err());
    assert!(parse(br#"{"filter":{"$expr":{"$gt":["$A"]}}}"#).is_err());
    assert!(parse(br#"{"filter":{"$expr":{"$gt":[{"x":1},"$A"]}}}"#).is_err());
}

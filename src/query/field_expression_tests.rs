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
fn composes_expression_comparisons_with_boolean_operators() {
    let values = json!({
        "AGE": 29,
        "ACTIVE": true,
        "ROLE": "author"
    });
    let filter = json!({
        "$expr": {
            "$and": [
                {"$gte": ["$AGE", 18]},
                {"$or": [
                    {"$eq": ["$ACTIVE", true]},
                    {"$eq": ["$ROLE", "admin"]}
                ]},
                {"$not": {"$eq": ["$ROLE", "guest"]}}
            ]
        }
    });

    assert!(matches_filter(values.as_object().unwrap(), filter.as_object().unwrap()).unwrap());
}

#[test]
fn compares_numeric_expression_results() {
    let values = json!({
        "PRICE": 12,
        "TAX": 1.5,
        "TOTAL": 13.5,
    });
    let filter = json!({
        "$expr": {"$eq": [
            {"$add": ["$PRICE", "$TAX"]},
            "$TOTAL"
        ]}
    });

    assert!(matches_filter(values.as_object().unwrap(), filter.as_object().unwrap()).unwrap());
}

#[test]
fn compares_multiplication_expression_results() {
    let values = json!({
        "PRICE": 12,
        "QUANTITY": 3,
        "TOTAL": 36,
    });
    let filter = json!({
        "$expr": {"$eq": [
            {"$multiply": ["$PRICE", "$QUANTITY"]},
            "$TOTAL"
        ]}
    });

    assert!(matches_filter(values.as_object().unwrap(), filter.as_object().unwrap()).unwrap());
}

#[test]
fn compares_division_expression_results() {
    let values = json!({
        "WHOLE": 12,
        "PARTS": 3,
        "ODD": 5,
        "FRACTION": 2.5,
    });
    let filter = json!({
        "$expr": {"$and": [
            {"$eq": [{"$divide": ["$WHOLE", "$PARTS"]}, 4]},
            {"$eq": [{"$divide": ["$ODD", "$PARTS"]}, "$FRACTION"]}
        ]}
    });

    assert!(matches_filter(values.as_object().unwrap(), filter.as_object().unwrap()).unwrap());
}

#[test]
fn rejects_division_by_zero() {
    let filter = json!({
        "$expr": {"$eq": [{"$divide": ["$VALUE", 0]}, 1]}
    });

    let error = matches_filter(
        json!({"VALUE": 1}).as_object().unwrap(),
        filter.as_object().unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("cannot divide by zero"));
}

#[test]
fn missing_or_non_numeric_expression_fields_do_not_match() {
    let missing = json!({
        "$expr": {"$eq": [{"$add": ["$MISSING", 1]}, 1]}
    });
    let non_numeric = json!({"$expr": {"$eq": [{"$subtract": ["$NAME", 1]}, 0]}});

    assert!(
        !matches_filter(
            json!({"VALUE": 1}).as_object().unwrap(),
            missing.as_object().unwrap()
        )
        .unwrap()
    );
    assert!(
        !matches_filter(
            json!({"NAME": "one"}).as_object().unwrap(),
            non_numeric.as_object().unwrap()
        )
        .unwrap()
    );
}

#[test]
fn rejects_numeric_expression_results_that_do_not_fit_json() {
    let filter = json!({
        "$expr": {"$eq": [{"$add": [u64::MAX, 1]}, 0]}
    });

    let error =
        matches_filter(json!({}).as_object().unwrap(), filter.as_object().unwrap()).unwrap_err();
    assert!(error.to_string().contains("does not fit JSON"));
}

#[test]
fn rejects_unsupported_or_malformed_expr() {
    assert!(parse(br#"{"filter":{"$expr":{"$regex":["$A","x"]}}}"#).is_err());
    assert!(parse(br#"{"filter":{"$expr":{"$gt":["$A"]}}}"#).is_err());
    assert!(parse(br#"{"filter":{"$expr":{"$gt":[{"x":1},"$A"]}}}"#).is_err());
    assert!(parse(br#"{"filter":{"$expr":{"$and":["$A"]}}}"#).is_err());
    assert!(parse(br#"{"filter":{"$expr":{"$not":[{"$eq":["$A",1]}]}}}"#).is_err());
    assert!(parse(br#"{"filter":{"$expr":{"$eq":[{"$add":["$A",true]},1]}}}"#).is_err());
    assert!(parse(br#"{"filter":{"$expr":{"$eq":[{"$add":["$A"]},1]}}}"#).is_err());
    assert!(parse(br#"{"filter":{"$expr":{"$eq":[{"$multiply":["$A",true]},1]}}}"#).is_err());
    assert!(parse(br#"{"filter":{"$expr":{"$eq":[{"$divide":["$A",true]},1]}}}"#).is_err());
}

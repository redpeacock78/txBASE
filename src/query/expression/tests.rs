use super::{
    EvaluationContext, QueryError, evaluate_scalar, matches, parse_scalar_operand,
    scope::MAX_ARRAY_EXPRESSION_ITERATIONS, uses_only_group_key_fields,
};
use serde_json::{Value, json};

fn evaluate(document: Value, expression: Value) -> Result<Option<Value>, QueryError> {
    let values = document.as_object().unwrap().clone();
    let expression = parse_scalar_operand(&expression, "test.expression")?;
    evaluate_scalar(&values, &expression, "test.expression")
}

fn evaluate_value(document: Value, expression: Value) -> Value {
    evaluate(document, expression).unwrap().unwrap()
}

#[test]
fn maps_filters_and_reduces_with_item_and_index_bindings() {
    let document = json!({
        "ITEMS": [
            {"score": 2},
            {"score": 7},
            {"score": 9}
        ]
    });

    assert_eq!(
        evaluate_value(
            document.clone(),
            json!({"$map": {
                "input": "$ITEMS",
                "as": "item",
                "arrayIndexAs": "position",
                "in": ["$$item.score", {"$add": ["$$position", 1]}]
            }})
        ),
        json!([[2, 1], [7, 2], [9, 3]])
    );
    assert_eq!(
        evaluate_value(
            document.clone(),
            json!({"$filter": {
                "input": "$ITEMS",
                "as": "item",
                "cond": {"$gt": ["$$item.score", 5]},
                "limit": {"$literal": 1}
            }})
        ),
        json!([{"score": 7}])
    );
    assert_eq!(
        evaluate_value(
            document.clone(),
            json!({"$filter": {
                "input": "$ITEMS",
                "as": "item",
                "cond": {"$gt": ["$$item.score", 5]},
                "limit": {"$literal": null}
            }})
        ),
        json!([{"score": 7}, {"score": 9}])
    );
    assert_eq!(
        evaluate_value(
            document,
            json!({"$reduce": {
                "input": "$ITEMS",
                "initialValue": 0,
                "as": "item",
                "valueAs": "total",
                "in": {"$add": ["$$total", "$$item.score"]}
            }})
        ),
        json!(18)
    );
}

#[test]
fn scopes_shadow_lexically_and_expose_root_current_and_default_index() {
    let document = json!({
        "NAME": "root",
        "ITEMS": [{"name": "inner", "values": [10, 11]}]
    });
    let expression = json!({"$let": {
        "vars": {"item": "outer"},
        "in": {"$map": {
            "input": "$ITEMS",
            "as": "item",
            "in": [
                "$$item.name",
                "$$ROOT.NAME",
                "$$CURRENT.NAME",
                "$$IDX",
                {"$map": {
                    "input": "$$item.values",
                    "as": "item",
                    "in": ["$$item", "$$IDX"]
                }}
            ]
        }}
    }});

    assert_eq!(
        evaluate_value(document, expression),
        json!([["inner", "root", "root", 0, [[10, 0], [11, 1]]]])
    );
}

#[test]
fn let_bindings_are_simultaneous_and_confined_to_the_in_expression() {
    let document = json!({});
    assert_eq!(
        evaluate_value(
            document.clone(),
            json!({"$let": {
                "vars": {"x": 7},
                "in": {"$let": {
                    "vars": {"x": 2, "y": "$$x"},
                    "in": ["$$x", "$$y"]
                }}
            }})
        ),
        json!([2, 7])
    );

    let error = evaluate(
        document,
        json!({"$let": {"vars": {"x": 2, "y": "$$x"}, "in": "$$y"}}),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("unbound expression variable $$x"));
}

#[test]
fn array_inputs_follow_null_empty_and_type_boundaries() {
    assert_eq!(
        evaluate_value(
            json!({}),
            json!({"$map": {"input": "$MISSING", "in": "$$this"}})
        ),
        Value::Null
    );
    assert_eq!(
        evaluate_value(
            json!({"ITEMS": null}),
            json!({"$filter": {"input": "$ITEMS", "cond": {"$eq": ["$$this", 1]}}})
        ),
        Value::Null
    );
    assert_eq!(
        evaluate_value(
            json!({"ITEMS": []}),
            json!({"$reduce": {"input": "$ITEMS", "initialValue": 7, "in": "$$value"}})
        ),
        json!(7)
    );
    assert!(
        evaluate(
            json!({"ITEMS": 3}),
            json!({"$map": {"input": "$ITEMS", "in": "$$this"}})
        )
        .is_err()
    );
    assert!(
        evaluate(
            json!({"ITEMS": [1, 2]}),
            json!({"$filter": {
                "input": "$ITEMS",
                "cond": {"$eq": ["$$this", 1]},
                "limit": 0
            }})
        )
        .is_err()
    );
    assert!(
        evaluate(
            json!({"ITEMS": [1, 2]}),
            json!({"$filter": {
                "input": "$ITEMS",
                "cond": {"$eq": ["$$this", 1]},
                "limit": "$MISSING"
            }})
        )
        .is_err()
    );
}

#[test]
fn rejects_invalid_array_shapes_and_variable_aliases() {
    let invalid = [
        json!({"$map": {"input": [], "in": "$$this", "unknown": true}}),
        json!({"$map": {"input": []}}),
        json!({"$filter": {"input": []}}),
        json!({"$map": {"input": [], "as": "same", "arrayIndexAs": "same", "in": "$$same"}}),
        json!({"$reduce": {
            "input": [], "initialValue": 0, "as": "same", "valueAs": "same", "in": "$$same"
        }}),
        json!({"$let": {"vars": {"bad-name": 1}, "in": 1}}),
        json!({"$let": {"vars": {}, "in": 1, "unknown": 2}}),
    ];

    for expression in invalid {
        assert!(evaluate(json!({}), expression).is_err());
    }
}

#[test]
fn group_key_provenance_tracks_scoped_array_and_let_variables() {
    for expression in [
        json!({"$let": {
            "vars": {"key": "$_id.parts"},
            "in": {"$map": {"input": "$$key", "as": "part", "in": "$$part"}}
        }}),
        json!({"$reduce": {
            "input": "$_id.parts",
            "initialValue": 0,
            "as": "part",
            "valueAs": "total",
            "in": {"$add": ["$$total", "$$part"]}
        }}),
        json!({"$filter": {
            "input": "$_id.parts",
            "as": "part",
            "cond": {"$gt": ["$$IDX", 0]}
        }}),
        json!({"$let": {
            "vars": {"name": "$_id.name"},
            "in": {"$strLenCP": "$$name"}
        }}),
        json!({"$substrCP": ["$_id.name", 0, 2]}),
        json!({"$split": ["$_id.name", "/"]}),
        json!({"$indexOfCP": ["$_id.name", "x", 0, 2]}),
        json!({"$replaceOne": {
            "input": "$_id.name", "find": "x", "replacement": "y"
        }}),
        json!({"$replaceAll": {
            "input": "$_id.name", "find": "x", "replacement": "y"
        }}),
    ] {
        let expression = parse_scalar_operand(&expression, "test.expression").unwrap();
        assert!(uses_only_group_key_fields(&expression));
    }

    for expression in [
        json!({"$let": {"vars": {"key": "$$ROOT._id"}, "in": "$$key"}}),
        json!({"$map": {"input": "$OTHER.parts", "as": "part", "in": "$$part"}}),
        json!({"$map": {"input": "$_id.parts", "in": "$$ROOT._id"}}),
        json!({"$strLenCP": "$OTHER.name"}),
        json!({"$substrCP": ["$_id.name", "$OTHER.start", 2]}),
        json!({"$split": ["$_id.name", "$OTHER.delimiter"]}),
        json!({"$indexOfCP": ["$_id.name", "$OTHER.search", 0, 2]}),
        json!({"$indexOfCP": ["$_id.name", "x", "$OTHER.start", 2]}),
        json!({"$indexOfCP": ["$_id.name", "x", 0, "$OTHER.end"]}),
        json!({"$replaceAll": {
            "input": "$_id.name", "find": "x", "replacement": "$OTHER.text"
        }}),
    ] {
        let expression = parse_scalar_operand(&expression, "test.expression").unwrap();
        assert!(!uses_only_group_key_fields(&expression));
    }
}

#[test]
fn array_evaluation_budgets_reject_work_and_output_past_the_limit() {
    let values = json!({}).as_object().unwrap().clone();
    let mut context = EvaluationContext::new(&values);
    for _ in 0..MAX_ARRAY_EXPRESSION_ITERATIONS {
        context.visit_array_item("test").unwrap();
    }
    assert!(context.visit_array_item("test").is_err());

    let mut output = Vec::new();
    let mut encoded_bytes = usize::MAX;
    assert!(
        super::array::push_array_value(&mut output, &mut encoded_bytes, json!(1), "test").is_err()
    );
}

#[test]
fn expression_predicates_short_circuit_inside_scopes() {
    let values = json!({}).as_object().unwrap().clone();
    assert!(
        !matches(
            &values,
            &json!({"$and": [
                {"$eq": [1, 2]},
                {"$eq": ["$$missing", 0]}
            ]})
        )
        .unwrap()
    );
}

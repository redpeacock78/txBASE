use super::super::{QueryError, evaluate_scalar, parse_scalar_operand};
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
fn string_length_and_substring_count_unicode_code_points() {
    let document = json!({"TEXT": "寿司🦀a"});
    assert_eq!(
        evaluate_value(document.clone(), json!({"$strLenCP": "$TEXT"})),
        json!(4)
    );
    assert_eq!(
        evaluate_value(document.clone(), json!({"$substrCP": ["$TEXT", 1, 2.0]})),
        json!("司🦀")
    );
    assert_eq!(
        evaluate_value(document.clone(), json!({"$substrCP": ["$TEXT", 99, 1]})),
        json!("")
    );
    assert_eq!(
        evaluate_value(json!({"TEXT": null}), json!({"$substrCP": ["$TEXT", 0, 2]})),
        json!("")
    );
    assert_eq!(
        evaluate_value(json!({}), json!({"$substrCP": ["$MISSING", 0, 2]})),
        json!("")
    );
    assert!(evaluate(json!({}), json!({"$strLenCP": "$MISSING"})).is_err());
    assert!(evaluate(json!({"TEXT": null}), json!({"$strLenCP": "$TEXT"})).is_err());
    assert!(evaluate(json!({}), json!({"$substrCP": ["text", -1, 1]})).is_err());
    assert!(evaluate(json!({}), json!({"$substrCP": ["text", 0, 1.5]})).is_err());
}

#[test]
fn split_preserves_empty_fields_and_bounds_its_array_result() {
    assert_eq!(
        evaluate_value(json!({}), json!({"$split": ["::a::::b::", "::"]})),
        json!(["", "a", "", "b", ""])
    );
    assert_eq!(
        evaluate_value(json!({}), json!({"$split": ["whole", "/"]})),
        json!(["whole"])
    );
    assert_eq!(
        evaluate_value(json!({}), json!({"$split": ["", "/"]})),
        json!([""])
    );
    assert!(evaluate(json!({}), json!({"$split": ["text", ""]})).is_err());
    assert!(evaluate(json!({}), json!({"$split": [null, "/"]})).is_err());
    assert!(evaluate(json!({}), json!({"$split": ["text", 1]})).is_err());

    let input = "a".repeat(400_000);
    assert!(evaluate(json!({"TEXT": input}), json!({"$split": ["$TEXT", "a"]})).is_err());
}

#[test]
fn index_of_cp_uses_code_point_offsets_and_half_open_ranges() {
    assert_eq!(
        evaluate_value(json!({}), json!({"$indexOfCP": ["寿司a寿司", "a"]})),
        json!(2)
    );
    assert_eq!(
        evaluate_value(json!({}), json!({"$indexOfCP": ["寿司a寿司", "寿司", 1]})),
        json!(3)
    );
    assert_eq!(
        evaluate_value(json!({}), json!({"$indexOfCP": ["a寿司a", "a", 1, 99]})),
        json!(3)
    );
    assert_eq!(
        evaluate_value(json!({}), json!({"$indexOfCP": ["a寿司a", "a", 0, 3]})),
        json!(0)
    );
    assert_eq!(
        evaluate_value(json!({}), json!({"$indexOfCP": ["abc", "a", 4]})),
        json!(-1)
    );
    assert_eq!(
        evaluate_value(json!({}), json!({"$indexOfCP": ["abc", "a", 2, 1]})),
        json!(-1)
    );
    assert_eq!(
        evaluate_value(json!({}), json!({"$indexOfCP": ["abc", "", 2]})),
        json!(2)
    );
    assert_eq!(
        evaluate_value(json!({"TEXT": null}), json!({"$indexOfCP": ["$TEXT", "a"]})),
        Value::Null
    );
    assert_eq!(
        evaluate_value(json!({}), json!({"$indexOfCP": ["$MISSING", "a"]})),
        Value::Null
    );
    assert!(evaluate(json!({}), json!({"$indexOfCP": ["abc", "$MISSING"]})).is_err());
    assert!(evaluate(json!({}), json!({"$indexOfCP": ["abc", "a", -1]})).is_err());
    assert!(evaluate(json!({}), json!({"$indexOfCP": ["abc", "a", 1.5]})).is_err());
}

#[test]
fn replace_operators_are_literal_case_sensitive_and_null_aware() {
    let one = json!({"$replaceOne": {
        "input": "$TEXT", "find": "an", "replacement": "X"
    }});
    let all = json!({"$replaceAll": {
        "input": "$TEXT", "find": "an", "replacement": "X"
    }});
    assert_eq!(
        evaluate_value(json!({"TEXT": "banana"}), one),
        json!("bXana")
    );
    assert_eq!(
        evaluate_value(json!({"TEXT": "banana"}), all),
        json!("bXXa")
    );
    assert_eq!(
        evaluate_value(
            json!({"TEXT": "Cafe cafe"}),
            json!({"$replaceOne": {
                "input": "$TEXT", "find": "cafe", "replacement": "X"
            }})
        ),
        json!("Cafe X")
    );
    assert_eq!(
        evaluate_value(
            json!({"TEXT": "cafe\u{301}"}),
            json!({"$replaceAll": {
                "input": "$TEXT", "find": "café", "replacement": "X"
            }})
        ),
        json!("cafe\u{301}")
    );
    assert_eq!(
        evaluate_value(
            json!({"TEXT": "abc"}),
            json!({"$replaceAll": {
                "input": "$TEXT", "find": "$MISSING", "replacement": "X"
            }})
        ),
        Value::Null
    );
    assert_eq!(
        evaluate_value(
            json!({"TEXT": "abc"}),
            json!({"$replaceOne": {
                "input": "$TEXT", "find": "a", "replacement": "$MISSING"
            }})
        ),
        Value::Null
    );
    assert!(
        evaluate(
            json!({"TEXT": "abc"}),
            json!({"$replaceAll": {
                "input": "$TEXT", "find": "a", "replacement": 1
            }})
        )
        .is_err()
    );
    assert!(
        evaluate(
            json!({"TEXT": "abc"}),
            json!({"$replaceOne": {
                "input": "$TEXT", "find": "", "replacement": "X"
            }})
        )
        .is_err()
    );
}

#[test]
fn replacement_output_is_rejected_before_it_exceeds_the_string_limit() {
    let input = "a".repeat(300_000);
    assert!(
        evaluate(
            json!({"TEXT": input}),
            json!({"$replaceAll": {
                "input": "$TEXT", "find": "a", "replacement": "aaaa"
            }})
        )
        .is_err()
    );
}

#[test]
fn string_expression_shapes_reject_wrong_arity_and_unknown_options() {
    for expression in [
        json!({"$substrCP": ["text", 0]}),
        json!({"$split": ["text"]}),
        json!({"$indexOfCP": ["text"]}),
        json!({"$indexOfCP": ["text", "x", 0, 1, 2]}),
        json!({"$replaceOne": {"input": "text", "find": "t"}}),
        json!({"$replaceAll": {
            "input": "text", "find": "t", "replacement": "T", "options": {}
        }}),
    ] {
        assert!(evaluate(json!({}), expression).is_err());
    }
}

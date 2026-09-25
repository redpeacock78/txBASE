use super::matches_filter;
use serde_json::{Map, Value, json};
use std::cmp::Ordering;

#[test]
fn scalar_predicates_match_the_independent_reference_matrix() {
    let records = generated_records();
    let filters = generated_filters();
    assert!(records.len() * filters.len() >= 1_000);

    for (filter_index, filter) in filters.iter().enumerate() {
        let filter = filter.as_object().expect("generated filter is an object");
        for (record_index, record) in records.iter().enumerate() {
            let actual = matches_filter(record, filter).unwrap_or_else(|error| {
                panic!("generated filter {filter_index} is invalid: {error}")
            });
            assert_eq!(
                actual,
                reference_filter(record, filter),
                "filter {filter_index} {filter:?} disagreed for record {record_index}: {record:?}"
            );
        }
    }
}

fn generated_records() -> Vec<Map<String, Value>> {
    let mut records = Vec::new();
    for number in -4_i64..=4 {
        for active in [false, true] {
            let mut record = json!({
                "N": number,
                "ACTIVE": active,
                "LABEL": if active { "ALICE" } else { "Alice" },
            });
            let fields = record.as_object_mut().unwrap();
            match number.rem_euclid(3) {
                0 => {
                    fields.insert("OPTIONAL".into(), Value::Null);
                }
                1 => {
                    fields.insert("OPTIONAL".into(), json!(number));
                }
                _ => {}
            }
            records.push(fields.clone());
        }
    }
    records
}

fn generated_filters() -> Vec<Value> {
    let mut filters = Vec::new();
    for operator in ["$eq", "$ne", "$gt", "$gte", "$lt", "$lte"] {
        for expected in [
            json!(-5),
            json!(-2),
            json!(0),
            json!(2),
            json!(5),
            json!(1.5),
        ] {
            filters.push(field_filter("N", operator, expected));
        }
    }
    for candidates in [
        json!([]),
        json!([-4, -1, 4]),
        json!([2, 5]),
        json!(["other", null]),
    ] {
        for operator in ["$in", "$nin"] {
            filters.push(field_filter("N", operator, candidates.clone()));
        }
    }
    for operator in ["$eq", "$ne"] {
        for expected in [json!(null), json!(-3), json!(0), json!(4)] {
            filters.push(field_filter("OPTIONAL", operator, expected));
        }
        for expected in [json!(false), json!(true)] {
            filters.push(field_filter("ACTIVE", operator, expected));
        }
        for expected in [json!("Alice"), json!("ALICE"), json!("guest")] {
            filters.push(field_filter("LABEL", operator, expected));
        }
    }
    for operator in ["$in", "$nin"] {
        filters.push(field_filter("LABEL", operator, json!(["ALICE", "guest"])));
    }
    filters.extend([
        json!({"$and": [{"N": {"$gte": -2}}, {"ACTIVE": true}]}),
        json!({"$or": [{"N": {"$lt": -2}}, {"ACTIVE": true}]}),
        json!({"$not": {"N": {"$lt": 0}}}),
        json!({"$not": {"OPTIONAL": {"$eq": 1}}}),
    ]);
    filters
}

fn field_filter(field: &str, operator: &str, operand: Value) -> Value {
    let mut condition = Map::new();
    condition.insert(operator.into(), operand);
    let mut filter = Map::new();
    filter.insert(field.into(), Value::Object(condition));
    Value::Object(filter)
}

fn reference_filter(record: &Map<String, Value>, filter: &Map<String, Value>) -> bool {
    filter
        .iter()
        .all(|(field, condition)| match field.as_str() {
            "$and" => condition.as_array().is_some_and(|clauses| {
                clauses.iter().all(|clause| {
                    clause
                        .as_object()
                        .is_some_and(|clause| reference_filter(record, clause))
                })
            }),
            "$or" => condition.as_array().is_some_and(|clauses| {
                clauses.iter().any(|clause| {
                    clause
                        .as_object()
                        .is_some_and(|clause| reference_filter(record, clause))
                })
            }),
            "$not" => condition
                .as_object()
                .is_some_and(|clause| !reference_filter(record, clause)),
            _ => reference_condition(record.get(field), condition),
        })
}

fn reference_condition(actual: Option<&Value>, condition: &Value) -> bool {
    let Some(operators) = condition
        .as_object()
        .filter(|object| object.keys().any(|key| key.starts_with('$')))
    else {
        return actual.is_some_and(|actual| actual == condition);
    };
    operators
        .iter()
        .all(|(operator, expected)| match operator.as_str() {
            "$eq" => actual.is_some_and(|actual| actual == expected),
            "$ne" => actual.is_none_or(|actual| actual != expected),
            "$gt" => reference_compare(actual, expected, Ordering::is_gt),
            "$gte" => reference_compare(actual, expected, Ordering::is_ge),
            "$lt" => reference_compare(actual, expected, Ordering::is_lt),
            "$lte" => reference_compare(actual, expected, Ordering::is_le),
            "$in" => expected.as_array().is_some_and(|candidates| {
                actual.is_some_and(|actual| candidates.iter().any(|candidate| candidate == actual))
            }),
            "$nin" => expected.as_array().is_some_and(|candidates| {
                actual.is_none_or(|actual| candidates.iter().all(|candidate| candidate != actual))
            }),
            "$not" => !reference_condition(actual, expected),
            _ => false,
        })
}

fn reference_compare(
    actual: Option<&Value>,
    expected: &Value,
    predicate: impl FnOnce(Ordering) -> bool,
) -> bool {
    let (Some(actual), Some(expected)) = (actual.and_then(Value::as_f64), expected.as_f64()) else {
        return false;
    };
    actual.partial_cmp(&expected).is_some_and(predicate)
}

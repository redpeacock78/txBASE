use super::*;
use crate::index::{IndexDefinition, IndexFile};
use serde_json::Number;
use std::fs;

fn table_with_two_active_records() -> DbfTable {
    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    DbfTable::from_bytes(&bytes).unwrap()
}

#[test]
fn parses_query_shape() {
    let query = parse(
        br#"{
            "filter": {"age": {"$gte": 20}},
            "sort": {"age": 1},
            "projection": {"name": 1},
            "limit": 10
        }"#,
    )
    .unwrap();

    assert_eq!(query.sort["age"], 1);
    assert_eq!(query.limit, Some(10));
}

#[test]
fn rejects_invalid_sort_direction() {
    let error = parse(br#"{"sort":{"age":2}}"#).unwrap_err();
    assert!(error.to_string().contains("must be 1 or -1"));
}

#[test]
fn executes_filter_sort_projection_and_pagination() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "filter": {"AGE": {"$gte": 7}},
            "sort": {"AGE": -1},
            "projection": {"NAME": 1, "AGE": 1},
            "skip": 1,
            "limit": 1
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![serde_json::json!({
            "NAME": "Bob",
            "AGE": 7
        })]
    );
}

#[test]
fn supports_dotted_paths_for_nested_values() {
    let first_values = serde_json::json!({
        "PROFILE": {
            "CITY": "Tokyo",
            "TAGS": [{"NAME": "jp"}, {"NAME": "db"}]
        }
    })
    .as_object()
    .unwrap()
    .clone();
    let second_values = serde_json::json!({
        "PROFILE": {"CITY": "Osaka"}
    })
    .as_object()
    .unwrap()
    .clone();
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: first_values.clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: second_values,
    };

    assert!(
        matches_filter(
            &first_values,
            serde_json::json!({
                "PROFILE.CITY": "Tokyo",
                "PROFILE.TAGS.NAME": {"$in": ["db"]}
            })
            .as_object()
            .unwrap()
        )
        .unwrap()
    );
    assert_eq!(
        compare_records(
            &first,
            &second,
            &IndexMap::from([(String::from("PROFILE.CITY"), 1)])
        ),
        Ordering::Greater
    );

    let projection = BTreeMap::from([
        (String::from("PROFILE.CITY"), 1),
        (String::from("PROFILE.TAGS.NAME"), 1),
    ]);
    assert_eq!(
        project(&first, &projection),
        serde_json::json!({
            "PROFILE": {
                "CITY": "Tokyo",
                "TAGS": {"NAME": ["jp", "db"]}
            }
        })
    );

    let exclusion = BTreeMap::from([(String::from("PROFILE.CITY"), 0)]);
    assert_eq!(
        project(&first, &exclusion),
        serde_json::json!({
            "PROFILE": {"TAGS": [{"NAME": "jp"}, {"NAME": "db"}]}
        })
    );

    let mut literal_values = first_values;
    literal_values.insert(
        String::from("PROFILE.CITY"),
        Value::String("literal".into()),
    );
    assert_eq!(
        field_value(&literal_values, "PROFILE.CITY"),
        Some(Value::String("literal".into()))
    );
}

#[test]
fn supports_explicit_array_indices_in_paths() {
    let values = serde_json::json!({
        "PROFILE": {
            "TAGS": [{"NAME": "jp"}, {"NAME": "db"}]
        }
    })
    .as_object()
    .unwrap()
    .clone();
    let other_values = serde_json::json!({
        "PROFILE": {
            "TAGS": [{"NAME": "aa"}]
        }
    })
    .as_object()
    .unwrap()
    .clone();
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: values.clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: other_values,
    };

    assert_eq!(
        field_value(&values, "PROFILE.TAGS.0.NAME"),
        Some(Value::String("jp".into()))
    );
    assert_eq!(field_value(&values, "PROFILE.TAGS.2.NAME"), None);
    assert!(
        matches_filter(
            &values,
            serde_json::json!({"PROFILE.TAGS.1.NAME": "db"})
                .as_object()
                .unwrap()
        )
        .unwrap()
    );
    assert_eq!(
        compare_records(
            &first,
            &second,
            &IndexMap::from([(String::from("PROFILE.TAGS.1.NAME"), 1)])
        ),
        Ordering::Greater
    );

    let inclusion = BTreeMap::from([(String::from("PROFILE.TAGS.1.NAME"), 1)]);
    assert_eq!(
        project(&first, &inclusion),
        serde_json::json!({
            "PROFILE": {"TAGS": [null, {"NAME": "db"}]}
        })
    );

    let exclusion = BTreeMap::from([(String::from("PROFILE.TAGS.0.NAME"), 0)]);
    assert_eq!(
        project(&first, &exclusion),
        serde_json::json!({
            "PROFILE": {"TAGS": [{}, {"NAME": "db"}]}
        })
    );
}

#[test]
fn preserves_multi_key_sort_order() {
    let table = table_with_two_active_records();
    let request = parse(br#"{"sort":{"NAME":1,"AGE":1}}"#).unwrap();
    let records = execute_query(&table, &request).unwrap();

    assert_eq!(records[0]["NAME"], "Alice");
    assert_eq!(records[1]["NAME"], "Bob");
}

#[test]
fn executes_logical_and_membership_predicates() {
    let table = table_with_two_active_records();
    let request = parse(br#"{"filter":{"$and":[{"AGE":{"$in":[29]}},{"ACTIVE":true}]}}"#).unwrap();

    assert_eq!(execute_query(&table, &request).unwrap().len(), 1);
}

#[test]
fn rejects_unknown_and_mixed_projection_operators() {
    assert!(parse(br#"{"filter":{"AGE":{"$regex":"2"}}}"#).is_err());
    assert!(parse(br#"{"projection":{"NAME":1,"AGE":0}}"#).is_err());
}

#[test]
fn rejects_unknown_query_fields() {
    assert!(parse(br#"{"filtre":{"AGE":29}}"#).is_err());
}

#[test]
fn compares_large_integer_values_exactly() {
    let maximum = Value::Number(Number::from(u64::MAX));
    let condition = serde_json::json!({"$gt": u64::MAX - 1});

    assert!(matches_condition(Some(&maximum), &condition).unwrap());
}

#[test]
fn uses_a_valid_equality_index_and_preserves_scan_results() {
    let path =
        std::env::temp_dir().join(format!("txbase-query-planner-{}.dbf", std::process::id()));
    let sidecar = crate::index::sidecar_path(&path);
    let lock = path.with_extension("txbase.lock");
    let wal = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&sidecar);
    let _ = fs::remove_file(&lock);
    let _ = fs::remove_file(&wal);

    let bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    let mut bytes = bytes;
    bytes[179] = b' ';
    let table = DbfTable::from_bytes(&bytes).unwrap();
    fs::write(&path, bytes).unwrap();
    IndexFile::build(
        &path,
        vec![
            IndexDefinition::named("by_name", "NAME"),
            IndexDefinition::named("by_age", "AGE"),
        ],
    )
    .unwrap()
    .save(&path)
    .unwrap();

    let request = parse(br#"{"filter":{"ACTIVE":true,"NAME":"Alice"}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::EqualityIndex {
            name: "by_name".into(),
            field: "NAME".into(),
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &request).unwrap(),
        execute_query(&table, &request).unwrap()
    );

    let intersection_request = parse(br#"{"filter":{"NAME":"Alice","AGE":29}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &intersection_request).unwrap(),
        QueryPlan::IndexIntersection {
            names: vec!["by_age".into(), "by_name".into()],
            fields: vec!["AGE".into(), "NAME".into()],
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &intersection_request).unwrap(),
        execute_query(&table, &intersection_request).unwrap()
    );

    let empty_intersection_request = parse(br#"{"filter":{"NAME":"Alice","AGE":7}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &empty_intersection_request).unwrap(),
        QueryPlan::IndexIntersection {
            names: vec!["by_age".into(), "by_name".into()],
            fields: vec!["AGE".into(), "NAME".into()],
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &empty_intersection_request).unwrap(),
        execute_query(&table, &empty_intersection_request).unwrap()
    );

    let range_request = parse(br#"{"filter":{"AGE":{"$gt":7,"$lt":31}}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &range_request).unwrap(),
        QueryPlan::RangeIndex {
            name: "by_age".into(),
            field: "AGE".into(),
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &range_request).unwrap(),
        execute_query(&table, &range_request).unwrap()
    );

    let lower_bound_request = parse(br#"{"filter":{"NAME":{"$gte":"Bob"}}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &lower_bound_request).unwrap(),
        QueryPlan::RangeIndex {
            name: "by_name".into(),
            field: "NAME".into(),
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &lower_bound_request).unwrap(),
        execute_query(&table, &lower_bound_request).unwrap()
    );

    let mismatched_bound_request = parse(br#"{"filter":{"NAME":{"$gt":7}}}"#).unwrap();
    assert_eq!(
        execute_query_at(&table, &path, &mismatched_bound_request).unwrap(),
        execute_query(&table, &mismatched_bound_request).unwrap()
    );

    let sort_request = parse(br#"{"sort":{"NAME":-1}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &sort_request).unwrap(),
        QueryPlan::OrderedIndex {
            name: "by_name".into(),
            field: "NAME".into(),
            direction: -1,
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &sort_request).unwrap(),
        execute_query(&table, &sort_request).unwrap()
    );

    fs::remove_file(&sidecar).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::TableScan
    );
    assert_eq!(
        execute_query_at(&table, &path, &request).unwrap(),
        execute_query(&table, &request).unwrap()
    );

    fs::remove_file(path).unwrap();
    let _ = fs::remove_file(lock);
    let _ = fs::remove_file(wal);
}

#[test]
fn orders_equality_intersection_by_candidate_cardinality() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-selectivity-{}.dbf",
        std::process::id()
    ));
    let sidecar = crate::index::sidecar_path(&path);
    let lock = path.with_extension("txbase.lock");
    let wal = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&sidecar);
    let _ = fs::remove_file(&lock);
    let _ = fs::remove_file(&wal);

    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[176..178].copy_from_slice(b"07");
    bytes[179] = b' ';
    let table = DbfTable::from_bytes(&bytes).unwrap();
    fs::write(&path, bytes).unwrap();
    IndexFile::build(
        &path,
        vec![
            IndexDefinition::named("by_name", "NAME"),
            IndexDefinition::named("by_age", "AGE"),
        ],
    )
    .unwrap()
    .save(&path)
    .unwrap();

    let request = parse(br#"{"filter":{"AGE":7,"NAME":"Alice"}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::IndexIntersection {
            names: vec!["by_name".into(), "by_age".into()],
            fields: vec!["NAME".into(), "AGE".into()],
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &request).unwrap(),
        execute_query(&table, &request).unwrap()
    );

    fs::remove_file(&sidecar).unwrap();
    fs::remove_file(path).unwrap();
    let _ = fs::remove_file(lock);
    let _ = fs::remove_file(wal);
}

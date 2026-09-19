use super::*;
use crate::index::{IndexDefinition, IndexFile};
use std::fs;

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
fn orders_equality_intersection_by_index_statistics() {
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

#[test]
fn orders_multiple_range_access_by_histogram_estimate() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-range-statistics-{}.dbf",
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
    bytes[179] = b' ';
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    for (id, name, age) in [(3, "Carol", 42), (4, "Dave", 43)] {
        table
            .insert_record(
                serde_json::json!({
                    "ID": id,
                    "NAME": name,
                    "AGE": age,
                    "ACTIVE": true
                })
                .as_object()
                .unwrap()
                .clone(),
            )
            .unwrap();
    }
    fs::write(&path, table.to_bytes()).unwrap();
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

    let request = parse(br#"{"filter":{"AGE":{"$gte":0},"NAME":{"$gte":"C"}}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::RangeIndex {
            name: "by_name".into(),
            field: "NAME".into(),
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

#[test]
fn uses_an_ordered_index_prefix_for_multi_key_sort() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-ordered-prefix-{}.dbf",
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
    bytes[179] = b' ';
    bytes[183..193].copy_from_slice(b"Alice     ");
    let table = DbfTable::from_bytes(&bytes).unwrap();
    fs::write(&path, bytes).unwrap();
    IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")])
        .unwrap()
        .save(&path)
        .unwrap();

    let request = parse(br#"{"sort":{"NAME":1,"AGE":1}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::OrderedIndexPrefix {
            name: "NAME".into(),
            field: "NAME".into(),
            direction: 1,
        }
    );
    let indexed = execute_query_at(&table, &path, &request).unwrap();
    assert_eq!(indexed, execute_query(&table, &request).unwrap());
    assert_eq!(indexed[0]["AGE"], 7);

    fs::remove_file(&sidecar).unwrap();
    fs::remove_file(path).unwrap();
    let _ = fs::remove_file(lock);
    let _ = fs::remove_file(wal);
}

#[test]
fn uses_a_compound_index_for_multi_key_sort() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-compound-{}.dbf",
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
    bytes[179] = b' ';
    bytes[183..193].copy_from_slice(b"Alice     ");
    let table = DbfTable::from_bytes(&bytes).unwrap();
    fs::write(&path, bytes).unwrap();
    IndexFile::build(
        &path,
        vec![IndexDefinition::named_fields(
            "by_name_age",
            vec!["NAME".into(), "AGE".into()],
        )],
    )
    .unwrap()
    .save(&path)
    .unwrap();

    let request = parse(br#"{"sort":{"NAME":1,"AGE":1}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::CompoundOrderedIndex {
            name: "by_name_age".into(),
            fields: vec!["NAME".into(), "AGE".into()],
            directions: vec![1, 1],
        }
    );
    let indexed = execute_query_at(&table, &path, &request).unwrap();
    assert_eq!(indexed, execute_query(&table, &request).unwrap());
    assert_eq!(indexed[0]["AGE"], 7);

    let descending = parse(br#"{"sort":{"NAME":-1,"AGE":-1}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &descending).unwrap(),
        QueryPlan::CompoundOrderedIndex {
            name: "by_name_age".into(),
            fields: vec!["NAME".into(), "AGE".into()],
            directions: vec![1, 1],
        }
    );
    let indexed = execute_query_at(&table, &path, &descending).unwrap();
    assert_eq!(indexed, execute_query(&table, &descending).unwrap());
    assert_eq!(indexed[0]["AGE"], 29);

    let mixed = parse(br#"{"sort":{"NAME":1,"AGE":-1}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &mixed).unwrap(),
        QueryPlan::TableScan
    );
    assert_eq!(
        execute_query_at(&table, &path, &mixed).unwrap(),
        execute_query(&table, &mixed).unwrap()
    );

    fs::remove_file(&sidecar).unwrap();
    fs::remove_file(path).unwrap();
    let _ = fs::remove_file(lock);
    let _ = fs::remove_file(wal);
}

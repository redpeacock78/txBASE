use super::*;
use crate::index::{IndexDefinition, IndexFile};
use std::fs;

fn remove_table_files(path: &std::path::Path) {
    for candidate in [
        path.to_path_buf(),
        crate::index::sidecar_path(path),
        path.with_extension("txbase.wal"),
        path.with_extension("txbase.lock"),
    ] {
        let _ = fs::remove_file(candidate);
    }
}

#[test]
fn uses_a_mixed_direction_compound_index() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-mixed-compound-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&path);

    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    let table = DbfTable::from_bytes(&bytes).unwrap();
    fs::write(&path, bytes).unwrap();
    IndexFile::build(
        &path,
        vec![IndexDefinition::named_fields_with_directions(
            "by_name_age",
            vec!["NAME".into(), "AGE".into()],
            vec![1, -1],
        )],
    )
    .unwrap()
    .save(&path)
    .unwrap();

    let request = parse(br#"{"sort":{"NAME":1,"AGE":-1}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::CompoundOrderedIndex {
            name: "by_name_age".into(),
            fields: vec!["NAME".into(), "AGE".into()],
            directions: vec![1, -1],
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &request).unwrap(),
        execute_query(&table, &request).unwrap()
    );

    let reverse = parse(br#"{"sort":{"NAME":-1,"AGE":1}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &reverse).unwrap(),
        QueryPlan::CompoundOrderedIndex {
            name: "by_name_age".into(),
            fields: vec!["NAME".into(), "AGE".into()],
            directions: vec![1, -1],
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &reverse).unwrap(),
        execute_query(&table, &reverse).unwrap()
    );

    let unsupported = parse(br#"{"sort":{"NAME":1,"AGE":1}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &unsupported).unwrap(),
        QueryPlan::TableScan
    );

    remove_table_files(&path);
}

#[test]
fn uses_a_compound_range_after_an_equality_prefix() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-compound-range-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&path);

    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    let table = DbfTable::from_bytes(&bytes).unwrap();
    fs::write(&path, bytes).unwrap();
    IndexFile::build(
        &path,
        vec![IndexDefinition::named_fields_with_directions(
            "by_active_age",
            vec!["ACTIVE".into(), "AGE".into()],
            vec![-1, 1],
        )],
    )
    .unwrap()
    .save(&path)
    .unwrap();

    let request = parse(br#"{"filter":{"ACTIVE":true,"AGE":{"$gte":20,"$lt":30}}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::RangeIndex {
            name: "by_active_age".into(),
            field: "AGE".into(),
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &request).unwrap(),
        execute_query(&table, &request).unwrap()
    );

    remove_table_files(&path);
}

#[test]
fn uses_a_compound_index_for_exact_equality() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-compound-equality-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&path);

    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
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

    let request = parse(br#"{"filter":{"NAME":"Alice","AGE":29}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::CompoundEqualityIndex {
            name: "by_name_age".into(),
            fields: vec!["NAME".into(), "AGE".into()],
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &request).unwrap(),
        execute_query(&table, &request).unwrap()
    );

    remove_table_files(&path);
}

#[test]
fn uses_a_compound_index_for_an_equality_prefix() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-compound-equality-prefix-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&path);

    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    for age in 0..32 {
        table
            .insert_record(
                serde_json::json!({
                    "ID": age + 3,
                    "NAME": format!("N{age:02}"),
                    "AGE": age,
                    "ACTIVE": age % 2 == 0
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
        vec![IndexDefinition::named_fields_with_directions(
            "by_active_name",
            vec!["ACTIVE".into(), "NAME".into()],
            vec![-1, 1],
        )],
    )
    .unwrap()
    .save(&path)
    .unwrap();

    let request = parse(br#"{"filter":{"ACTIVE":true,"AGE":{"$ne":0}}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::CompoundEqualityPrefixIndex {
            name: "by_active_name".into(),
            fields: vec!["ACTIVE".into()],
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &request).unwrap(),
        execute_query(&table, &request).unwrap()
    );

    remove_table_files(&path);
}

#[test]
fn chooses_a_compound_sort_index_with_the_smallest_equality_prefix() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-compound-cost-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&path);

    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    table
        .insert_record(
            serde_json::json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 42,
                "ACTIVE": false
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    fs::write(&path, table.to_bytes()).unwrap();
    IndexFile::build(
        &path,
        vec![
            IndexDefinition::named_fields_with_directions(
                "by_name_age",
                vec!["NAME".into(), "AGE".into()],
                vec![1, -1],
            ),
            IndexDefinition::named_fields_with_directions(
                "by_active_name_age",
                vec!["ACTIVE".into(), "NAME".into(), "AGE".into()],
                vec![1, 1, -1],
            ),
        ],
    )
    .unwrap()
    .save(&path)
    .unwrap();

    let request = parse(br#"{"filter":{"ACTIVE":true},"sort":{"NAME":1,"AGE":-1}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::CompoundOrderedIndex {
            name: "by_active_name_age".into(),
            fields: vec!["ACTIVE".into(), "NAME".into(), "AGE".into()],
            directions: vec![1, 1, -1],
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &request).unwrap(),
        execute_query(&table, &request).unwrap()
    );

    remove_table_files(&path);
}

#[test]
fn uses_a_compound_index_for_a_single_sort_key_after_an_equality_prefix() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-single-compound-sort-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&path);

    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    let table = DbfTable::from_bytes(&bytes).unwrap();
    fs::write(&path, bytes).unwrap();
    IndexFile::build(
        &path,
        vec![IndexDefinition::named_fields(
            "by_active_name",
            vec!["ACTIVE".into(), "NAME".into()],
        )],
    )
    .unwrap()
    .save(&path)
    .unwrap();

    let request = parse(br#"{"filter":{"ACTIVE":true},"sort":{"NAME":1}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::CompoundOrderedIndex {
            name: "by_active_name".into(),
            fields: vec!["ACTIVE".into(), "NAME".into()],
            directions: vec![1, 1],
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &request).unwrap(),
        execute_query(&table, &request).unwrap()
    );

    remove_table_files(&path);
}

#[test]
fn chooses_the_access_path_with_fewer_exact_candidates() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-candidate-count-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&path);

    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    for (id, name, age, active) in [
        (3, "Carol", 7, true),
        (4, "Dave", 8, true),
        (5, "Eve", 9, true),
        (6, "Frank", 7, false),
    ] {
        table
            .insert_record(
                serde_json::json!({
                    "ID": id,
                    "NAME": name,
                    "AGE": age,
                    "ACTIVE": active
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
            IndexDefinition::named("by_active", "ACTIVE"),
            IndexDefinition::named_fields(
                "by_age_name_id",
                vec!["AGE".into(), "NAME".into(), "ID".into()],
            ),
        ],
    )
    .unwrap()
    .save(&path)
    .unwrap();

    let request = parse(br#"{"filter":{"ACTIVE":true,"AGE":7},"sort":{"NAME":1,"ID":1}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::CompoundOrderedIndex {
            name: "by_age_name_id".into(),
            fields: vec!["AGE".into(), "NAME".into(), "ID".into()],
            directions: vec![1, 1, 1],
        }
    );
    assert_eq!(
        execute_query_at(&table, &path, &request).unwrap(),
        execute_query(&table, &request).unwrap()
    );

    remove_table_files(&path);
}

#[test]
fn chooses_a_table_scan_for_a_non_selective_index() {
    let path = std::env::temp_dir().join(format!(
        "txbase-query-planner-non-selective-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&path);

    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    bytes[196] = b'T';
    let table = DbfTable::from_bytes(&bytes).unwrap();
    fs::write(&path, bytes).unwrap();
    IndexFile::build(&path, vec![IndexDefinition::named("by_active", "ACTIVE")])
        .unwrap()
        .save(&path)
        .unwrap();

    let request = parse(br#"{"filter":{"ACTIVE":true}}"#).unwrap();
    assert_eq!(
        explain_query_at(&path, &request).unwrap(),
        QueryPlan::TableScan
    );
    assert_eq!(
        execute_query_at(&table, &path, &request).unwrap(),
        execute_query(&table, &request).unwrap()
    );

    remove_table_files(&path);
}

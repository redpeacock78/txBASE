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

use super::*;
use serde_json::Value;
use std::fs;

fn fixture() -> Vec<u8> {
    include_str!("../../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn remove_table_files(path: &std::path::Path) {
    for extension in ["dbf", "dbt", "fpt", "txbase.wal", "txbase.lock"] {
        let candidate = if extension == "dbf" {
            path.to_path_buf()
        } else {
            path.with_extension(extension)
        };
        let _ = fs::remove_file(candidate);
    }
}

#[test]
fn schema_json_describes_fields_and_record_counts() {
    let table = DbfTable::from_bytes(&fixture()).unwrap();
    table.verify().unwrap();

    let schema = table.schema_json();
    assert_eq!(schema["format"], "dbf");
    assert_eq!(schema["record_count"], 2);
    assert_eq!(schema["active_record_count"], 1);
    assert_eq!(schema["fields"][0]["name"], "ID");
    assert_eq!(schema["fields"][0]["type"], "N");
    assert!(
        schema["fields"]
            .as_array()
            .is_some_and(|fields| !fields.is_empty())
    );
}

#[test]
fn copy_table_files_preserves_dbf_and_memo_sidecar() {
    let source = std::env::temp_dir().join(format!(
        "txbase-maintenance-source-{}.dbf",
        std::process::id()
    ));
    let destination = std::env::temp_dir().join(format!(
        "txbase-maintenance-destination-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&source);
    remove_table_files(&destination);

    fs::write(&source, fixture()).unwrap();
    fs::write(source.with_extension("dbt"), b"source memo").unwrap();
    fs::write(&destination, b"old dbf").unwrap();
    fs::write(destination.with_extension("fpt"), b"stale memo").unwrap();

    copy_table_files(&source, &destination).unwrap();

    assert_eq!(fs::read(&destination).unwrap(), fixture());
    assert_eq!(
        fs::read(destination.with_extension("dbt")).unwrap(),
        b"source memo"
    );
    assert!(!destination.with_extension("fpt").exists());
    let restored = DbfTable::from_path(&destination).unwrap();
    assert_eq!(
        restored.active_json(),
        DbfTable::from_bytes(&fixture()).unwrap().active_json()
    );

    let _ = fs::remove_file(source.with_extension("dbt"));
    remove_table_files(&source);
    remove_table_files(&destination);
}

#[test]
fn schema_json_is_valid_json_output() {
    let table = DbfTable::from_bytes(&fixture()).unwrap();
    let encoded = serde_json::to_vec(&table.schema_json()).unwrap();
    let decoded: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded["format"], "dbf");
}

#[test]
fn recall_restores_a_logically_deleted_record() {
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();
    assert!(table.records()[1].deleted);

    table.recall_record(2).unwrap();

    assert!(!table.records()[1].deleted);
    assert_eq!(table.active_json().len(), 2);
    table.verify().unwrap();
}

#[test]
fn pack_removes_deleted_records_and_renumbers_physical_records() {
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();

    table.pack().unwrap();

    assert_eq!(table.header.record_count, 1);
    assert_eq!(table.records().len(), 1);
    assert_eq!(table.records()[0].number, 1);
    assert_eq!(table.active_json().len(), 1);
    let reparsed = DbfTable::from_bytes(&table.to_bytes()).unwrap();
    assert_eq!(reparsed.records().len(), 1);
}

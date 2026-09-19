use super::{DbfTable, schema_export};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn schema_bytes() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {}
    }))
    .unwrap()
}

fn path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "txbase-schema-export-{}-{name}.dbf",
        std::process::id()
    ))
}

fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("txschema.json"));
    let _ = fs::remove_file(schema_export::test_journal_path(path));
    let _ = fs::remove_dir_all(schema_export::test_transaction_directory(path));
    for extension in ["dbt", "DBT", "fpt", "FPT"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
}

#[test]
fn schema_export_commits_dbf_and_schema_together() {
    let destination = path("commit");
    cleanup(&destination);
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();
    table.delete_record(1).unwrap();
    let schema = schema_bytes();

    schema_export::commit_schema_export(&destination, &table, &schema).unwrap();

    let loaded = DbfTable::from_path(&destination).unwrap();
    assert!(loaded.records()[0].deleted);
    assert_eq!(
        fs::read(destination.with_extension("txschema.json")).unwrap(),
        schema
    );
    assert!(!schema_export::test_journal_path(&destination).exists());

    cleanup(&destination);
}

#[test]
fn schema_export_read_recovers_after_dbf_replacement() {
    let destination = path("recovery");
    cleanup(&destination);
    let old_bytes = fixture();
    fs::write(&destination, &old_bytes).unwrap();
    let mut table = DbfTable::from_bytes(&old_bytes).unwrap();
    table.delete_record(1).unwrap();
    let new_bytes = table.to_bytes();
    let schema = schema_bytes();

    let directory = schema_export::test_transaction_directory(&destination);
    fs::create_dir(&directory).unwrap();
    schema_export::test_write_file(&schema_export::test_stage_path(&directory, 0), &new_bytes)
        .unwrap();
    schema_export::test_write_file(&schema_export::test_stage_path(&directory, 1), &schema)
        .unwrap();
    schema_export::test_write_file(&schema_export::test_base_path(&directory, 0), &old_bytes)
        .unwrap();
    schema_export::test_write_journal(&destination, 1).unwrap();
    fs::write(&destination, &new_bytes).unwrap();

    let loaded = DbfTable::from_path(&destination).unwrap();

    assert!(loaded.records()[0].deleted);
    assert_eq!(
        fs::read(destination.with_extension("txschema.json")).unwrap(),
        schema
    );
    assert!(!schema_export::test_journal_path(&destination).exists());

    cleanup(&destination);
}

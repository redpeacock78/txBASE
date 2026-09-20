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
    let _ = fs::remove_file(path.with_extension("txbase.state"));
    let _ = fs::remove_file(crate::index::sidecar_path(path));
    let _ = fs::remove_file(path.with_extension("txbase.lock"));
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
    fs::write(&destination, fixture()).unwrap();
    crate::index::IndexFile::build(
        &destination,
        vec![crate::index::IndexDefinition::for_field("ID")],
    )
    .unwrap()
    .save(&destination)
    .unwrap();
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
    assert!(
        crate::index::IndexFile::load(&destination)
            .unwrap()
            .lookup_eq("ID", &serde_json::json!(1))
            .unwrap()
            .is_empty()
    );
    assert!(!schema_export::test_journal_path(&destination).exists());

    cleanup(&destination);
}

#[test]
fn schema_export_removes_stale_memo_sidecars() {
    let destination = path("memo-cleanup");
    cleanup(&destination);
    fs::write(&destination, fixture()).unwrap();
    for extension in ["dbt", "DBT", "fpt", "FPT"] {
        fs::write(destination.with_extension(extension), b"stale memo").unwrap();
    }
    let table = DbfTable::from_bytes(&fixture()).unwrap();

    schema_export::commit_schema_export(&destination, &table, &schema_bytes()).unwrap();

    for extension in ["dbt", "DBT", "fpt", "FPT"] {
        assert!(!destination.with_extension(extension).exists());
    }
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

#[test]
fn schema_export_read_rejects_an_external_target_change() {
    let destination = path("conflict");
    cleanup(&destination);
    let old_bytes = fixture();
    let mut table = DbfTable::from_bytes(&old_bytes).unwrap();
    table.delete_record(1).unwrap();
    let new_bytes = table.to_bytes();
    let schema = schema_bytes();

    fs::write(&destination, &old_bytes).unwrap();
    let directory = schema_export::test_transaction_directory(&destination);
    fs::create_dir(&directory).unwrap();
    schema_export::test_write_file(&schema_export::test_stage_path(&directory, 0), &new_bytes)
        .unwrap();
    schema_export::test_write_file(&schema_export::test_stage_path(&directory, 1), &schema)
        .unwrap();
    schema_export::test_write_file(&schema_export::test_base_path(&directory, 0), &old_bytes)
        .unwrap();
    schema_export::test_write_journal(&destination, 1).unwrap();
    fs::write(&destination, b"external change").unwrap();

    let error = DbfTable::from_path(&destination).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("XBF schema export target changed during recovery")
    );
    assert!(schema_export::test_journal_path(&destination).exists());
    cleanup(&destination);
}

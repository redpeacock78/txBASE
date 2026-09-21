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
    assert_eq!(loaded.transaction_id(), Some(1));
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
    assert!(!schema_export::test_transaction_directory(&destination).exists());

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
    let old_state = super::persistence::transaction_state_bytes(1).unwrap();
    let new_state = super::persistence::transaction_state_bytes(2).unwrap();

    let directory = schema_export::test_transaction_directory(&destination);
    fs::write(destination.with_extension("txbase.state"), &old_state).unwrap();
    fs::create_dir(&directory).unwrap();
    schema_export::test_write_file(&schema_export::test_stage_path(&directory, 0), &new_bytes)
        .unwrap();
    schema_export::test_write_file(&schema_export::test_stage_path(&directory, 1), &schema)
        .unwrap();
    schema_export::test_write_file(&schema_export::test_base_path(&directory, 0), &old_bytes)
        .unwrap();
    schema_export::test_write_file(&schema_export::test_stage_path(&directory, 6), &new_state)
        .unwrap();
    schema_export::test_write_file(&schema_export::test_base_path(&directory, 6), &old_state)
        .unwrap();
    schema_export::test_write_journal(&destination, 1 | (1 << 6)).unwrap();
    fs::write(&destination, &new_bytes).unwrap();

    let loaded = DbfTable::from_path(&destination).unwrap();

    assert!(loaded.records()[0].deleted);
    assert_eq!(loaded.transaction_id(), Some(2));
    assert_eq!(
        fs::read(destination.with_extension("txschema.json")).unwrap(),
        schema
    );
    assert!(!schema_export::test_journal_path(&destination).exists());

    cleanup(&destination);
}

#[test]
fn schema_export_recovery_replays_index_target() {
    let destination = path("index-recovery");
    cleanup(&destination);
    let old_bytes = fixture();
    fs::write(&destination, &old_bytes).unwrap();
    let definition = crate::index::IndexDefinition::for_field("ID");
    let old_index = crate::index::IndexFile::build(&destination, vec![definition.clone()]).unwrap();
    let old_index_bytes = serde_json::to_vec_pretty(&old_index).unwrap();
    old_index.save(&destination).unwrap();

    let mut table = DbfTable::from_bytes(&old_bytes).unwrap();
    table.delete_record(1).unwrap();
    let new_bytes = table.to_bytes();
    fs::write(&destination, &new_bytes).unwrap();
    let new_index = crate::index::IndexFile::build(&destination, vec![definition]).unwrap();
    let new_index_bytes = serde_json::to_vec_pretty(&new_index).unwrap();
    fs::write(&destination, &old_bytes).unwrap();
    fs::write(crate::index::sidecar_path(&destination), &old_index_bytes).unwrap();

    let schema = schema_bytes();
    let old_state = super::persistence::transaction_state_bytes(1).unwrap();
    let new_state = super::persistence::transaction_state_bytes(2).unwrap();
    let directory = schema_export::test_transaction_directory(&destination);
    fs::write(destination.with_extension("txbase.state"), &old_state).unwrap();
    fs::create_dir(&directory).unwrap();
    schema_export::test_write_file(&schema_export::test_stage_path(&directory, 0), &new_bytes)
        .unwrap();
    schema_export::test_write_file(&schema_export::test_stage_path(&directory, 1), &schema)
        .unwrap();
    schema_export::test_write_file(&schema_export::test_stage_path(&directory, 6), &new_state)
        .unwrap();
    schema_export::test_write_file(
        &schema_export::test_stage_path(&directory, 7),
        &new_index_bytes,
    )
    .unwrap();
    schema_export::test_write_file(&schema_export::test_base_path(&directory, 0), &old_bytes)
        .unwrap();
    schema_export::test_write_file(&schema_export::test_base_path(&directory, 6), &old_state)
        .unwrap();
    schema_export::test_write_file(
        &schema_export::test_base_path(&directory, 7),
        &old_index_bytes,
    )
    .unwrap();
    schema_export::test_write_journal(&destination, 1 | (1 << 6) | (1 << 7)).unwrap();
    fs::write(&destination, &new_bytes).unwrap();

    let loaded = DbfTable::from_path(&destination).unwrap();

    assert!(loaded.records()[0].deleted);
    assert_eq!(loaded.transaction_id(), Some(2));
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

#[test]
fn schema_export_rejects_malformed_journal_records() {
    let cases = [
        (
            "invalid-header",
            vec![b'b', b'a', b'd'],
            "header is invalid",
        ),
        (
            "unsupported-version",
            vec![b'T', b'X', b'S', b'E', 2, 0, 0, 0],
            "unsupported XBF schema export journal version",
        ),
        (
            "unknown-flags",
            vec![b'T', b'X', b'S', b'E', 1, 0, 0, 1],
            "XBF schema export journal contains unknown flags",
        ),
    ];

    for (name, record, message) in cases {
        let destination = path(name);
        cleanup(&destination);
        schema_export::test_write_journal_record(&destination, &record).unwrap();

        let error = schema_export::recover_schema_export_locked(&destination).unwrap_err();
        assert!(error.to_string().contains(message));

        cleanup(&destination);
    }
}

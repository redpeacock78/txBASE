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
    for extension in [
        "dbf",
        "dbt",
        "fpt",
        "txidx",
        "txbase.wal",
        "txbase.lock",
        "txbase.state",
        "txbase.mvcc",
        "txbase.cdc",
        "txschema.json",
        "txbase.xbf-export.wal",
    ] {
        let candidate = if extension == "dbf" {
            path.to_path_buf()
        } else {
            path.with_extension(extension)
        };
        let _ = fs::remove_file(candidate);
    }
    let mut export_directory = path.as_os_str().to_os_string();
    export_directory.push(".txbase-xbf-export");
    let _ = fs::remove_dir_all(export_directory);
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
fn copy_table_files_preserves_schema_and_replaces_a_stale_destination_schema() {
    let source = std::env::temp_dir().join(format!(
        "txbase-maintenance-schema-source-{}.dbf",
        std::process::id()
    ));
    let destination = std::env::temp_dir().join(format!(
        "txbase-maintenance-schema-destination-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&source);
    remove_table_files(&destination);

    let schema = serde_json::to_vec(&serde_json::json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {
            "ID": {"primary": true},
            "NAME": {"not_null": true}
        }
    }))
    .unwrap();
    fs::write(&source, fixture()).unwrap();
    fs::write(source.with_extension("txschema.json"), &schema).unwrap();
    fs::write(&destination, fixture()).unwrap();
    fs::write(destination.with_extension("txschema.json"), b"stale schema").unwrap();

    copy_table_files(&source, &destination).unwrap();

    assert_eq!(
        fs::read(destination.with_extension("txschema.json")).unwrap(),
        schema
    );
    let loaded = DbfTable::from_path(&destination).unwrap();
    assert_eq!(
        loaded.schema_json()["schema_metadata"]["fields"]["ID"]["primary"],
        true
    );

    remove_table_files(&source);
    remove_table_files(&destination);
}

#[test]
fn copy_table_files_recovers_a_pending_destination_wal_before_replacement() {
    let source = std::env::temp_dir().join(format!(
        "txbase-maintenance-destination-wal-source-{}.dbf",
        std::process::id()
    ));
    let destination = std::env::temp_dir().join(format!(
        "txbase-maintenance-destination-wal-destination-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&source);
    remove_table_files(&destination);

    let original = fixture();
    fs::write(&source, &original).unwrap();
    fs::write(&destination, &original).unwrap();
    let mut pending = DbfTable::from_bytes(&original).unwrap();
    pending
        .patch_record(
            1,
            serde_json::json!({"NAME": "stale destination"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let mut payload = SNAPSHOT_MAGIC.to_vec();
    payload.extend_from_slice(&pending.to_bytes());
    let wal_path = destination.with_extension("txbase.wal");
    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    copy_table_files(&source, &destination).unwrap();

    let restored = DbfTable::from_path(&destination).unwrap();
    assert_eq!(restored.to_bytes(), original);
    assert!(!wal_path.exists());

    remove_table_files(&source);
    remove_table_files(&destination);
}

#[test]
fn copy_table_files_preserves_transaction_state() {
    let source = std::env::temp_dir().join(format!(
        "txbase-maintenance-state-source-{}.dbf",
        std::process::id()
    ));
    let destination = std::env::temp_dir().join(format!(
        "txbase-maintenance-state-destination-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&source);
    remove_table_files(&destination);

    fs::write(&source, fixture()).unwrap();
    let mut table = DbfTable::from_path(&source).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
        )
        .unwrap();
    table.save_with_wal(&source).unwrap();

    copy_table_files(&source, &destination).unwrap();

    assert_eq!(
        DbfTable::from_path(&destination).unwrap().transaction_id(),
        Some(1)
    );
    assert_eq!(DbfTable::mvcc_versions(&destination).unwrap(), vec![1]);

    remove_table_files(&source);
    remove_table_files(&destination);
}

#[test]
fn copy_table_files_preserves_change_events() {
    let source = std::env::temp_dir().join(format!(
        "txbase-maintenance-cdc-source-{}.dbf",
        std::process::id()
    ));
    let destination = std::env::temp_dir().join(format!(
        "txbase-maintenance-cdc-destination-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&source);
    remove_table_files(&destination);

    fs::write(&source, fixture()).unwrap();
    let mut table = DbfTable::from_path(&source).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
        )
        .unwrap();
    table.save_with_wal(&source).unwrap();

    copy_table_files(&source, &destination).unwrap();

    assert_eq!(
        DbfTable::cdc_events(&destination, None)
            .unwrap()
            .iter()
            .map(|event| event.transaction_id)
            .collect::<Vec<_>>(),
        vec![1]
    );

    remove_table_files(&source);
    remove_table_files(&destination);
}

#[test]
fn copy_table_files_preserves_a_valid_index_sidecar() {
    let source = std::env::temp_dir().join(format!(
        "txbase-maintenance-index-source-{}.dbf",
        std::process::id()
    ));
    let destination = std::env::temp_dir().join(format!(
        "txbase-maintenance-index-destination-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&source);
    remove_table_files(&destination);

    fs::write(&source, fixture()).unwrap();
    crate::index::IndexFile::build(
        &source,
        vec![crate::index::IndexDefinition::for_field("NAME")],
    )
    .unwrap()
    .save(&source)
    .unwrap();

    copy_table_files(&source, &destination).unwrap();

    let restored = crate::index::IndexFile::load(&destination).unwrap();
    assert_eq!(restored.index_names(), vec!["NAME"]);

    remove_table_files(&source);
    remove_table_files(&destination);
}

#[test]
fn copy_table_files_rejects_a_stale_source_index() {
    let source = std::env::temp_dir().join(format!(
        "txbase-maintenance-stale-index-source-{}.dbf",
        std::process::id()
    ));
    let destination = std::env::temp_dir().join(format!(
        "txbase-maintenance-stale-index-destination-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&source);
    remove_table_files(&destination);

    fs::write(&source, fixture()).unwrap();
    crate::index::IndexFile::build(
        &source,
        vec![crate::index::IndexDefinition::for_field("NAME")],
    )
    .unwrap()
    .save(&source)
    .unwrap();
    let mut changed = DbfTable::from_path(&source).unwrap();
    changed
        .patch_record(
            1,
            serde_json::json!({"NAME": "Changed"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    fs::write(&source, changed.to_bytes()).unwrap();

    let destination_before = fixture();
    fs::write(&destination, &destination_before).unwrap();
    let error = copy_table_files(&source, &destination).unwrap_err();
    assert!(error.to_string().contains("source index sidecar"));
    assert_eq!(fs::read(&destination).unwrap(), destination_before);

    remove_table_files(&source);
    remove_table_files(&destination);
}

#[test]
fn copy_table_files_removes_an_old_destination_index_when_source_has_none() {
    let source = std::env::temp_dir().join(format!(
        "txbase-maintenance-no-index-source-{}.dbf",
        std::process::id()
    ));
    let destination = std::env::temp_dir().join(format!(
        "txbase-maintenance-no-index-destination-{}.dbf",
        std::process::id()
    ));
    remove_table_files(&source);
    remove_table_files(&destination);

    fs::write(&source, fixture()).unwrap();
    fs::write(&destination, fixture()).unwrap();
    crate::index::IndexFile::build(
        &destination,
        vec![crate::index::IndexDefinition::for_field("NAME")],
    )
    .unwrap()
    .save(&destination)
    .unwrap();

    copy_table_files(&source, &destination).unwrap();

    assert!(!crate::index::sidecar_path(&destination).exists());

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

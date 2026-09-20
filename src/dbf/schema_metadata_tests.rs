use super::*;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_SCHEMA_ID: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn temporary_path() -> std::path::PathBuf {
    let id = NEXT_SCHEMA_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "txbase-schema-metadata-{}-{id}.dbf",
        std::process::id()
    ))
}

fn cleanup(path: &Path) {
    for extension in ["dbf", "txschema.json", "txbase.wal", "txbase.lock"] {
        let candidate = if extension == "dbf" {
            path.to_path_buf()
        } else {
            path.with_extension(extension)
        };
        let _ = fs::remove_file(candidate);
    }
}

fn metadata() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {
            "ID": {"primary": true},
            "NAME": {"unique": true, "not_null": true}
        }
    }))
    .unwrap()
}

fn check_metadata() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {},
        "checks": [{"AGE": {"$gte": 0}}]
    }))
    .unwrap()
}

fn default_metadata() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {
            "NAME": {"not_null": true, "default": "Unknown"}
        }
    }))
    .unwrap()
}

fn encoding_metadata(name: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "format": "txbase-schema",
        "version": 1,
        "encoding": name,
        "fields": {}
    }))
    .unwrap()
}

#[test]
fn loads_schema_metadata_and_enforces_local_constraints() {
    let path = temporary_path();
    cleanup(&path);
    fs::write(&path, fixture()).unwrap();
    fs::write(path.with_extension("txschema.json"), metadata()).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.schema_json()["schema_metadata"]["version"], 1);
    assert_eq!(
        table.schema_json()["schema_metadata"]["fields"]["ID"]["primary"],
        true
    );

    let duplicate = table
        .insert_record(
            serde_json::json!({"ID": 1, "NAME": "Carol"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(
        duplicate
            .to_string()
            .contains("duplicate value for field ID")
    );

    let missing_name = table
        .insert_record(serde_json::json!({"ID": 3}).as_object().unwrap().clone())
        .unwrap_err();
    assert!(
        missing_name
            .to_string()
            .contains("field NAME must not be null")
    );

    cleanup(&path);
}

#[test]
fn schema_checks_reject_invalid_candidates() {
    let path = temporary_path();
    cleanup(&path);
    fs::write(&path, fixture()).unwrap();
    fs::write(path.with_extension("txschema.json"), check_metadata()).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    let invalid = table
        .insert_record(
            serde_json::json!({"ID": 3, "NAME": "Carol", "AGE": -1})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(
        invalid
            .to_string()
            .contains("constraint violation: check 0 failed")
    );

    let id = table
        .insert_record(
            serde_json::json!({"ID": 3, "NAME": "Carol", "AGE": 30})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(id, 3);
    cleanup(&path);
}

#[test]
fn schema_defaults_fill_missing_insert_fields() {
    let path = temporary_path();
    cleanup(&path);
    fs::write(&path, fixture()).unwrap();
    fs::write(path.with_extension("txschema.json"), default_metadata()).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    let id = table
        .insert_record(
            serde_json::json!({"ID": 3, "AGE": 30})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(table.active_record(id).unwrap().values["NAME"], "Unknown");

    let explicit_null = table
        .insert_record(
            serde_json::json!({"ID": 4, "NAME": null, "AGE": 30})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(
        explicit_null
            .to_string()
            .contains("field NAME must not be null")
    );
    cleanup(&path);
}

#[test]
fn rejects_existing_rows_that_break_schema_metadata() {
    let path = temporary_path();
    cleanup(&path);
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();
    table
        .insert_record(
            serde_json::json!({"ID": 1, "NAME": "Carol"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    fs::write(&path, table.to_bytes()).unwrap();
    fs::write(path.with_extension("txschema.json"), metadata()).unwrap();

    let error = DbfTable::from_path(&path).unwrap_err();
    assert!(error.to_string().contains("duplicate value for field ID"));

    cleanup(&path);
}

#[test]
fn copies_and_removes_schema_sidecars_with_the_table() {
    let source = temporary_path();
    let destination = source.with_file_name(format!(
        "txbase-schema-metadata-destination-{}.dbf",
        std::process::id()
    ));
    cleanup(&source);
    cleanup(&destination);
    fs::write(&source, fixture()).unwrap();
    fs::write(source.with_extension("txschema.json"), metadata()).unwrap();

    copy_table_files(&source, &destination).unwrap();
    assert_eq!(
        fs::read(destination.with_extension("txschema.json")).unwrap(),
        metadata()
    );
    assert!(DbfTable::from_path(&destination).is_ok());

    fs::remove_file(source.with_extension("txschema.json")).unwrap();
    copy_table_files(&source, &destination).unwrap();
    assert!(!destination.with_extension("txschema.json").exists());

    cleanup(&source);
    cleanup(&destination);
}

#[test]
fn rejects_a_schema_sidecar_changed_after_load() {
    let path = temporary_path();
    cleanup(&path);
    fs::write(&path, fixture()).unwrap();
    let schema_path = path.with_extension("txschema.json");
    fs::write(&schema_path, metadata()).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "Alicia"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    fs::write(
        &schema_path,
        serde_json::json!({
            "format": "txbase-schema",
            "version": 1,
            "fields": {"ID": {"primary": true}}
        })
        .to_string(),
    )
    .unwrap();

    let error = table.save_with_wal(&path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("schema metadata changed since the table was loaded")
    );

    cleanup(&path);
}

#[test]
fn applies_a_supported_encoding_override_to_reads_and_writes() {
    let path = temporary_path();
    cleanup(&path);
    let mut bytes = fixture();
    bytes[29] = 0x00;
    fs::write(&path, bytes).unwrap();
    fs::write(
        path.with_extension("txschema.json"),
        encoding_metadata("gbk"),
    )
    .unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.schema_json()["encoding_override"], "GBK/CP936");
    assert_eq!(
        table.schema_json()["encoding_metadata"]["source"],
        "explicit-override"
    );
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "中文"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reloaded = DbfTable::from_path(&path).unwrap();
    assert_eq!(reloaded.active_record(1).unwrap().values["NAME"], "中文");

    cleanup(&path);
}

#[test]
fn applies_a_path_encoding_override_without_persisting_it() {
    let path = temporary_path();
    cleanup(&path);
    let mut bytes = fixture();
    bytes[29] = 0x00;
    fs::write(&path, bytes).unwrap();
    let schema_path = path.with_extension("txschema.json");
    let schema_bytes = encoding_metadata("big5");
    fs::write(&schema_path, &schema_bytes).unwrap();

    let mut table = DbfTable::from_path_with_encoding(&path, Some("gbk")).unwrap();
    assert_eq!(table.schema_json()["encoding_override"], "GBK/CP936");
    assert_eq!(
        table.schema_json()["encoding_metadata"]["source"],
        "explicit-override"
    );
    assert_eq!(
        table.schema_json()["schema_metadata"]["encoding"],
        "Big5/CP950"
    );
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "中文"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    assert_eq!(fs::read(&path).unwrap()[29], 0x00);
    assert_eq!(fs::read(schema_path).unwrap(), schema_bytes);
    let reloaded = DbfTable::from_path_with_encoding(&path, Some("GBK/CP936")).unwrap();
    assert_eq!(reloaded.active_record(1).unwrap().values["NAME"], "中文");
    cleanup(&path);
}

#[test]
fn accepts_strict_shift_jis_as_an_explicit_override() {
    let path = temporary_path();
    cleanup(&path);
    fs::write(&path, fixture()).unwrap();
    fs::write(
        path.with_extension("txschema.json"),
        encoding_metadata("shift_jis"),
    )
    .unwrap();

    let table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.schema_json()["encoding_override"], "Shift_JIS");

    cleanup(&path);
}

#[test]
fn accepts_iso_2022_jp_as_an_explicit_override() {
    let path = temporary_path();
    cleanup(&path);
    fs::write(&path, fixture()).unwrap();
    fs::write(
        path.with_extension("txschema.json"),
        encoding_metadata("iso2022-jp"),
    )
    .unwrap();

    let table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.schema_json()["encoding_override"], "ISO-2022-JP");

    cleanup(&path);
}

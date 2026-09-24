use super::schema_metadata_tests::{cleanup, encoding_metadata, fixture, temporary_path};
use super::*;
use crate::transaction::{FileWal, Wal};
use crate::xbase::{OperationIr, OperationMethod};
use std::fs;

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
fn recovers_an_operation_with_a_path_encoding_override() {
    let path = temporary_path();
    cleanup(&path);
    let mut bytes = fixture();
    bytes[29] = 0x00;
    fs::write(&path, bytes).unwrap();
    fs::write(
        path.with_extension("txschema.json"),
        encoding_metadata("big5"),
    )
    .unwrap();

    let operation = OperationIr {
        method: OperationMethod::Patch,
        path: "/records/1".into(),
        body: Some(serde_json::json!({"NAME": "中文"})),
    };
    let wal_path = path.with_extension("txbase.wal");
    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&operation_payload(&operation).unwrap()).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path_with_encoding(&path, Some("gbk")).unwrap();

    assert_eq!(recovered.active_record(1).unwrap().values["NAME"], "中文");
    assert_eq!(recovered.schema_json()["encoding_override"], "GBK/CP936");
    assert!(!wal_path.exists());
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

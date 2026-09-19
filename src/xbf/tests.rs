use super::{
    XbfField, XbfLimits, XbfRecord, XbfTable, XbfType, XbfValue, decode, decode_with_limits,
    encode, encode_with_limits, read_path, recover_path, write_path,
};
use crate::transaction::{FileWal, Wal};
use std::fs;
use std::path::PathBuf;

fn fixture() -> XbfTable {
    XbfTable {
        generation: 7,
        fields: vec![
            XbfField {
                name: "BOOL".into(),
                ty: XbfType::Boolean,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "I32".into(),
                ty: XbfType::Signed32,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "I64".into(),
                ty: XbfType::Signed64,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "U64".into(),
                ty: XbfType::Unsigned64,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "F32".into(),
                ty: XbfType::Float32,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "F64".into(),
                ty: XbfType::Float64,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "TEXT".into(),
                ty: XbfType::String,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "BYTES".into(),
                ty: XbfType::Bytes,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "DATE".into(),
                ty: XbfType::Date,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "STAMP".into(),
                ty: XbfType::Timestamp,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "UUID".into(),
                ty: XbfType::Uuid,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "DOC".into(),
                ty: XbfType::Json,
                nullable: true,
                primary_key: false,
                unique: false,
            },
        ],
        records: vec![XbfRecord {
            deleted: true,
            values: vec![
                XbfValue::Boolean(true),
                XbfValue::Signed32(-32),
                XbfValue::Signed64(-64),
                XbfValue::Unsigned64(u64::MAX),
                XbfValue::Float32(1.25),
                XbfValue::Float64(-2.5),
                XbfValue::String("日本語".into()),
                XbfValue::Bytes(vec![0, 1, 255]),
                XbfValue::Date(20_000),
                XbfValue::Timestamp(-123_456),
                XbfValue::Uuid([7; 16]),
                XbfValue::Json(serde_json::json!({"ok": true})),
            ],
        }],
    }
}

#[test]
fn converts_a_dbf_fixture_to_xbf_without_dropping_records() {
    let bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect::<Vec<_>>();
    let dbf = crate::dbf::DbfTable::from_bytes(&bytes).unwrap();

    let xbf = super::from_dbf(&dbf).unwrap();

    assert_eq!(xbf.generation, 0);
    assert_eq!(xbf.fields[0].name, "ID");
    assert_eq!(xbf.fields[0].ty, XbfType::Signed64);
    assert_eq!(xbf.fields[1].name, "NAME");
    assert_eq!(xbf.fields[1].ty, XbfType::String);
    assert_eq!(xbf.fields[2].name, "AGE");
    assert_eq!(xbf.fields[2].ty, XbfType::Signed64);
    assert_eq!(xbf.records.len(), dbf.records().len());
    assert_eq!(xbf.records[0].deleted, dbf.records()[0].deleted);
    assert!(matches!(xbf.records[0].values[1], XbfValue::String(_)));
}

#[test]
fn exports_a_representable_xbf_table_to_dbf() {
    let table = XbfTable {
        generation: 3,
        fields: vec![
            XbfField {
                name: "ID".into(),
                ty: XbfType::Signed64,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "NAME".into(),
                ty: XbfType::String,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "ACTIVE".into(),
                ty: XbfType::Boolean,
                nullable: false,
                primary_key: false,
                unique: false,
            },
        ],
        records: vec![
            XbfRecord {
                deleted: false,
                values: vec![
                    XbfValue::Signed64(1),
                    XbfValue::String("Alice".into()),
                    XbfValue::Boolean(true),
                ],
            },
            XbfRecord {
                deleted: true,
                values: vec![
                    XbfValue::Signed64(2),
                    XbfValue::String("Bob".into()),
                    XbfValue::Boolean(false),
                ],
            },
        ],
    };

    let dbf = super::to_dbf(&table).unwrap();
    let round_trip = crate::dbf::DbfTable::from_bytes(&dbf.to_bytes()).unwrap();

    assert_eq!(round_trip.records().len(), 2);
    assert_eq!(
        round_trip.records()[0].values["NAME"],
        serde_json::json!("Alice")
    );
    assert!(!round_trip.records()[0].deleted);
    assert!(round_trip.records()[1].deleted);
}

#[test]
fn rejects_nonrepresentable_xbf_dbf_export_types() {
    let table = XbfTable {
        generation: 0,
        fields: vec![XbfField {
            name: "DOC".into(),
            ty: XbfType::Json,
            nullable: false,
            primary_key: false,
            unique: false,
        }],
        records: vec![XbfRecord {
            deleted: false,
            values: vec![XbfValue::Json(serde_json::json!({"ok": true}))],
        }],
    };

    assert!(super::to_dbf(&table).is_err());
}

#[test]
fn round_trips_a_fixture_with_every_v1_value_type() {
    let table = fixture();
    let bytes = encode(&table).unwrap();
    let decoded = decode(&bytes).unwrap();

    assert_eq!(decoded, table);
}

#[test]
fn preserves_nullable_and_deleted_records() {
    let mut table = fixture();
    table.records.push(XbfRecord {
        deleted: false,
        values: vec![
            XbfValue::Boolean(false),
            XbfValue::Signed32(1),
            XbfValue::Signed32(2),
            XbfValue::Unsigned64(3),
            XbfValue::Float32(4.0),
            XbfValue::Float64(5.0),
            XbfValue::String("second".into()),
            XbfValue::Bytes(Vec::new()),
            XbfValue::Date(0),
            XbfValue::Timestamp(0),
            XbfValue::Uuid([0; 16]),
            XbfValue::Null,
        ],
    });

    assert_eq!(decode(&encode(&table).unwrap()).unwrap(), table);
}

#[test]
fn rejects_header_and_section_corruption() {
    let bytes = encode(&fixture()).unwrap();

    let mut header_corruption = bytes.clone();
    header_corruption[8] ^= 1;
    assert!(decode(&header_corruption).is_err());

    let mut data_corruption = bytes;
    let last = data_corruption.len() - 1;
    data_corruption[last] ^= 1;
    assert!(decode(&data_corruption).is_err());
}

#[test]
fn enforces_explicit_size_limits_before_decoding() {
    let bytes = encode(&fixture()).unwrap();
    let limits = XbfLimits {
        max_file_size: bytes.len() - 1,
        ..XbfLimits::default()
    };
    assert!(decode_with_limits(&bytes, &limits).is_err());

    let limits = XbfLimits {
        max_record_size: 1,
        ..XbfLimits::default()
    };
    assert!(decode_with_limits(&bytes, &limits).is_err());
}

#[test]
fn rejects_duplicate_unique_values_and_non_nullable_nulls() {
    let table = XbfTable {
        generation: 1,
        fields: vec![XbfField {
            name: "ID".into(),
            ty: XbfType::Signed64,
            nullable: false,
            primary_key: true,
            unique: true,
        }],
        records: vec![
            XbfRecord {
                deleted: false,
                values: vec![XbfValue::Signed64(1)],
            },
            XbfRecord {
                deleted: false,
                values: vec![XbfValue::Signed32(1)],
            },
        ],
    };
    assert!(encode(&table).is_err());

    let table = XbfTable {
        generation: 1,
        fields: vec![XbfField {
            name: "ID".into(),
            ty: XbfType::Signed64,
            nullable: false,
            primary_key: false,
            unique: false,
        }],
        records: vec![XbfRecord {
            deleted: false,
            values: vec![XbfValue::Null],
        }],
    };
    assert!(encode_with_limits(&table, &XbfLimits::default()).is_err());
}

#[test]
fn writes_and_reads_a_durable_snapshot_path() {
    let path = snapshot_test_path("durable");
    let _ = fs::remove_file(&path);
    let table = fixture();

    write_path(&path, &table).unwrap();
    assert_eq!(read_path(&path).unwrap(), table);
    write_path(&path, &table).unwrap();
    assert_eq!(read_path(&path).unwrap(), table);

    fs::remove_file(&path).unwrap();
}

#[test]
fn recovers_a_generation_checked_full_snapshot_wal() {
    let path = snapshot_test_path("recovery");
    let wal_path = path.with_extension("xwl");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let base = fixture();
    let mut target = fixture();
    target.generation = base.generation + 1;
    write_path(&path, &base).unwrap();

    let snapshot = encode(&target).unwrap();
    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&super::wal::encode_record(base.generation, target.generation, &snapshot).unwrap())
        .unwrap();
    wal.sync().unwrap();
    drop(wal);

    assert!(recover_path(&path).unwrap());
    assert_eq!(read_path(&path).unwrap(), target);
    assert!(!wal_path.exists());
    assert!(!recover_path(&path).unwrap());

    fs::remove_file(&path).unwrap();
}

#[test]
fn rejects_a_generation_mismatched_xbf_wal() {
    let path = snapshot_test_path("mismatch");
    let wal_path = path.with_extension("xwl");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let base = fixture();
    let mut target = fixture();
    target.generation = base.generation + 1;
    write_path(&path, &base).unwrap();

    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(
        &super::wal::encode_record(
            base.generation - 1,
            target.generation,
            &encode(&target).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    wal.sync().unwrap();
    drop(wal);

    assert!(recover_path(&path).is_err());
    assert!(wal_path.exists());
    fs::remove_file(&path).unwrap();
    fs::remove_file(&wal_path).unwrap();
}

#[test]
fn rejects_xbf_wal_records_over_the_file_wal_limit() {
    assert!(super::wal::validate_record_size(crate::transaction::MAX_WAL_RECORD_SIZE + 1).is_err());
    assert!(super::wal::validate_record_size(crate::transaction::MAX_WAL_RECORD_SIZE).is_ok());
}

fn snapshot_test_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "txbase-xbf-snapshot-{}-{name}.xbf",
        std::process::id(),
    ))
}

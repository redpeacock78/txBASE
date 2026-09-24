use super::{
    XbfField, XbfLimits, XbfRecord, XbfTable, XbfType, XbfValue, decode, decode_with_limits,
    encode, encode_with_limits, read_path, recover_path, save_with_wal, write_path,
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

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
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
fn enforces_explicit_size_limits_for_encoding_and_decoding() {
    let table = fixture();
    let bytes = encode(&table).unwrap();
    let mut two_records = table.clone();
    two_records.records.push(table.records[0].clone());
    let two_record_bytes = encode(&two_records).unwrap();
    let cases = [
        (
            XbfLimits {
                max_file_size: bytes.len() - 1,
                ..XbfLimits::default()
            },
            &bytes,
            &table,
        ),
        (
            XbfLimits {
                max_section_size: 1,
                ..XbfLimits::default()
            },
            &bytes,
            &table,
        ),
        (
            XbfLimits {
                max_record_size: 1,
                ..XbfLimits::default()
            },
            &bytes,
            &table,
        ),
        (
            XbfLimits {
                max_value_size: 1,
                ..XbfLimits::default()
            },
            &bytes,
            &table,
        ),
        (
            XbfLimits {
                max_field_name: 1,
                ..XbfLimits::default()
            },
            &bytes,
            &table,
        ),
        (
            XbfLimits {
                max_fields: 1,
                ..XbfLimits::default()
            },
            &bytes,
            &table,
        ),
        (
            XbfLimits {
                max_records: 1,
                ..XbfLimits::default()
            },
            &two_record_bytes,
            &two_records,
        ),
    ];
    for (limits, bytes, table) in cases {
        assert!(
            decode_with_limits(bytes, &limits).is_err(),
            "decoder accepted bytes over an explicit limit"
        );
        assert!(
            encode_with_limits(table, &limits).is_err(),
            "encoder accepted a table over an explicit limit"
        );
    }
}

#[test]
fn rejects_file_and_section_limits_while_building_record_data() {
    let mut table = fixture();
    table.records = vec![table.records[0].clone(); 64];
    let encoded = encode(&table).unwrap();
    let directory_offset = get_u64(&encoded, 36) as usize;
    let data_offset = get_u64(&encoded, 56) as usize;
    let first_record_length = get_u64(&encoded, directory_offset + 8) as usize;
    let data_length = get_u64(&encoded, 64) as usize;
    let schema_length = get_u64(&encoded, 24) as usize;
    let directory_length = get_u64(&encoded, 44) as usize;

    let file_limited = XbfLimits {
        max_file_size: data_offset + first_record_length,
        ..XbfLimits::default()
    };
    let error = encode_with_limits(&table, &file_limited).unwrap_err();
    assert!(error.to_string().contains("encoded XBF file exceeds"));

    assert!(data_length > schema_length.max(directory_length));
    let section_limited = XbfLimits {
        max_section_size: data_length - 1,
        ..XbfLimits::default()
    };
    let error = encode_with_limits(&table, &section_limited).unwrap_err();
    assert!(error.to_string().contains("record data section exceeds"));
}

#[test]
fn rejects_invalid_limits_before_reading_or_writing() {
    let default = XbfLimits::default();
    let cases = [
        XbfLimits {
            max_file_size: super::HEADER_SIZE - 1,
            ..default
        },
        XbfLimits {
            max_section_size: 0,
            ..default
        },
        XbfLimits {
            max_record_size: 0,
            ..default
        },
        XbfLimits {
            max_value_size: 0,
            ..default
        },
        XbfLimits {
            max_field_name: 0,
            ..default
        },
        XbfLimits {
            max_fields: 0,
            ..default
        },
        XbfLimits {
            max_records: 0,
            ..default
        },
    ];
    for limits in cases {
        assert!(decode_with_limits(&[], &limits).is_err());
        assert!(encode_with_limits(&fixture(), &limits).is_err());
    }
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
fn saves_a_newer_snapshot_through_the_xbf_wal_boundary() {
    let path = snapshot_test_path("save-with-wal");
    let wal_path = path.with_extension("xwl");
    let lock_path = path.with_extension("txbase.lock");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&lock_path);
    let base = fixture();
    let mut target = fixture();
    target.generation = base.generation + 1;
    write_path(&path, &base).unwrap();

    save_with_wal(&path, &target).unwrap();

    assert_eq!(read_path(&path).unwrap(), target);
    assert!(!wal_path.exists());
    fs::remove_file(&path).unwrap();
    fs::remove_file(&lock_path).unwrap();
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
fn reading_a_snapshot_recovers_a_pending_wal() {
    let path = snapshot_test_path("automatic-recovery");
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
            base.generation,
            target.generation,
            &encode(&target).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    wal.sync().unwrap();
    drop(wal);

    assert_eq!(read_path(&path).unwrap(), target);
    assert!(!wal_path.exists());

    fs::remove_file(&path).unwrap();
}

#[test]
fn direct_snapshot_write_recovers_a_pending_wal_before_replacing_it() {
    let path = snapshot_test_path("direct-write-recovery");
    let wal_path = path.with_extension("xwl");
    let lock_path = path.with_extension("txbase.lock");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&lock_path);
    let base = fixture();
    let mut pending = fixture();
    pending.generation = base.generation + 1;
    let mut replacement = fixture();
    replacement.generation = base.generation + 2;
    write_path(&path, &base).unwrap();

    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(
        &super::wal::encode_record(
            base.generation,
            pending.generation,
            &encode(&pending).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    wal.sync().unwrap();
    drop(wal);

    write_path(&path, &replacement).unwrap();

    assert_eq!(read_path(&path).unwrap(), replacement);
    assert!(!wal_path.exists());
    fs::remove_file(&path).unwrap();
    fs::remove_file(&lock_path).unwrap();
}

#[test]
fn direct_snapshot_write_preserves_a_generation_conflict() {
    let path = snapshot_test_path("direct-write-conflict");
    let wal_path = path.with_extension("xwl");
    let lock_path = path.with_extension("txbase.lock");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&lock_path);
    let base = fixture();
    let mut pending = fixture();
    pending.generation = base.generation + 1;
    let mut replacement = fixture();
    replacement.generation = base.generation + 2;
    write_path(&path, &base).unwrap();

    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(
        &super::wal::encode_record(
            base.generation - 1,
            pending.generation,
            &encode(&pending).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    wal.sync().unwrap();
    drop(wal);

    assert!(write_path(&path, &replacement).is_err());
    assert_eq!(read_path_without_recovery(&path), base);
    assert!(wal_path.exists());
    fs::remove_file(&path).unwrap();
    fs::remove_file(&wal_path).unwrap();
    fs::remove_file(&lock_path).unwrap();
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

fn read_path_without_recovery(path: &std::path::Path) -> XbfTable {
    super::persistence::read_path_without_recovery_with_limits(path, &XbfLimits::default()).unwrap()
}

fn snapshot_test_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "txbase-xbf-snapshot-{}-{name}.xbf",
        std::process::id(),
    ))
}

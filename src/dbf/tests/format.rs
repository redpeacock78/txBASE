use super::super::codec::foxpro_datetime_bytes;
use super::super::*;
use super::fixture;
use crate::xbase::{OperationIr, OperationMethod};

#[test]
fn reads_header_fields_and_active_records() {
    let table = DbfTable::from_bytes(&fixture()).unwrap();

    assert_eq!(table.header.record_count, 2);
    assert_eq!(table.header.record_length, 18);
    assert_eq!(table.fields[0].name, "ID");
    assert_eq!(table.fields[1].offset, 4);
    assert!(table.records()[1].deleted);
    assert_eq!(table.active_json().len(), 1);
    assert_eq!(table.active_json()[0]["NAME"], "Alice");
    assert_eq!(table.active_json()[0]["AGE"], 29);
}

#[test]
fn rejects_truncated_records() {
    let mut bytes = fixture();
    bytes.pop();
    bytes.pop();
    assert!(matches!(
        DbfTable::from_bytes(&bytes),
        Err(DbfError::Invalid(message)) if message == "record area is truncated"
    ));
}

#[test]
fn rejects_malformed_memo_snapshots() {
    assert!(decode_snapshot(MEMO_SNAPSHOT_MAGIC).is_err());

    let mut unknown_format = MEMO_SNAPSHOT_MAGIC.to_vec();
    unknown_format.push(0xff);
    unknown_format.extend_from_slice(&[0; 16]);
    assert!(decode_snapshot(&unknown_format).is_err());

    let mut mismatched_length = MEMO_SNAPSHOT_MAGIC.to_vec();
    mismatched_length.push(MemoFormat::Dbase3.tag());
    mismatched_length.extend_from_slice(&1u64.to_le_bytes());
    mismatched_length.extend_from_slice(&1u64.to_le_bytes());
    mismatched_length.push(0);
    assert!(decode_snapshot(&mismatched_length).is_err());
}

#[test]
fn applies_byte_deltas_idempotently_and_rejects_wrong_base() {
    let delta = ByteDelta::new(b"base", b"target").unwrap();

    assert_eq!(apply_byte_delta(b"base", &delta, "DBF").unwrap(), b"target");
    assert_eq!(
        apply_byte_delta(b"target", &delta, "DBF").unwrap(),
        b"target"
    );
    assert!(apply_byte_delta(b"other", &delta, "DBF").is_err());
}

#[test]
fn operation_wal_payload_round_trips() {
    let operation = OperationIr {
        method: OperationMethod::Patch,
        path: "/records/1".into(),
        body: Some(serde_json::json!({"$inc": {"AGE": 1}})),
    };
    let payload = operation_payload(&operation).unwrap();
    assert!(payload.starts_with(OPERATION_MAGIC));
    assert_eq!(decode_operation_payload(&payload).unwrap(), Some(operation));
    assert_eq!(
        decode_operation_payload(SNAPSHOT_MAGIC.as_ref()).unwrap(),
        None
    );
}

#[test]
fn rejects_unknown_deletion_markers() {
    let mut bytes = fixture();
    bytes[161] = b'!';
    assert!(matches!(
        DbfTable::from_bytes(&bytes),
        Err(DbfError::Invalid(message)) if message.contains("unknown deletion marker")
    ));
}

#[test]
fn reads_level7_descriptor_layout() {
    let mut bytes = vec![0; 122];
    bytes[0] = 0x04;
    bytes[4] = 1;
    bytes[8] = 117;
    bytes[10] = 4;
    bytes[68] = b'V';
    bytes[100] = b'C';
    bytes[101] = 3;
    bytes[116] = FIELD_TERMINATOR;
    bytes[117..121].copy_from_slice(b" yes");
    bytes[121] = 0x1a;

    let table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.fields[0].name, "V");
    assert_eq!(table.active_json()[0]["V"], "yes");
}

#[test]
fn decodes_float_fields_as_numbers() {
    assert_eq!(decode_field(b'F', b" 1.5", 0, None), serde_json::json!(1.5));
}

#[test]
fn round_trips_visual_foxpro_double_fields() {
    let field = FieldDescriptor {
        name: "AMOUNT".into(),
        field_type: b'B',
        length: 8,
        decimal_count: 0,
        flags: 0,
        offset: 1,
    };
    let value = serde_json::json!(12.5);
    let encoded = encode_field(&field, &value, 0).unwrap();

    assert_eq!(encoded, 12.5f64.to_le_bytes());
    assert_eq!(decode_field(b'B', &encoded, 0, None), value);
}

#[test]
fn round_trips_visual_foxpro_currency_fields() {
    let field = FieldDescriptor {
        name: "PRICE".into(),
        field_type: b'Y',
        length: 8,
        decimal_count: 4,
        flags: 0,
        offset: 1,
    };
    let raw = (-12_345_678i64).to_le_bytes();

    assert_eq!(
        decode_field(b'Y', &raw, 0, None),
        serde_json::json!("-1234.5678")
    );
    assert_eq!(
        encode_field(&field, &serde_json::json!("-1234.5678"), 0).unwrap(),
        raw.to_vec()
    );
    assert_eq!(
        encode_field(&field, &Value::Null, 0).unwrap(),
        0i64.to_le_bytes().to_vec()
    );
    assert!(encode_field(&field, &serde_json::json!("1.23456"), 0).is_err());
}

#[test]
fn validates_date_field_encoding() {
    let field = FieldDescriptor {
        name: "BORN".into(),
        field_type: b'D',
        length: 8,
        decimal_count: 0,
        flags: 0,
        offset: 1,
    };

    assert_eq!(
        encode_field(&field, &serde_json::json!("20260918"), 0).unwrap(),
        b"20260918"
    );
    assert_eq!(encode_field(&field, &Value::Null, 0).unwrap(), b"        ");
    assert_eq!(decode_field(b'D', b"        ", 0, None), Value::Null);
    assert_eq!(decode_field(b'D', &[0; 8], 0, None), Value::Null);
    assert!(encode_field(&field, &serde_json::json!("2026-09-18"), 0).is_err());
}

#[test]
fn validates_numeric_field_encoding() {
    let field = FieldDescriptor {
        name: "AMOUNT".into(),
        field_type: b'N',
        length: 8,
        decimal_count: 2,
        flags: 0,
        offset: 1,
    };

    assert_eq!(
        encode_field(&field, &serde_json::json!("1.25"), 0).unwrap(),
        b"    1.25"
    );
    assert!(encode_field(&field, &serde_json::json!("not-a-number"), 0).is_err());
    assert!(encode_field(&field, &Value::Bool(true), 0).is_err());
}

#[test]
fn round_trips_hex_timestamp_fields() {
    let field = FieldDescriptor {
        name: "STAMP".into(),
        field_type: b'T',
        length: 8,
        decimal_count: 0,
        flags: 0,
        offset: 1,
    };
    let raw = [0x00, 0x01, 0x1a, 0x7f, 0x80, 0xfe, 0xff, 0x42];
    let value = decode_field(b'T', &raw, 0, None);

    assert_eq!(value, serde_json::json!("00011a7f80feff42"));
    assert_eq!(encode_field(&field, &value, 0).unwrap(), raw);
}

#[test]
fn round_trips_visual_foxpro_datetime_fields() {
    let field = FieldDescriptor {
        name: "STAMP".into(),
        field_type: b'T',
        length: 8,
        decimal_count: 0,
        flags: 0,
        offset: 1,
    };
    let value = "2026-09-18T12:34:56";
    let raw = foxpro_datetime_bytes(value).unwrap().unwrap();

    assert_eq!(u32::from_le_bytes(raw[..4].try_into().unwrap()), 2_461_302);
    assert_eq!(u32::from_le_bytes(raw[4..].try_into().unwrap()), 45_296_000);
    assert_eq!(
        decode_field(b'T', &raw, 0, Some(MemoFormat::FoxPro)),
        serde_json::json!(value)
    );
    assert_eq!(
        encode_field(&field, &serde_json::json!(value), 0).unwrap(),
        raw.to_vec()
    );
    assert_eq!(
        decode_field(b'T', &[0; 8], 0, Some(MemoFormat::FoxPro)),
        Value::Null
    );
    assert_eq!(encode_field(&field, &Value::Null, 0).unwrap(), vec![0; 8]);
    assert!(encode_field(&field, &serde_json::json!("2026-02-30T00:00:00"), 0).is_err());
}

#[test]
fn encodes_memo_pointers_in_format_byte_order() {
    let field = FieldDescriptor {
        name: "MEMO".into(),
        field_type: b'M',
        length: 4,
        decimal_count: 0,
        flags: 0,
        offset: 1,
    };
    let block = 0x0102_0304;

    assert_eq!(
        encode_memo_pointer(&field, block, MemoFormat::Dbase4).unwrap(),
        block.to_le_bytes()
    );
    assert_eq!(
        encode_memo_pointer(&field, block, MemoFormat::FoxPro).unwrap(),
        block.to_le_bytes()
    );
    assert_eq!(
        memo_index(&block.to_le_bytes(), MemoFormat::Dbase4).unwrap(),
        Some(block)
    );
    assert_eq!(
        memo_index(&block.to_le_bytes(), MemoFormat::FoxPro).unwrap(),
        Some(block)
    );
    assert_eq!(memo_index(b"    ", MemoFormat::FoxPro).unwrap(), None);
    assert_eq!(
        decode_field(b'M', &block.to_le_bytes(), 0, Some(MemoFormat::FoxPro)),
        serde_json::json!(block)
    );
    for version in [0x30, 0x31, 0x32] {
        assert_eq!(
            decode_field(
                b'M',
                &block.to_le_bytes(),
                0,
                memo_format_for_version(version)
            ),
            serde_json::json!(block)
        );
    }
}

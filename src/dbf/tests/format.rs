use super::super::codec::foxpro_datetime_bytes;
use super::super::*;
use super::{fixture, foxpro_variable_fixture};
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
fn round_trips_visual_foxpro_variable_fields() {
    let mut table = DbfTable::from_bytes(&foxpro_variable_fixture()).unwrap();

    assert_eq!(
        table.active_json(),
        vec![serde_json::json!({"NAME": "abc", "_PAYLOAD": "00ff"})]
    );
    assert!(
        !table.active_json()[0]
            .as_object()
            .unwrap()
            .contains_key("_NullFlags")
    );

    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "xy", "_PAYLOAD": "a1b2"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let bytes = table.to_bytes();
    assert_eq!(&bytes[130..135], b"xy  \x02");
    assert_eq!(&bytes[135..140], &[0xa1, 0xb2, 0, 0, 2]);
    assert_eq!(bytes[140], 0x05);

    table
        .patch_record(
            1,
            serde_json::json!({"NAME": null, "_PAYLOAD": null})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(
        table.active_json(),
        vec![serde_json::json!({"NAME": null, "_PAYLOAD": null})]
    );
    assert_eq!(&table.to_bytes()[130..140], &[0; 10]);
    assert_eq!(table.to_bytes()[140], 0x0f);

    assert_eq!(
        table
            .insert_record(
                serde_json::json!({"NAME": "new", "_PAYLOAD": "cafe"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap(),
        2
    );
    let second = 129 + 12;
    let bytes = table.to_bytes();
    assert_eq!(&bytes[second + 1..second + 6], b"new \x03");
    assert_eq!(&bytes[second + 6..second + 11], &[0xca, 0xfe, 0, 0, 2]);
    assert_eq!(bytes[second + 11], 0x05);
}

#[test]
fn reads_visual_foxpro_nullable_fixed_fields() {
    for version in [0x30, 0x31] {
        let mut bytes = vec![0; 101];
        bytes[0] = version;
        bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
        bytes[8..10].copy_from_slice(&97u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&3u16.to_le_bytes());
        bytes[32..36].copy_from_slice(b"NAME");
        bytes[43] = b'C';
        bytes[48] = 1;
        bytes[50] = 0x02;
        bytes[64..74].copy_from_slice(b"_NullFlags");
        bytes[80] = 1;
        bytes[82] = 0x01;
        bytes[96] = FIELD_TERMINATOR;
        bytes[97] = ACTIVE_RECORD;
        bytes[98] = b'A';
        bytes[99] = 0x01;
        bytes[100] = EOF_MARKER;

        assert_eq!(
            DbfTable::from_bytes(&bytes).unwrap().active_json(),
            vec![serde_json::json!({"NAME": null})]
        );
    }
}

#[test]
fn round_trips_visual_foxpro_binary_character_fields() {
    let mut bytes = vec![0; 71];
    bytes[0] = 0x32;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&65u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[32..35].copy_from_slice(b"RAW");
    bytes[43] = b'C';
    bytes[48] = 4;
    bytes[50] = 0x04;
    bytes[64] = FIELD_TERMINATOR;
    bytes[65] = ACTIVE_RECORD;
    bytes[66..70].copy_from_slice(&[0, 0xff, 0, 0]);
    bytes[70] = EOF_MARKER;

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_json()[0]["RAW"], "00ff0000");
    table
        .patch_record(
            1,
            serde_json::json!({"RAW": "a1b2"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(&table.to_bytes()[66..70], &[0xa1, 0xb2, 0, 0]);
}

#[test]
fn assigns_level7_auto_increment_values_on_insert() {
    let mut bytes = vec![0; 123];
    bytes[0] = 0x04;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&117u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[68..72].copy_from_slice(b"AUTO");
    bytes[100] = b'+';
    bytes[101] = 4;
    bytes[108..112].copy_from_slice(&7u32.to_le_bytes());
    bytes[116] = FIELD_TERMINATOR;
    bytes[117] = ACTIVE_RECORD;
    bytes[122] = EOF_MARKER;

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.insert_record(Map::new()).unwrap(), 2);
    assert_eq!(table.active_record(2).unwrap().values["AUTO"], 7);
    assert_eq!(
        u32::from_le_bytes(table.to_bytes()[108..112].try_into().unwrap()),
        8
    );

    let before = table.to_bytes();
    let error = table
        .insert_record(serde_json::json!({"AUTO": 99}).as_object().unwrap().clone())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("auto-increment field AUTO is read-only")
    );
    assert_eq!(table.to_bytes(), before);

    table.replace_record(2, Map::new()).unwrap();
    assert_eq!(table.active_record(2).unwrap().values["AUTO"], 7);
    let error = table
        .patch_record(
            2,
            serde_json::json!({"AUTO": 8}).as_object().unwrap().clone(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("auto-increment field AUTO is read-only")
    );
    assert_eq!(table.active_record(2).unwrap().values["AUTO"], 7);
}

#[test]
fn assigns_visual_foxpro_auto_increment_values_on_insert() {
    let mut bytes = vec![0; 71];
    bytes[0] = 0x31;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&65u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[32..36].copy_from_slice(b"AUTO");
    bytes[43] = b'I';
    bytes[48] = 4;
    bytes[50] = 0x0c;
    bytes[51..55].copy_from_slice(&7i32.to_le_bytes());
    bytes[55] = 3;
    bytes[64] = FIELD_TERMINATOR;
    bytes[65] = ACTIVE_RECORD;
    bytes[66..70].copy_from_slice(&4i32.to_le_bytes());
    bytes[70] = EOF_MARKER;

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["AUTO"], 4);
    assert_eq!(table.insert_record(Map::new()).unwrap(), 2);
    assert_eq!(table.active_record(2).unwrap().values["AUTO"], 10);
    assert_eq!(
        i32::from_le_bytes(table.to_bytes()[51..55].try_into().unwrap()),
        10
    );

    let before = table.to_bytes();
    let error = table
        .insert_record(serde_json::json!({"AUTO": 99}).as_object().unwrap().clone())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("auto-increment field AUTO is read-only")
    );
    assert_eq!(table.to_bytes(), before);
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
        block.to_be_bytes()
    );
    assert_eq!(
        memo_index(&block.to_le_bytes(), MemoFormat::Dbase4).unwrap(),
        Some(block)
    );
    assert_eq!(
        memo_index(&block.to_be_bytes(), MemoFormat::FoxPro).unwrap(),
        Some(block)
    );
    assert_eq!(memo_index(b"    ", MemoFormat::FoxPro).unwrap(), None);
    assert_eq!(
        decode_field(b'M', &block.to_be_bytes(), 0, Some(MemoFormat::FoxPro)),
        serde_json::json!(block)
    );
    for version in [0x30, 0x31, 0x32] {
        assert_eq!(
            decode_field(
                b'M',
                &block.to_be_bytes(),
                0,
                memo_format_for_version(version)
            ),
            serde_json::json!(block)
        );
    }
}

#[test]
fn decodes_and_encodes_windows_1252_character_fields() {
    let mut bytes = fixture();
    bytes[29] = 0x03;
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let name_start = record_start + 4;
    bytes[name_start..name_start + 10].fill(b' ');
    bytes[name_start] = 0xe9;

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "é");

    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "€"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(table.to_bytes()[name_start], 0x80);
    assert_eq!(
        DbfTable::from_bytes(&table.to_bytes())
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["NAME"],
        "€"
    );

    let error = table
        .patch_record(
            1,
            serde_json::json!({"NAME": "漢"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("outside Windows-1252"));
}

#[test]
fn decodes_and_encodes_windows_character_fields() {
    let record_start = usize::from(u16::from_le_bytes([fixture()[8], fixture()[9]]));
    let name_start = record_start + 4;
    for (language_driver, raw, value, code_page) in [
        (0xc8, 0x8a, "Š", "Windows-1250"),
        (0xc9, 0xdf, "Я", "Windows-1251"),
        (0xca, 0xdd, "İ", "Windows-1254"),
        (0xcb, 0xd9, "Ω", "Windows-1253"),
        (0x7d, 0xe9, "י", "Windows-1255"),
        (0x7e, 0xc7, "ا", "Windows-1256"),
    ] {
        let mut bytes = fixture();
        bytes[29] = language_driver;
        bytes[name_start..name_start + 10].fill(b' ');
        bytes[name_start] = raw;
        let mut table = DbfTable::from_bytes(&bytes).unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], value);

        table
            .patch_record(
                1,
                serde_json::json!({"NAME": value})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        assert_eq!(table.to_bytes()[name_start], raw);
        assert_eq!(
            DbfTable::from_bytes(&table.to_bytes())
                .unwrap()
                .active_record(1)
                .unwrap()
                .values["NAME"],
            value
        );

        let error = table
            .patch_record(
                1,
                serde_json::json!({"NAME": "漢"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap_err();
        assert!(error.to_string().contains(&format!("outside {code_page}")));
    }
}

#[test]
fn decodes_and_encodes_oem_character_fields() {
    let mut bytes = fixture();
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let name_start = record_start + 4;

    bytes[29] = 0x01;
    bytes[name_start..name_start + 10].fill(b' ');
    bytes[name_start] = 0x82;
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "é");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "é"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(table.to_bytes()[name_start], 0x82);

    bytes[29] = 0x02;
    bytes[name_start..name_start + 10].fill(b' ');
    bytes[name_start] = 0x9b;
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "ø");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "ø"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(table.to_bytes()[name_start], 0x9b);

    bytes[29] = 0x64;
    bytes[name_start..name_start + 10].fill(b' ');
    bytes[name_start] = 0x88;
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "ł");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "ł"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(table.to_bytes()[name_start], 0x88);

    bytes[29] = 0x65;
    bytes[name_start..name_start + 10].fill(b' ');
    bytes[name_start] = 0x9f;
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "Я");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "Я"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(table.to_bytes()[name_start], 0x9f);

    let error = table
        .patch_record(
            1,
            serde_json::json!({"NAME": "漢"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("outside CP866"));
}

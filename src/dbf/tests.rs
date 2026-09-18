use super::codec::foxpro_datetime_bytes;
use super::*;

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn foxpro_variable_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 142];
    bytes[0] = 0x32;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&129u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&12u16.to_le_bytes());

    bytes[32..36].copy_from_slice(b"NAME");
    bytes[43] = b'V';
    bytes[48] = 5;
    bytes[50] = 0x02;

    bytes[64..72].copy_from_slice(b"_PAYLOAD");
    bytes[75] = b'Q';
    bytes[80] = 5;
    bytes[82] = 0x02;

    bytes[96..106].copy_from_slice(b"_NullFlags");
    bytes[107] = 0;
    bytes[112] = 1;
    bytes[114] = 1;

    bytes[128] = FIELD_TERMINATOR;
    bytes[129] = ACTIVE_RECORD;
    bytes[130..133].copy_from_slice(b"abc");
    bytes[133] = b' ';
    bytes[134] = 3;
    bytes[135..137].copy_from_slice(&[0, 0xff]);
    bytes[139] = 2;
    bytes[140] = 0x05;
    bytes[141] = EOF_MARKER;
    bytes
}

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

#[test]
fn mutates_records_and_round_trips_to_dbf() {
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();
    let inserted = table
        .insert_record(
            serde_json::json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 42,
                "ACTIVE": false
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    assert_eq!(inserted, 3);

    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "Alicia"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table
        .replace_record(
            3,
            serde_json::json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 43,
                "ACTIVE": true
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    table.delete_record(1).unwrap();

    let round_trip = DbfTable::from_bytes(&table.to_bytes()).unwrap();
    assert_eq!(round_trip.header.record_count, 3);
    assert!(round_trip.records()[0].deleted);
    assert_eq!(round_trip.active_record(3).unwrap().values["NAME"], "Carol");
    assert_eq!(round_trip.active_record(3).unwrap().values["AGE"], 43);
}

#[test]
fn rejects_unknown_mutation_fields_without_changing_table() {
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();
    let before = table.to_bytes();
    let error = table
        .patch_record(
            1,
            serde_json::json!({"UNKNOWN": true})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();

    assert!(error.to_string().contains("unknown field UNKNOWN"));
    assert_eq!(table.to_bytes(), before);
}

#[test]
fn rejects_nonempty_sidecar_mutations_without_sidecar() {
    let mut bytes = fixture();
    bytes[64 + 11] = b'M';
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    let before = table.to_bytes();

    let error = table
        .patch_record(
            1,
            serde_json::json!({"NAME": "new memo"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("memo sidecar is missing for non-empty field NAME")
    );
    assert_eq!(table.to_bytes(), before);

    let error = table
        .insert_record(
            serde_json::json!({
                "ID": 3,
                "NAME": "new memo",
                "AGE": 42,
                "ACTIVE": false
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("memo sidecar is missing for non-empty field NAME")
    );
    assert_eq!(table.to_bytes(), before);
}

#[test]
fn preserves_unresolved_sidecar_pointers_on_other_mutations() {
    let mut bytes = fixture();
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let name_start = record_start + 4;
    bytes[name_start..name_start + 10].fill(0xff);

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
        )
        .unwrap();

    assert_eq!(
        &table.to_bytes()[name_start..name_start + 10],
        &bytes[name_start..name_start + 10]
    );
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);
}

#[test]
fn preserves_opaque_fields_on_other_mutations() {
    let mut bytes = fixture();
    bytes[64 + 11] = b'Z';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let field_start = record_start + 4;
    let raw = bytes[field_start..field_start + 10].to_vec();
    let mut table = DbfTable::from_bytes(&bytes).unwrap();

    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
        )
        .unwrap();

    assert_eq!(
        &table.to_bytes()[field_start..field_start + 10],
        raw.as_slice()
    );
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);
}

#[test]
fn applies_update_operators_without_ambiguous_writes() {
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({
                "$set": {"NAME": "Alicia"},
                "$inc": {"AGE": 1},
                "$unset": {"ACTIVE": true}
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "Alicia");
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);
    assert_eq!(
        table.active_record(1).unwrap().values["ACTIVE"],
        Value::Null
    );

    let before = table.to_bytes();
    let error = table
        .patch_record(
            1,
            serde_json::json!({"$set": {"AGE": 31}, "$inc": {"AGE": 1}})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("multiple update operators"));
    assert_eq!(table.to_bytes(), before);

    let error = table
        .patch_record(
            1,
            serde_json::json!({"$unknown": {"AGE": 31}})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("unsupported update operator"));
}

#[test]
fn reads_and_writes_dbase3_memo_sidecar() {
    let path = std::env::temp_dir().join(format!("txbase-dbase3-memo-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("dbt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = fixture();
    bytes[0] = 0x83;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let memo_start = record_start + 4;
    bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    let text = b"memo from dbt";
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + text.len()].copy_from_slice(text);
    memo[DBT_BLOCK_SIZE + text.len()] = EOF_MARKER;
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        table.active_record(1).unwrap().values["NAME"],
        "memo from dbt"
    );
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "memo from dbt"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "new memo"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["NAME"], "new memo");
    assert_eq!(reread.active_record(1).unwrap().values["AGE"], 30);
    assert_eq!(
        &reread.to_bytes()[memo_start..memo_start + 10],
        b"         2"
    );
    let memo = MemoFile::open(&memo_path, 0x83).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), b"new memo");
    assert_eq!(u32::from_be_bytes(memo.bytes[..4].try_into().unwrap()), 3);

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_dbase3_binary_sidecar() {
    let path =
        std::env::temp_dir().join(format!("txbase-dbase3-binary-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("dbt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = fixture();
    bytes[0] = 0x83;
    bytes[64 + 11] = b'B';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let binary_start = record_start + 4;
    bytes[binary_start..binary_start + 10].copy_from_slice(b"         1");
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    let binary = [0x00, 0x1a, 0xff, 0x7f];
    memo[..4].copy_from_slice(&2u32.to_be_bytes());
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + binary.len()].copy_from_slice(&binary);
    memo[DBT_BLOCK_SIZE + binary.len()..DBT_BLOCK_SIZE + binary.len() + 2]
        .copy_from_slice(&[EOF_MARKER, EOF_MARKER]);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "001aff7f");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "deadbeef"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let mut reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["NAME"], "deadbeef");
    assert_eq!(
        &reread.to_bytes()[binary_start..binary_start + 10],
        b"         2"
    );
    let memo = MemoFile::open(&memo_path, 0x83).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(u32::from_be_bytes(memo.bytes[..4].try_into().unwrap()), 3);

    let error = reread
        .patch_record(
            1,
            serde_json::json!({"NAME": "001a1aff"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("dBASE III binary data cannot contain its 0x1a1a terminator")
    );
    assert_eq!(reread.active_record(1).unwrap().values["NAME"], "deadbeef");

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_dbase4_memo_sidecar() {
    let path = std::env::temp_dir().join(format!("txbase-dbase4-memo-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("dbt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = fixture();
    bytes[0] = 0x8b;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let memo_start = record_start + 4;
    bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    let text = b"memo from dbase4";
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&[0xff, 0xff, 0x08, 0x00]);
    memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
        .copy_from_slice(&((text.len() as u32 + 8).to_le_bytes()));
    memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + text.len()].copy_from_slice(text);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        table.active_record(1).unwrap().values["NAME"],
        "memo from dbase4"
    );
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "changed dbase4"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        reread.active_record(1).unwrap().values["NAME"],
        "changed dbase4"
    );
    assert_eq!(
        &reread.to_bytes()[memo_start..memo_start + 10],
        b"         2"
    );
    let memo = MemoFile::open(&memo_path, 0x8b).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), b"changed dbase4");
    assert_eq!(
        &memo.bytes[DBT_BLOCK_SIZE * 2..DBT_BLOCK_SIZE * 2 + 4],
        &[0xff, 0xff, 0x08, 0x00]
    );

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_dbase4_binary_sidecar() {
    let path =
        std::env::temp_dir().join(format!("txbase-dbase4-binary-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("DBT");
    let lowercase_memo_path = path.with_extension("dbt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);
    let _ = fs::remove_file(&lowercase_memo_path);

    let mut bytes = fixture();
    bytes[0] = 0x8b;
    bytes[64 + 11] = b'B';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let binary_start = record_start + 4;
    bytes[binary_start..binary_start + 10].copy_from_slice(b"         1");
    fs::write(&path, bytes).unwrap();

    let dbt_block_size = 1024;
    let mut memo = vec![0; dbt_block_size * 2];
    memo[20..22].copy_from_slice(&(dbt_block_size as u16).to_le_bytes());
    let binary = [0x00, 0x1a, 0xff, 0x7f];
    memo[dbt_block_size..dbt_block_size + 4].copy_from_slice(&[0xff, 0xff, 0x08, 0x00]);
    memo[dbt_block_size + 4..dbt_block_size + 8]
        .copy_from_slice(&((binary.len() as u32 + 8).to_le_bytes()));
    memo[dbt_block_size + 8..dbt_block_size + 8 + binary.len()].copy_from_slice(&binary);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "001aff7f");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "deadbeef"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["NAME"], "deadbeef");
    assert_eq!(
        &reread.to_bytes()[binary_start..binary_start + 10],
        b"         2"
    );
    let memo = MemoFile::open(&memo_path, 0x8b).unwrap();
    assert_eq!(memo.block_size, dbt_block_size);
    assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(u32::from_le_bytes(memo.bytes[..4].try_into().unwrap()), 3);
    let sidecar_count = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate.file_stem() == path.file_stem()
                && candidate
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("dbt"))
        })
        .count();
    assert_eq!(sidecar_count, 1);

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_foxpro_fpt_memo_sidecar() {
    let path = std::env::temp_dir().join(format!("txbase-foxpro-memo-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("fpt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = fixture();
    bytes[0] = 0xf5;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let memo_start = record_start + 4;
    bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[6..8].copy_from_slice(&(DBT_BLOCK_SIZE as u16).to_be_bytes());
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&1u32.to_be_bytes());
    let text = b"memo from fpt";
    memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
        .copy_from_slice(&(text.len() as u32).to_be_bytes());
    memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + text.len()].copy_from_slice(text);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        table.active_record(1).unwrap().values["NAME"],
        "memo from fpt"
    );
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "changed fpt"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        reread.active_record(1).unwrap().values["NAME"],
        "changed fpt"
    );
    assert_eq!(
        &reread.to_bytes()[memo_start..memo_start + 10],
        b"         2"
    );
    let memo = MemoFile::open(&memo_path, 0xf5).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), b"changed fpt");

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_foxpro_binary_memo_sidecar_as_hex() {
    let path = std::env::temp_dir().join(format!(
        "txbase-foxpro-binary-memo-{}.dbf",
        std::process::id()
    ));
    let memo_path = path.with_extension("fpt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = vec![0; 71];
    bytes[0] = 0xf5;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&65u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[32..37].copy_from_slice(b"BMEMO");
    bytes[43] = b'M';
    bytes[48] = 4;
    bytes[50] = 0x04;
    bytes[64] = FIELD_TERMINATOR;
    bytes[65] = ACTIVE_RECORD;
    let pointer_start = 66;
    bytes[pointer_start..pointer_start + 4].copy_from_slice(&1u32.to_be_bytes());
    bytes[70] = EOF_MARKER;
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[6..8].copy_from_slice(&(DBT_BLOCK_SIZE as u16).to_be_bytes());
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&0u32.to_be_bytes());
    let binary = [0x10, 0x20, 0xf0];
    memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
        .copy_from_slice(&(binary.len() as u32).to_be_bytes());
    memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + binary.len()].copy_from_slice(&binary);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["BMEMO"], "1020f0");
    table
        .patch_record(
            1,
            serde_json::json!({"BMEMO": "deadbeef"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["BMEMO"], "deadbeef");
    assert_eq!(
        &reread.to_bytes()[pointer_start..pointer_start + 4],
        &2u32.to_be_bytes()
    );
    let memo = MemoFile::open(&memo_path, 0xf5).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn preserves_visual_foxpro_null_sidecar_values() {
    let mut bytes = vec![0; 104];
    bytes[0] = 0x30;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&97u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&6u16.to_le_bytes());
    bytes[32..37].copy_from_slice(b"BMEMO");
    bytes[43] = b'M';
    bytes[48] = 4;
    bytes[50] = 0x06;
    bytes[64..74].copy_from_slice(b"_NullFlags");
    bytes[80] = 1;
    bytes[82] = 0x01;
    bytes[96] = FIELD_TERMINATOR;
    bytes[97] = ACTIVE_RECORD;
    bytes[98..102].copy_from_slice(&1u32.to_be_bytes());
    bytes[102] = 0x01;
    bytes[103] = EOF_MARKER;

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[512..516].copy_from_slice(&0u32.to_be_bytes());
    memo[516..520].copy_from_slice(&3u32.to_be_bytes());
    memo[520..523].copy_from_slice(&[0x10, 0x20, 0xf0]);

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    table
        .resolve_memos(&MemoFile {
            bytes: memo,
            block_size: DBT_BLOCK_SIZE,
            format: MemoFormat::FoxPro,
        })
        .unwrap();
    assert_eq!(table.active_record(1).unwrap().values["BMEMO"], Value::Null);
}

#[test]
fn reads_and_writes_visual_foxpro_picture_fpt_sidecar_as_hex() {
    let path =
        std::env::temp_dir().join(format!("txbase-foxpro-binary-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("fpt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = vec![0; 71];
    bytes[0] = 0x30;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&65u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[32..37].copy_from_slice(b"IMAGE");
    bytes[43] = b'P';
    bytes[48] = 4;
    bytes[64] = FIELD_TERMINATOR;
    bytes[65] = ACTIVE_RECORD;
    let pointer_start = 66;
    bytes[pointer_start..pointer_start + 4].copy_from_slice(&1u32.to_be_bytes());
    bytes[70] = EOF_MARKER;
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[6..8].copy_from_slice(&(DBT_BLOCK_SIZE as u16).to_be_bytes());
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&0u32.to_be_bytes());
    let binary = [0x00, 0x01, 0xff, 0x7f];
    memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
        .copy_from_slice(&(binary.len() as u32).to_be_bytes());
    memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + binary.len()].copy_from_slice(&binary);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["IMAGE"], "0001ff7f");
    table
        .patch_record(
            1,
            serde_json::json!({"IMAGE": "deadbeef"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["IMAGE"], "deadbeef");
    assert_eq!(
        &reread.to_bytes()[pointer_start..pointer_start + 4],
        &2u32.to_be_bytes()
    );
    let memo = MemoFile::open(&memo_path, 0x30).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_visual_foxpro_blob_fpt_sidecar_as_hex() {
    let path = std::env::temp_dir().join(format!("txbase-foxpro-blob-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("fpt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = vec![0; 71];
    bytes[0] = 0x32;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&65u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[32..36].copy_from_slice(b"BLOB");
    bytes[43] = b'W';
    bytes[48] = 4;
    bytes[64] = FIELD_TERMINATOR;
    bytes[65] = ACTIVE_RECORD;
    let pointer_start = 66;
    bytes[pointer_start..pointer_start + 4].copy_from_slice(&1u32.to_be_bytes());
    bytes[70] = EOF_MARKER;
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[6..8].copy_from_slice(&(DBT_BLOCK_SIZE as u16).to_be_bytes());
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&0u32.to_be_bytes());
    let binary = [0x00, 0x01, 0xff, 0x7f];
    memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
        .copy_from_slice(&(binary.len() as u32).to_be_bytes());
    memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + binary.len()].copy_from_slice(&binary);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["BLOB"], "0001ff7f");
    table
        .patch_record(
            1,
            serde_json::json!({"BLOB": "deadbeef"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["BLOB"], "deadbeef");
    assert_eq!(
        &reread.to_bytes()[pointer_start..pointer_start + 4],
        &2u32.to_be_bytes()
    );
    let memo = MemoFile::open(&memo_path, 0x32).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn rejects_stale_dbf_before_save() {
    let path = std::env::temp_dir().join(format!("txbase-stale-save-{}.dbf", std::process::id()));
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);

    let original = fixture();
    fs::write(&path, &original).unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 31}).as_object().unwrap().clone(),
        )
        .unwrap();

    let mut external = original;
    let record_start = usize::from(u16::from_le_bytes([external[8], external[9]]));
    external[record_start + 14..record_start + 17].copy_from_slice(b" 30");
    fs::write(&path, &external).unwrap();

    let error = table.save_with_wal(&path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("DBF changed since the table was loaded")
    );
    assert_eq!(fs::read(&path).unwrap(), external);
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
}

#[test]
fn rejects_stale_memo_sidecar_before_save() {
    let path = std::env::temp_dir().join(format!("txbase-stale-memo-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("dbt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = fixture();
    bytes[0] = 0x83;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    bytes[record_start + 4..record_start + 14].copy_from_slice(b"         1");
    fs::write(&path, &bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(b"memo");
    memo[DBT_BLOCK_SIZE + 4] = EOF_MARKER;
    fs::write(&memo_path, &memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 31}).as_object().unwrap().clone(),
        )
        .unwrap();
    memo[DBT_BLOCK_SIZE] = b'X';
    fs::write(&memo_path, memo).unwrap();

    let error = table.save_with_wal(&path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("memo sidecar changed since the table was loaded")
    );

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn recovers_latest_snapshot_from_wal_before_reading() {
    let path = std::env::temp_dir().join(format!(
        "txbase-dbf-recovery-{}-{}.dbf",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);

    let mut pending = DbfTable::from_bytes(&fixture()).unwrap();
    pending
        .insert_record(
            serde_json::json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 42,
                "ACTIVE": true
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    fs::write(&path, fixture()).unwrap();

    let mut wal = FileWal::open(&wal_path).unwrap();
    let mut payload = SNAPSHOT_MAGIC.to_vec();
    payload.extend_from_slice(&pending.to_bytes());
    wal.append(&payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(recovered.active_record(3).unwrap().values["NAME"], "Carol");
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
}

#[test]
fn recovers_dbf_delta_from_wal_before_reading() {
    let path = std::env::temp_dir().join(format!(
        "txbase-dbf-delta-recovery-{}-{}.dbf",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);

    fs::write(&path, fixture()).unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "Delta"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let full_payload = snapshot_payload(&table.to_bytes());
    let payload = delta_payload(&table, &path, None, full_payload.len())
        .unwrap()
        .expect("path-loaded mutation should fit in a delta");
    assert!(payload.starts_with(DELTA_MAGIC));
    assert!(payload.len() < full_payload.len());

    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(recovered.active_record(1).unwrap().values["NAME"], "Delta");
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
}

#[test]
fn recovers_dbf_and_memo_from_txdm_snapshot() {
    let path = std::env::temp_dir().join(format!(
        "txbase-dbf-memo-recovery-{}-{}.dbf",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let memo_path = path.with_extension("dbt");
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);
    let _ = fs::remove_file(&wal_path);

    let mut bytes = fixture();
    bytes[0] = 0x83;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let memo_start = record_start + 4;
    bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
    fs::write(&path, &bytes).unwrap();

    let mut memo_bytes = vec![0; DBT_BLOCK_SIZE * 2];
    let text = b"memo before crash";
    memo_bytes[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + text.len()].copy_from_slice(text);
    memo_bytes[DBT_BLOCK_SIZE + text.len()] = EOF_MARKER;
    fs::write(&memo_path, memo_bytes).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "memo after crash"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let mut prepared = table.clone();
    let memo = prepared.apply_memo_updates(&path).unwrap().unwrap();
    let payload = memo_snapshot_payload(&prepared.to_bytes(), &memo).unwrap();
    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        recovered.active_record(1).unwrap().values["NAME"],
        "memo after crash"
    );
    assert_eq!(
        &recovered.to_bytes()[memo_start..memo_start + 10],
        b"         2"
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn recovers_dbf_and_memo_delta_from_wal_before_reading() {
    let path = std::env::temp_dir().join(format!(
        "txbase-dbf-memo-delta-recovery-{}-{}.dbf",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let memo_path = path.with_extension("dbt");
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);
    let _ = fs::remove_file(&wal_path);

    let mut bytes = fixture();
    bytes[0] = 0x83;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let memo_start = record_start + 4;
    bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
    fs::write(&path, &bytes).unwrap();

    let mut memo_bytes = vec![0; DBT_BLOCK_SIZE * 2];
    let text = b"memo before delta";
    memo_bytes[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + text.len()].copy_from_slice(text);
    memo_bytes[DBT_BLOCK_SIZE + text.len()] = EOF_MARKER;
    fs::write(&memo_path, memo_bytes).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "memo after delta"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let mut prepared = table.clone();
    let memo = prepared.apply_memo_updates(&path).unwrap().unwrap();
    let full_payload = memo_snapshot_payload(&prepared.to_bytes(), &memo).unwrap();
    let payload = delta_payload(&prepared, &path, Some(&memo), full_payload.len())
        .unwrap()
        .expect("path-loaded memo mutation should fit in a delta");
    assert!(payload.starts_with(DELTA_MAGIC));
    assert!(payload.len() < full_payload.len());

    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        recovered.active_record(1).unwrap().values["NAME"],
        "memo after delta"
    );
    assert_eq!(
        &recovered.to_bytes()[memo_start..memo_start + 10],
        b"         2"
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

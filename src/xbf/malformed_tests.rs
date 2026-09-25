use super::{
    XbfField, XbfLimits, XbfRecord, XbfTable, XbfType, XbfValue, decode, decode_with_limits, encode,
};

fn hex_fixture(input: &str) -> Vec<u8> {
    input
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

#[test]
fn rejects_malformed_xbf_header_corpus() {
    let cases = [
        (
            "truncated-header",
            include_str!("../../tests/corpus/xbf/truncated-header.hex"),
            "header is truncated",
        ),
        (
            "invalid-magic",
            include_str!("../../tests/corpus/xbf/invalid-magic.hex"),
            "magic is invalid",
        ),
        (
            "unknown-flags",
            include_str!("../../tests/corpus/xbf/unknown-flags.hex"),
            "unknown feature flags",
        ),
    ];

    for (name, fixture, message) in cases {
        let error = decode(&hex_fixture(fixture)).unwrap_err();
        assert!(
            error.to_string().contains(message),
            "{name}: expected {message:?}, got {error}"
        );
    }
}

fn table_fixture() -> XbfTable {
    XbfTable {
        generation: 1,
        fields: vec![
            XbfField {
                name: "TEXT".into(),
                ty: XbfType::String,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "BOOL".into(),
                ty: XbfType::Boolean,
                nullable: false,
                primary_key: false,
                unique: false,
            },
        ],
        records: vec![
            XbfRecord {
                deleted: false,
                values: vec![XbfValue::String("ok".into()), XbfValue::Boolean(true)],
            },
            XbfRecord {
                deleted: false,
                values: vec![XbfValue::String("second".into()), XbfValue::Boolean(false)],
            },
        ],
    }
}

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn refresh_header_checksum(bytes: &mut [u8]) {
    bytes[92..96].fill(0);
    let checksum = super::checksum::crc32c(&bytes[..100]);
    put_u32(bytes, 92, checksum);
}

fn refresh_schema_checksum(bytes: &mut [u8]) {
    let offset = get_u64(bytes, 16) as usize;
    let length = get_u64(bytes, 24) as usize;
    let checksum = super::checksum::crc32c(&bytes[offset..offset + length]);
    put_u32(bytes, 32, checksum);
    refresh_header_checksum(bytes);
}

fn refresh_directory_checksum(bytes: &mut [u8]) {
    let offset = get_u64(bytes, 36) as usize;
    let length = get_u64(bytes, 44) as usize;
    let checksum = super::checksum::crc32c(&bytes[offset..offset + length]);
    put_u32(bytes, 52, checksum);
    refresh_header_checksum(bytes);
}

fn refresh_data_checksum(bytes: &mut [u8]) {
    let offset = get_u64(bytes, 56) as usize;
    let length = get_u64(bytes, 64) as usize;
    let checksum = super::checksum::crc32c(&bytes[offset..offset + length]);
    put_u32(bytes, 72, checksum);
    refresh_header_checksum(bytes);
}

#[test]
fn rejects_each_xbf_checksum_mismatch() {
    let encoded = encode(&table_fixture()).unwrap();
    let checksum_offsets = [
        (92, "header checksum"),
        (get_u64(&encoded, 16) as usize, "schema checksum"),
        (get_u64(&encoded, 36) as usize, "record-directory checksum"),
        (get_u64(&encoded, 56) as usize, "record-data checksum"),
    ];

    for (offset, message) in checksum_offsets {
        let mut corrupted = encoded.clone();
        corrupted[offset] ^= 1;
        let error = decode(&corrupted).unwrap_err();
        assert!(
            error.to_string().contains(message),
            "expected {message:?}, got {error}"
        );
    }
}

#[test]
fn rejects_invalid_xbf_header_metadata() {
    let encoded = encode(&table_fixture()).unwrap();
    let mutations = [
        (4, 2, 2, "unsupported XBF major version"),
        (6, 2, 1, "unsupported XBF minor version"),
        (12, 4, 99, "XBF header length is not 100"),
        (96, 4, 1, "XBF header reserved field is non-zero"),
    ];

    for (offset, width, value, message) in mutations {
        let mut invalid = encoded.clone();
        if width == 2 {
            put_u16(&mut invalid, offset, value as u16);
        } else {
            put_u32(&mut invalid, offset, value as u32);
        }
        refresh_header_checksum(&mut invalid);
        let error = decode(&invalid).unwrap_err();
        assert!(
            error.to_string().contains(message),
            "expected {message:?}, got {error}"
        );
    }
}

#[test]
fn rejects_malformed_xbf_sections_and_directory() {
    let encoded = encode(&table_fixture()).unwrap();

    let mut unordered_sections = encoded.clone();
    let schema_offset = get_u64(&unordered_sections, 16);
    put_u64(&mut unordered_sections, 36, schema_offset);
    refresh_header_checksum(&mut unordered_sections);
    let error = decode(&unordered_sections).unwrap_err();
    assert!(error.to_string().contains("sections are unordered"));

    let mut mismatched_directory = encoded.clone();
    put_u64(&mut mismatched_directory, 76, 3);
    refresh_header_checksum(&mut mismatched_directory);
    let error = decode(&mismatched_directory).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("record directory length does not match record count")
    );

    let mut overlapping_records = encoded;
    let directory_offset = get_u64(&overlapping_records, 36) as usize;
    let first_record_offset = get_u64(&overlapping_records, directory_offset);
    put_u64(
        &mut overlapping_records,
        directory_offset + 24,
        first_record_offset,
    );
    refresh_directory_checksum(&mut overlapping_records);
    let error = decode(&overlapping_records).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("record directory entries overlap")
    );
}

#[test]
fn rejects_xbf_section_and_record_bounds_overflow() {
    let encoded = encode(&table_fixture()).unwrap();

    let mut invalid_section = encoded.clone();
    put_u64(&mut invalid_section, 16, u64::MAX);
    refresh_header_checksum(&mut invalid_section);
    let error = decode(&invalid_section).unwrap_err();
    assert!(error.to_string().contains("schema"), "got {error}");

    let mut excessive_count = encoded.clone();
    put_u64(&mut excessive_count, 76, u64::MAX);
    refresh_header_checksum(&mut excessive_count);
    let limits = XbfLimits {
        max_records: usize::MAX,
        ..XbfLimits::default()
    };
    assert!(
        decode_with_limits(&excessive_count, &limits).is_err(),
        "accepted an overflowing record count"
    );

    let directory_offset = get_u64(&encoded, 36) as usize;
    let mut overflowing_entry = encoded.clone();
    put_u64(&mut overflowing_entry, directory_offset, u64::MAX - 7);
    put_u64(&mut overflowing_entry, directory_offset + 8, 16);
    refresh_directory_checksum(&mut overflowing_entry);
    let error = decode(&overflowing_entry).unwrap_err();
    assert!(error.to_string().contains("overflows"), "got {error}");

    let data_end = get_u64(&encoded, 56) + get_u64(&encoded, 64);
    let mut out_of_range_entry = encoded;
    put_u64(&mut out_of_range_entry, directory_offset, data_end);
    put_u64(&mut out_of_range_entry, directory_offset + 8, 1);
    refresh_directory_checksum(&mut out_of_range_entry);
    let error = decode(&out_of_range_entry).unwrap_err();
    assert!(
        error.to_string().contains("outside record data"),
        "got {error}"
    );
}

#[test]
fn rejects_malformed_schema_and_directory_metadata() {
    let encoded = encode(&table_fixture()).unwrap();
    let schema_offset = get_u64(&encoded, 16) as usize;
    let first_field_type = schema_offset + 4 + 2 + 4;
    let first_field_flags = first_field_type + 1;
    let first_field_reserved = first_field_flags + 1;

    let mut reserved_type = encoded.clone();
    reserved_type[first_field_type] = 0x50;
    refresh_schema_checksum(&mut reserved_type);
    let error = decode(&reserved_type).unwrap_err();
    assert!(error.to_string().contains("decimal type is reserved"));

    let mut unknown_schema_flags = encoded.clone();
    unknown_schema_flags[first_field_flags] = 0x80;
    refresh_schema_checksum(&mut unknown_schema_flags);
    let error = decode(&unknown_schema_flags).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("schema contains unknown constraint flags")
    );

    let mut non_zero_schema_reserved = encoded.clone();
    put_u16(&mut non_zero_schema_reserved, first_field_reserved, 1);
    refresh_schema_checksum(&mut non_zero_schema_reserved);
    let error = decode(&non_zero_schema_reserved).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("schema reserved field is non-zero")
    );

    let directory_offset = get_u64(&encoded, 36) as usize;
    let mut unknown_directory_flags = encoded.clone();
    put_u32(&mut unknown_directory_flags, directory_offset + 16, 0x02);
    refresh_directory_checksum(&mut unknown_directory_flags);
    let error = decode(&unknown_directory_flags).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("record directory contains unknown flags")
    );

    let mut non_zero_directory_reserved = encoded;
    put_u32(&mut non_zero_directory_reserved, directory_offset + 20, 1);
    refresh_directory_checksum(&mut non_zero_directory_reserved);
    let error = decode(&non_zero_directory_reserved).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("record directory contains unknown flags")
    );
}

#[test]
fn rejects_malformed_xbf_utf8_and_payload_values() {
    let encoded = encode(&table_fixture()).unwrap();
    let data_offset = get_u64(&encoded, 56) as usize;

    let mut invalid_utf8 = encoded.clone();
    invalid_utf8[data_offset + 5] = 0xff;
    refresh_data_checksum(&mut invalid_utf8);
    let error = decode(&invalid_utf8).unwrap_err();
    assert!(error.to_string().contains("is not UTF-8"));

    let mut invalid_boolean = encoded;
    invalid_boolean[data_offset + 12] = 2;
    refresh_data_checksum(&mut invalid_boolean);
    let error = decode(&invalid_boolean).unwrap_err();
    assert!(error.to_string().contains("is not 0 or 1"));
}

#[test]
fn rejects_malformed_null_fixed_width_and_json_values() {
    let encoded = encode(&table_fixture()).unwrap();
    let data_offset = get_u64(&encoded, 56) as usize;

    let mut null_payload = encoded.clone();
    null_payload[data_offset] = 0;
    put_u32(&mut null_payload, data_offset + 1, 1);
    refresh_data_checksum(&mut null_payload);
    let error = decode(&null_payload).unwrap_err();
    assert!(error.to_string().contains("NULL value for field TEXT"));

    let text_length = u32::from_le_bytes(
        encoded[data_offset + 1..data_offset + 5]
            .try_into()
            .unwrap(),
    ) as usize;
    let bool_tag = data_offset + 1 + 4 + text_length;
    let mut invalid_boolean_width = encoded;
    put_u32(&mut invalid_boolean_width, bool_tag + 1, 2);
    refresh_data_checksum(&mut invalid_boolean_width);
    assert!(
        decode(&invalid_boolean_width).is_err(),
        "accepted a boolean payload with an invalid width"
    );

    let json_table = XbfTable {
        generation: 1,
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
    let mut invalid_json = encode(&json_table).unwrap();
    let json_data_offset = get_u64(&invalid_json, 56) as usize;
    invalid_json[json_data_offset + 5] = b'!';
    refresh_data_checksum(&mut invalid_json);
    let error = decode(&invalid_json).unwrap_err();
    assert!(error.to_string().contains("JSON field DOC is invalid"));
}

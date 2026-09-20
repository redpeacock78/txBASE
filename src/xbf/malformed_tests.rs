use super::{XbfField, XbfRecord, XbfTable, XbfType, XbfValue, decode, encode};

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

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn refresh_header_checksum(bytes: &mut [u8]) {
    bytes[92..96].fill(0);
    let checksum = super::checksum::crc32c(&bytes[..100]);
    put_u32(bytes, 92, checksum);
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

use super::{XbfLimits, XbfTable, decode, decode_with_limits, encode};

const EXTENSION_ENTRY_SIZE: usize = 28;

fn extension_fixture(entries: &[(u32, u32, &[u8])]) -> Vec<u8> {
    let header_length = super::HEADER_SIZE + entries.len() * EXTENSION_ENTRY_SIZE;
    let schema_offset = header_length;
    let directory_offset = schema_offset + 4;
    let data_offset = directory_offset;
    let mut bytes = vec![0; header_length];
    bytes[..4].copy_from_slice(&super::MAGIC);
    put_u16(&mut bytes, 4, 1);
    put_u16(&mut bytes, 6, 1);
    put_u32(&mut bytes, 12, header_length as u32);
    put_u64(&mut bytes, 16, schema_offset as u64);
    put_u64(&mut bytes, 24, 4);
    put_u32(&mut bytes, 32, super::checksum::crc32c(&[0, 0, 0, 0]));
    put_u64(&mut bytes, 36, directory_offset as u64);
    put_u64(&mut bytes, 44, 0);
    put_u32(&mut bytes, 52, super::checksum::crc32c(&[]));
    put_u64(&mut bytes, 56, data_offset as u64);
    put_u64(&mut bytes, 64, 0);
    put_u32(&mut bytes, 72, super::checksum::crc32c(&[]));
    put_u64(&mut bytes, 76, 0);
    put_u64(&mut bytes, 84, 7);
    bytes.extend_from_slice(&[0, 0, 0, 0]);

    let mut section_offset = data_offset as u64;
    for (index, (section_type, flags, payload)) in entries.iter().enumerate() {
        let descriptor = super::HEADER_SIZE + index * EXTENSION_ENTRY_SIZE;
        put_u32(&mut bytes, descriptor, *section_type);
        put_u32(&mut bytes, descriptor + 4, *flags);
        put_u64(&mut bytes, descriptor + 8, section_offset);
        put_u64(&mut bytes, descriptor + 16, payload.len() as u64);
        put_u32(
            &mut bytes,
            descriptor + 24,
            super::checksum::crc32c(payload),
        );
        bytes.extend_from_slice(payload);
        section_offset += payload.len() as u64;
    }
    refresh_header_checksum(&mut bytes);
    bytes
}

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn refresh_header_checksum(bytes: &mut [u8]) {
    bytes[92..96].fill(0);
    let header_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let checksum = super::checksum::crc32c(&bytes[..header_length.min(bytes.len())]);
    put_u32(bytes, 92, checksum);
}

#[test]
fn reads_unknown_optional_xbf_minor_sections_and_drops_them_on_encode() {
    let bytes = extension_fixture(&[(7, 1, &[0xa5, 0x5a])]);
    let table = XbfTable {
        generation: 7,
        fields: Vec::new(),
        records: Vec::new(),
    };

    assert_eq!(decode(&bytes).unwrap(), table);
    let mut later_minor = bytes;
    put_u16(&mut later_minor, 6, 2);
    refresh_header_checksum(&mut later_minor);
    assert_eq!(decode(&later_minor).unwrap(), table);

    let encoded = encode(&table).unwrap();
    assert_eq!(u16::from_le_bytes(encoded[6..8].try_into().unwrap()), 0);
    assert_eq!(u32::from_le_bytes(encoded[12..16].try_into().unwrap()), 100);
}

#[test]
fn rejects_unknown_required_xbf_extensions_and_invalid_flags() {
    let required = extension_fixture(&[(7, 0, &[0xa5])]);
    assert!(
        decode(&required)
            .unwrap_err()
            .to_string()
            .contains("unknown required extension section")
    );

    let mut unsupported_flags = extension_fixture(&[(7, 1, &[0xa5])]);
    put_u32(&mut unsupported_flags, 104, 0x02);
    refresh_header_checksum(&mut unsupported_flags);
    assert!(
        decode(&unsupported_flags)
            .unwrap_err()
            .to_string()
            .contains("unsupported flags")
    );
}

#[test]
fn rejects_malformed_xbf_extension_directory_and_payloads() {
    let mut zero_type = extension_fixture(&[(7, 1, &[0xa5])]);
    put_u32(&mut zero_type, 100, 0);
    refresh_header_checksum(&mut zero_type);
    assert!(
        decode(&zero_type)
            .unwrap_err()
            .to_string()
            .contains("types are zero")
    );

    let mut duplicate_types = extension_fixture(&[(7, 1, &[0xa5]), (9, 1, &[0x5a])]);
    put_u32(&mut duplicate_types, 128, 7);
    refresh_header_checksum(&mut duplicate_types);
    assert!(
        decode(&duplicate_types)
            .unwrap_err()
            .to_string()
            .contains("duplicated, or unordered")
    );

    let out_of_order = extension_fixture(&[(9, 1, &[0xa5]), (7, 1, &[0x5a])]);
    assert!(
        decode(&out_of_order)
            .unwrap_err()
            .to_string()
            .contains("duplicated, or unordered")
    );

    let mut invalid_directory_length = extension_fixture(&[(7, 1, &[0xa5])]);
    put_u32(&mut invalid_directory_length, 12, 129);
    refresh_header_checksum(&mut invalid_directory_length);
    assert!(
        decode(&invalid_directory_length)
            .unwrap_err()
            .to_string()
            .contains("extension directory length is invalid")
    );

    let mut invalid_payload_checksum = extension_fixture(&[(7, 1, &[0xa5])]);
    let payload_offset = get_u64(&invalid_payload_checksum, 56) as usize;
    invalid_payload_checksum[payload_offset] ^= 1;
    assert!(
        decode(&invalid_payload_checksum)
            .unwrap_err()
            .to_string()
            .contains("extension checksum does not match")
    );

    let mut gap = extension_fixture(&[(7, 1, &[0xa5])]);
    let gap_offset = get_u64(&gap, 108) + 1;
    put_u64(&mut gap, 108, gap_offset);
    refresh_header_checksum(&mut gap);
    assert!(
        decode(&gap)
            .unwrap_err()
            .to_string()
            .contains("unordered, overlapping, or have gaps")
    );

    let mut outside_file = extension_fixture(&[(7, 1, &[0xa5])]);
    put_u64(&mut outside_file, 116, 100);
    refresh_header_checksum(&mut outside_file);
    assert!(
        decode(&outside_file)
            .unwrap_err()
            .to_string()
            .contains("extension is outside the file")
    );

    let oversized = extension_fixture(&[(7, 1, &[0; 29])]);
    let mut limits = XbfLimits::default();
    limits.max_section_size = EXTENSION_ENTRY_SIZE;
    assert!(
        decode_with_limits(&oversized, &limits)
            .unwrap_err()
            .to_string()
            .contains("extension section exceeds the configured limit")
    );

    let bounded_directory = extension_fixture(&[(7, 1, &[0xa5])]);
    limits.max_section_size = EXTENSION_ENTRY_SIZE - 1;
    assert!(
        decode_with_limits(&bounded_directory, &limits)
            .unwrap_err()
            .to_string()
            .contains("extension directory exceeds the configured limit")
    );
}

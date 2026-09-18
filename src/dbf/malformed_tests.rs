use super::*;

fn hex_fixture(input: &str) -> Vec<u8> {
    input
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

#[test]
fn rejects_malformed_dbf_corpus() {
    let cases = [
        (
            "truncated-header",
            include_str!("../../tests/corpus/dbf/truncated-header.hex"),
            "header is truncated",
        ),
        (
            "invalid-header-length",
            include_str!("../../tests/corpus/dbf/invalid-header-length.hex"),
            "header length",
        ),
        (
            "missing-field-terminator",
            include_str!("../../tests/corpus/dbf/missing-field-terminator.hex"),
            "field descriptor terminator",
        ),
        (
            "truncated-record",
            include_str!("../../tests/corpus/dbf/truncated-record.hex"),
            "record area is truncated",
        ),
        (
            "invalid-deletion-marker",
            include_str!("../../tests/corpus/dbf/invalid-deletion-marker.hex"),
            "unknown deletion marker",
        ),
    ];

    for (name, fixture, message) in cases {
        let error = DbfTable::from_bytes(&hex_fixture(fixture)).unwrap_err();
        assert!(
            error.to_string().contains(message),
            "{name}: expected {message:?}, got {error}"
        );
    }
}

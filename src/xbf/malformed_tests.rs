use super::decode;

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

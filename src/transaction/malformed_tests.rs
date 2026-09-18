use super::*;
use std::fs;

fn hex_fixture(input: &str) -> Vec<u8> {
    input
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

#[test]
fn rejects_malformed_wal_corpus() {
    let cases = [
        (
            "invalid-magic",
            include_str!("../../tests/corpus/wal/invalid-magic.hex"),
            "invalid magic",
        ),
        (
            "invalid-size",
            include_str!("../../tests/corpus/wal/invalid-size.hex"),
            "exceeds the configured size limit",
        ),
        (
            "invalid-lsn",
            include_str!("../../tests/corpus/wal/invalid-lsn.hex"),
            "expected 0",
        ),
    ];

    for (name, fixture, message) in cases {
        let path = std::env::temp_dir().join(format!(
            "txbase-wal-corpus-{}-{name}.log",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        fs::write(&path, hex_fixture(fixture)).unwrap();
        let error = FileWal::open(&path).unwrap_err();
        assert!(
            error.to_string().contains(message),
            "{name}: expected {message:?}, got {error}"
        );
        fs::remove_file(path).unwrap();
    }
}

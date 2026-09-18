use super::*;
use std::fs;

fn hex_fixture(input: &str) -> Vec<u8> {
    input
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

#[test]
fn rejects_malformed_memo_corpus() {
    let cases = [
        (
            "truncated-dbt",
            include_str!("../../tests/corpus/memo/truncated-dbt.hex"),
            "dbt",
            0x83,
            "DBT header is truncated",
        ),
        (
            "truncated-fpt",
            include_str!("../../tests/corpus/memo/truncated-fpt.hex"),
            "fpt",
            0x30,
            "FPT header is truncated",
        ),
        (
            "invalid-fpt-block-size",
            include_str!("../../tests/corpus/memo/invalid-fpt-block-size.hex"),
            "fpt",
            0x30,
            "FPT block size or header is invalid",
        ),
    ];

    for (name, fixture, extension, version, message) in cases {
        let path = std::env::temp_dir().join(format!(
            "txbase-memo-corpus-{}-{name}.{extension}",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        fs::write(&path, hex_fixture(fixture)).unwrap();
        let error = MemoFile::open(&path, version).unwrap_err();
        assert!(
            error.to_string().contains(message),
            "{name}: expected {message:?}, got {error}"
        );
        fs::remove_file(path).unwrap();
    }
}

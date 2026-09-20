use std::fs;
use std::process::Command;

#[test]
fn wal_inspect_cli_reports_a_torn_tail_without_mutating_the_file() {
    let path =
        std::env::temp_dir().join(format!("txbase-cli-wal-inspect-{}.wal", std::process::id()));
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"TXWL");
    bytes.extend_from_slice(&6u32.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(b"commit");
    bytes.extend_from_slice(b"TXWL");
    fs::write(&path, &bytes).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_txbase"))
        .args(["wal", "inspect", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "wal inspect failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["format"], "txbase-wal");
    assert_eq!(report["file_bytes"], serde_json::json!(bytes.len()));
    assert_eq!(report["valid_bytes"], serde_json::json!(22));
    assert_eq!(report["truncated_tail"], serde_json::json!(true));
    assert_eq!(report["records"][0]["lsn"], serde_json::json!(0));
    assert_eq!(report["records"][0]["length"], serde_json::json!(6));
    assert_eq!(fs::read(&path).unwrap(), bytes);

    fs::remove_file(path).unwrap();
}

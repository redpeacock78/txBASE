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

#[test]
fn backup_and_restore_cli_copy_a_dbf() {
    let source = std::env::temp_dir().join(format!(
        "txbase-cli-backup-source-{}.dbf",
        std::process::id()
    ));
    let backup =
        std::env::temp_dir().join(format!("txbase-cli-backup-copy-{}.dbf", std::process::id()));
    let restored = std::env::temp_dir().join(format!(
        "txbase-cli-backup-restored-{}.dbf",
        std::process::id()
    ));
    for path in [&source, &backup, &restored] {
        let _ = fs::remove_file(path);
    }

    let fixture = include_str!("fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect::<Vec<_>>();
    fs::write(&source, &fixture).unwrap();

    for (operation, input, output_path) in [
        ("backup", &source, &backup),
        ("restore", &backup, &restored),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_txbase"))
            .args([
                operation,
                input.to_str().unwrap(),
                output_path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{operation} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    assert_eq!(fs::read(&restored).unwrap(), fixture);

    for path in [source, backup, restored] {
        fs::remove_file(path).unwrap();
    }
}

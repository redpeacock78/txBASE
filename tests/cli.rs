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
    let source_memo = source.with_extension("dbt");
    let backup_memo = backup.with_extension("dbt");
    let restored_memo = restored.with_extension("dbt");
    for path in [
        &source,
        &backup,
        &restored,
        &source_memo,
        &backup_memo,
        &restored_memo,
    ] {
        let _ = fs::remove_file(path);
    }

    let fixture = include_str!("fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect::<Vec<_>>();
    fs::write(&source, &fixture).unwrap();
    fs::write(&source_memo, b"source memo").unwrap();

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
    assert_eq!(fs::read(&backup_memo).unwrap(), b"source memo");
    assert_eq!(fs::read(&restored_memo).unwrap(), b"source memo");

    for path in [
        source,
        backup,
        restored,
        source_memo,
        backup_memo,
        restored_memo,
    ] {
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn dbf_maintenance_cli_commands_operate_on_a_dbf() {
    let recall_path =
        std::env::temp_dir().join(format!("txbase-cli-recall-{}.dbf", std::process::id()));
    let pack_path =
        std::env::temp_dir().join(format!("txbase-cli-pack-{}.dbf", std::process::id()));
    for path in [&recall_path, &pack_path] {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(path.with_extension("txbase.state"));
        let _ = fs::remove_file(path.with_extension("txbase.wal"));
        let _ = fs::remove_file(path.with_extension("txbase.lock"));
    }

    let fixture = include_str!("fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect::<Vec<_>>();
    fs::write(&recall_path, &fixture).unwrap();
    fs::write(&pack_path, &fixture).unwrap();

    let schema = Command::new(env!("CARGO_BIN_EXE_txbase"))
        .args(["schema", recall_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(schema.status.success());
    let schema_json: serde_json::Value = serde_json::from_slice(&schema.stdout).unwrap();
    assert_eq!(schema_json["format"], "dbf");
    assert_eq!(schema_json["record_count"], 2);

    let verify = Command::new(env!("CARGO_BIN_EXE_txbase"))
        .args(["verify", recall_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(verify.status.success());
    let verify_json: serde_json::Value = serde_json::from_slice(&verify.stdout).unwrap();
    assert_eq!(verify_json["valid"], true);

    let recall = Command::new(env!("CARGO_BIN_EXE_txbase"))
        .args(["recall", recall_path.to_str().unwrap(), "2"])
        .output()
        .unwrap();
    assert!(
        recall.status.success(),
        "recall failed: {}",
        String::from_utf8_lossy(&recall.stderr)
    );
    let recalled = Command::new(env!("CARGO_BIN_EXE_txbase"))
        .arg(recall_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(recalled.status.success());
    let recalled_json: serde_json::Value = serde_json::from_slice(&recalled.stdout).unwrap();
    assert_eq!(recalled_json.as_array().unwrap().len(), 2);

    let pack = Command::new(env!("CARGO_BIN_EXE_txbase"))
        .args(["pack", pack_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        pack.status.success(),
        "pack failed: {}",
        String::from_utf8_lossy(&pack.stderr)
    );
    let packed = Command::new(env!("CARGO_BIN_EXE_txbase"))
        .arg(pack_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(packed.status.success());
    let packed_json: serde_json::Value = serde_json::from_slice(&packed.stdout).unwrap();
    assert_eq!(packed_json.as_array().unwrap().len(), 1);

    for path in [recall_path, pack_path] {
        for extension in ["txbase.state", "txbase.wal", "txbase.lock"] {
            let _ = fs::remove_file(path.with_extension(extension));
        }
        fs::remove_file(path).unwrap();
    }
}

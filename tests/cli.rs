use std::fs;
use std::process::{Command, Output};

fn users_fixture() -> Vec<u8> {
    include_str!("fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect()
}

fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_txbase"))
        .args(args)
        .output()
        .unwrap()
}

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

    let output = run_cli(&["wal", "inspect", path.to_str().unwrap()]);
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

    let fixture = users_fixture();
    fs::write(&source, &fixture).unwrap();
    fs::write(&source_memo, b"source memo").unwrap();

    for (operation, input, output_path) in [
        ("backup", &source, &backup),
        ("restore", &backup, &restored),
    ] {
        let output = run_cli(&[
            operation,
            input.to_str().unwrap(),
            output_path.to_str().unwrap(),
        ]);
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

    let fixture = users_fixture();
    fs::write(&recall_path, &fixture).unwrap();
    fs::write(&pack_path, &fixture).unwrap();

    let schema = run_cli(&["schema", recall_path.to_str().unwrap()]);
    assert!(schema.status.success());
    let schema_json: serde_json::Value = serde_json::from_slice(&schema.stdout).unwrap();
    assert_eq!(schema_json["format"], "dbf");
    assert_eq!(schema_json["record_count"], 2);

    let verify = run_cli(&["verify", recall_path.to_str().unwrap()]);
    assert!(verify.status.success());
    let verify_json: serde_json::Value = serde_json::from_slice(&verify.stdout).unwrap();
    assert_eq!(verify_json["valid"], true);

    let recall = run_cli(&["recall", recall_path.to_str().unwrap(), "2"]);
    assert!(
        recall.status.success(),
        "recall failed: {}",
        String::from_utf8_lossy(&recall.stderr)
    );
    let recalled = run_cli(&[recall_path.to_str().unwrap()]);
    assert!(recalled.status.success());
    let recalled_json: serde_json::Value = serde_json::from_slice(&recalled.stdout).unwrap();
    assert_eq!(recalled_json.as_array().unwrap().len(), 2);

    let pack = run_cli(&["pack", pack_path.to_str().unwrap()]);
    assert!(
        pack.status.success(),
        "pack failed: {}",
        String::from_utf8_lossy(&pack.stderr)
    );
    let packed = run_cli(&[pack_path.to_str().unwrap()]);
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

#[test]
fn xbf_cli_import_report_and_export_a_dbf() {
    let source =
        std::env::temp_dir().join(format!("txbase-cli-xbf-source-{}.dbf", std::process::id()));
    let snapshot = std::env::temp_dir().join(format!(
        "txbase-cli-xbf-snapshot-{}.xbf",
        std::process::id()
    ));
    let exported = std::env::temp_dir().join(format!(
        "txbase-cli-xbf-exported-{}.dbf",
        std::process::id()
    ));
    let schema_exported = std::env::temp_dir().join(format!(
        "txbase-cli-xbf-schema-exported-{}.dbf",
        std::process::id()
    ));
    for path in [&source, &snapshot, &exported, &schema_exported] {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(path.with_extension("xwl"));
        let _ = fs::remove_file(path.with_extension("txschema.json"));
        let _ = fs::remove_file(path.with_extension("txbase.state"));
        let _ = fs::remove_file(path.with_extension("txbase.wal"));
        let _ = fs::remove_file(path.with_extension("txbase.lock"));
        let _ = fs::remove_file(path.with_extension("txidx"));
        let _ = fs::remove_dir_all(path.with_extension("txbase-xbf-export"));
    }

    let fixture = users_fixture();
    fs::write(&source, &fixture).unwrap();

    let import = run_cli(&[
        "xbf",
        "import",
        source.to_str().unwrap(),
        snapshot.to_str().unwrap(),
    ]);
    assert!(
        import.status.success(),
        "xbf import failed: {}",
        String::from_utf8_lossy(&import.stderr)
    );

    let report = run_cli(&["xbf", "report", snapshot.to_str().unwrap()]);
    assert!(report.status.success());
    let report_json: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(report_json["representable"], true);
    assert_eq!(report_json["requires_schema_sidecar"], true);

    let export = run_cli(&[
        "xbf",
        "export",
        snapshot.to_str().unwrap(),
        exported.to_str().unwrap(),
    ]);
    assert!(!export.status.success());
    assert!(
        String::from_utf8_lossy(&export.stderr)
            .contains("DBF export cannot preserve a not-null constraint")
    );
    assert!(!exported.exists());

    let schema_export = run_cli(&[
        "xbf",
        "export",
        snapshot.to_str().unwrap(),
        schema_exported.to_str().unwrap(),
        "--schema",
    ]);
    assert!(
        schema_export.status.success(),
        "schema-preserving XBF export failed: {}",
        String::from_utf8_lossy(&schema_export.stderr)
    );
    assert!(schema_exported.with_extension("txschema.json").exists());

    let source_table = txbase::dbf::DbfTable::from_path(&source).unwrap();
    assert_eq!(
        txbase::dbf::DbfTable::from_path(&schema_exported)
            .unwrap()
            .active_json(),
        source_table.active_json()
    );

    for path in [source, snapshot, exported, schema_exported] {
        for extension in [
            "xwl",
            "txschema.json",
            "txbase.state",
            "txbase.wal",
            "txbase.lock",
            "txidx",
        ] {
            let _ = fs::remove_file(path.with_extension(extension));
        }
        let _ = fs::remove_dir_all(path.with_extension("txbase-xbf-export"));
        fs::remove_file(path).unwrap();
    }
}

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::json;
use txbase::catalog::Catalog;
use txbase::replication::{REPLICATION_TRANSPORT_VERSION, ReplicationLog};
use txbase::xbase::{OperationIr, OperationMethod};

static NEXT_REPLICATION_CLI_ID: AtomicUsize = AtomicUsize::new(0);

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

fn replication_fixture_catalog(label: &str) -> PathBuf {
    let id = NEXT_REPLICATION_CLI_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "txbase-cli-replication-{label}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("users.dbf"),
        include_str!("fixtures/users.dbf.hex")
            .split_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    root
}

fn replication_post(record_id: i64, name: &str) -> OperationIr {
    OperationIr {
        method: OperationMethod::Post,
        path: "/users/records".into(),
        body: Some(json!({
            "ID": record_id,
            "NAME": name,
            "AGE": 42,
            "ACTIVE": true
        })),
    }
}

fn http_json_response(body: Vec<u8>) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body)
    .collect()
}

fn spawn_replication_sequence(responses: Vec<Vec<u8>>) -> (String, JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "replication CLI did not connect");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("replication CLI listener failed: {error}"),
                }
            };
            requests.push(read_http_request(&mut stream));
            stream.write_all(&response).unwrap();
        }
        requests
    });
    (format!("http://{address}/api/"), handle)
}

fn read_http_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0);
        request.extend_from_slice(&buffer[..read]);
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while request.len() < header_end + content_length {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0);
        request.extend_from_slice(&buffer[..read]);
    }
    request
}

#[test]
fn cli_help_lists_the_command_families_and_unknown_commands_fail() {
    let help = run_cli(&["--help"]);
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    for command in [
        "txbase read",
        "txbase schema apply",
        "txbase cdc",
        "txbase mvcc",
        "txbase wal inspect",
        "txbase xbf",
        "txbase index",
        "txbase serve",
        "txbase serve-catalog",
        "txbase replicate catch-up",
    ] {
        assert!(help.contains(command), "help is missing {command}");
    }
    assert!(help.contains("--replication-role authority|follower"));
    assert!(help.contains("--collation unicode-lowercase"));

    let unknown = run_cli(&["not-a-command"]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown command"));
}

#[test]
fn replication_catch_up_cli_applies_entries_and_reports_progress() {
    let leader_root = replication_fixture_catalog("leader");
    let follower_root = replication_fixture_catalog("follower");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let mut authority = ReplicationLog::new(4).unwrap();
    let entry = authority
        .propose(&leader, vec![replication_post(3, "Carol")])
        .unwrap();
    let batch = authority.entry_batch(0, 1).unwrap();
    let responses = vec![
        http_json_response(
            json!({
                "transport_version": REPLICATION_TRANSPORT_VERSION,
                "role": "authority",
                "term": 4,
                "base_index": 0,
                "base_transaction_id": 0,
                "last_index": 1,
                "last_transaction_id": 1,
                "follower_count": 0,
                "safe_compaction_index": null,
                "schema_tag": entry.schema_tag
            })
            .to_string()
            .into_bytes(),
        ),
        http_json_response(batch.to_json().unwrap()),
        http_json_response(
            json!({
                "transport_version": REPLICATION_TRANSPORT_VERSION,
                "outcome": "accepted",
                "follower_id": "follower-1",
                "index": 1,
                "transaction_id": 1,
                "safe_compaction_index": 1
            })
            .to_string()
            .into_bytes(),
        ),
    ];
    let (authority_url, server) = spawn_replication_sequence(responses);
    let output = run_cli(&[
        "replicate",
        "catch-up",
        follower_root.to_str().unwrap(),
        &authority_url,
        "--replication-term",
        "4",
        "--follower-id",
        "follower-1",
        "--limit",
        "1",
        "--timeout-ms",
        "5000",
    ]);
    assert!(
        output.status.success(),
        "replication catch-up failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["entries_applied"], 1);
    assert_eq!(report["snapshot_installed"], false);
    assert_eq!(report["progress"]["follower_id"], "follower-1");
    assert_eq!(server.join().unwrap().len(), 3);

    let follower = Catalog::from_path(&follower_root).unwrap();
    assert!(
        follower
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_some()
    );
    let log = ReplicationLog::open(&follower, 4).unwrap();
    assert_eq!(log.last_index(), 1);

    fs::remove_dir_all(leader_root).unwrap();
    fs::remove_dir_all(follower_root).unwrap();
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
fn cdc_cli_reads_committed_events_and_supports_an_after_cursor() {
    let path = std::env::temp_dir().join(format!("txbase-cli-cdc-{}.dbf", std::process::id()));
    for extension in [
        "txbase.cdc",
        "txbase.state",
        "txbase.wal",
        "txbase.mvcc",
        "txbase.lock",
    ] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    let _ = fs::remove_file(&path);

    for args in [
        vec!["init", path.to_str().unwrap(), "--field", "ID:N:4:0"],
        vec!["insert", path.to_str().unwrap(), r#"{"ID":1}"#],
        vec!["insert", path.to_str().unwrap(), r#"{"ID":2}"#],
    ] {
        let output = run_cli(&args);
        assert!(
            output.status.success(),
            "CLI command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = run_cli(&["cdc", path.to_str().unwrap()]);
    assert!(output.status.success());
    let events: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 2);
    assert_eq!(events[0]["transaction_id"], 1);
    assert_eq!(events[1]["transaction_id"], 2);

    let output = run_cli(&["cdc", path.to_str().unwrap(), "--after", "1"]);
    assert!(output.status.success());
    let events: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        events,
        serde_json::json!([
            {
                "transaction_id": 2,
                "reset": false,
                "changes": [{
                    "record_number": 2,
                    "before": null,
                    "after": {"deleted": false, "values": {"ID": 2}}
                }]
            }
        ])
    );

    for extension in [
        "txbase.cdc",
        "txbase.state",
        "txbase.wal",
        "txbase.mvcc",
        "txbase.lock",
    ] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn catalog_cdc_cli_reads_an_empty_catalog_stream() {
    let root = std::env::temp_dir().join(format!("txbase-cli-catalog-cdc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    fs::write(root.join("users.dbf"), users_fixture()).unwrap();

    let output = run_cli(&["cdc", "catalog", root.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "catalog CDC CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!([])
    );

    fs::remove_dir_all(root).unwrap();
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
    let recalled = run_cli(&["read", recall_path.to_str().unwrap()]);
    assert!(recalled.status.success());
    let recalled_json: serde_json::Value = serde_json::from_slice(&recalled.stdout).unwrap();
    assert_eq!(recalled_json.as_array().unwrap().len(), 2);

    let pack = run_cli(&["pack", pack_path.to_str().unwrap()]);
    assert!(
        pack.status.success(),
        "pack failed: {}",
        String::from_utf8_lossy(&pack.stderr)
    );
    let packed = run_cli(&["read", pack_path.to_str().unwrap()]);
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
fn schema_apply_cli_installs_valid_metadata_without_changing_dbf_bytes() {
    let path = std::env::temp_dir().join(format!(
        "txbase-cli-schema-apply-{}.dbf",
        std::process::id()
    ));
    let candidate = path.with_extension("candidate.json");
    for extension in ["txschema.json", "txbase.state", "txbase.wal", "txbase.lock"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&candidate);

    let fixture = users_fixture();
    fs::write(&path, &fixture).unwrap();
    fs::write(
        &candidate,
        serde_json::json!({
            "format": "txbase-schema",
            "version": 1,
            "fields": {"ID": {"primary": true}}
        })
        .to_string(),
    )
    .unwrap();

    let output = run_cli(&[
        "schema",
        "apply",
        path.to_str().unwrap(),
        candidate.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "schema apply failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&path).unwrap(), fixture);
    assert!(path.with_extension("txschema.json").exists());

    let schema = run_cli(&["schema", path.to_str().unwrap()]);
    assert!(schema.status.success());
    let schema_json: serde_json::Value = serde_json::from_slice(&schema.stdout).unwrap();
    assert_eq!(
        schema_json["schema_metadata"]["fields"]["ID"]["primary"],
        true
    );

    for extension in ["txschema.json", "txbase.state", "txbase.wal", "txbase.lock"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    fs::remove_file(path).unwrap();
    fs::remove_file(candidate).unwrap();
}

#[test]
fn init_and_insert_cli_lifecycle_preserves_record_numbers() {
    let path = std::env::temp_dir().join(format!(
        "txbase-cli-init-lifecycle-{}.dbf",
        std::process::id()
    ));
    for extension in ["txbase.state", "txbase.wal", "txbase.lock", "txbase.mvcc"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    let _ = fs::remove_file(&path);

    let init = run_cli(&[
        "init",
        path.to_str().unwrap(),
        "--field",
        "ID:N:4:0",
        "--field",
        "NAME:C:16",
    ]);
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    for record in [r#"{"ID":1,"NAME":"Alice"}"#, r#"{"ID":2,"NAME":"Bob"}"#] {
        let insert = run_cli(&["insert", path.to_str().unwrap(), record]);
        assert!(
            insert.status.success(),
            "insert failed: {}",
            String::from_utf8_lossy(&insert.stderr)
        );
    }

    let schema = run_cli(&["schema", path.to_str().unwrap()]);
    assert!(schema.status.success());
    let schema_json: serde_json::Value = serde_json::from_slice(&schema.stdout).unwrap();
    assert_eq!(schema_json["record_count"], 2);
    assert_eq!(schema_json["active_record_count"], 2);

    let read = run_cli(&["read", path.to_str().unwrap()]);
    assert!(read.status.success());
    let records: serde_json::Value = serde_json::from_slice(&read.stdout).unwrap();
    assert_eq!(records[0]["ID"], 1);
    assert_eq!(records[1]["ID"], 2);

    let verify = run_cli(&["verify", path.to_str().unwrap()]);
    assert!(verify.status.success());

    for extension in ["txbase.state", "txbase.wal", "txbase.lock", "txbase.mvcc"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn verify_cli_rejects_a_stale_index_sidecar() {
    let path = std::env::temp_dir().join(format!(
        "txbase-cli-verify-stale-index-{}.dbf",
        std::process::id()
    ));
    for extension in ["txidx", "txbase.state", "txbase.wal", "txbase.lock"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    let _ = fs::remove_file(&path);

    fs::write(&path, users_fixture()).unwrap();
    let build = run_cli(&[
        "index",
        "build",
        path.to_str().unwrap(),
        "NAME",
        "--collation",
        "unicode-lowercase",
    ]);
    assert!(
        build.status.success(),
        "index build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let schema: serde_json::Value = serde_json::from_slice(&build.stdout).unwrap();
    assert_eq!(
        schema["indexes"][0]["collation"],
        serde_json::json!("unicode-lowercase")
    );
    let mut changed = txbase::dbf::DbfTable::from_path(&path).unwrap();
    changed
        .patch_record(
            1,
            serde_json::json!({"NAME": "Changed"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    fs::write(&path, changed.to_bytes()).unwrap();

    let verify = run_cli(&["verify", path.to_str().unwrap()]);
    assert!(!verify.status.success());
    assert!(String::from_utf8_lossy(&verify.stderr).contains("index sidecar is invalid"));

    for extension in ["txidx", "txbase.state", "txbase.wal", "txbase.lock"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    fs::remove_file(path).unwrap();
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
        let _ = fs::remove_file(path);
    }
}

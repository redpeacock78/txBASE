use super::{JSON_QUERY_MEDIA_TYPE, header};
use crate::catalog::Catalog;
use crate::replication::{ReplicationLog, ReplicationSnapshot};
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::{Value, json};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use tiny_http::{Method, Request, StatusCode, TestRequest};

static NEXT_REPLICATION_SERVER_ID: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn temporary_catalog(label: &str) -> PathBuf {
    let id = NEXT_REPLICATION_SERVER_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "txbase-server-replication-{label}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    root
}

fn post(record_id: i64, name: &str) -> OperationIr {
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

fn json_request(method: Method, path: &'static str, body: Vec<u8>) -> Request {
    let body = Box::leak(String::from_utf8(body).unwrap().into_boxed_str());
    TestRequest::new()
        .with_method(method)
        .with_path(path)
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_body(body)
        .into()
}

fn response_json(response: super::HttpResponse) -> Value {
    let mut body = Vec::new();
    response.into_reader().read_to_end(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[test]
fn replication_http_delivers_entries_idempotently_and_reports_status() {
    let leader_root = temporary_catalog("leader");
    let follower_root = temporary_catalog("follower");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let mut follower = Catalog::from_path(&follower_root).unwrap();
    let mut authority = ReplicationLog::new(4).unwrap();
    let entry = authority.propose(&leader, vec![post(3, "Carol")]).unwrap();
    let mut replica = ReplicationLog::new(4).unwrap();

    let mut request = json_request(Method::Post, "/replication/entry", entry.to_json().unwrap());
    let response = super::replication::response(
        &mut request,
        "/replication/entry",
        &mut follower,
        &mut replica,
    )
    .unwrap();
    assert_eq!(response.status_code(), StatusCode(200));
    assert_eq!(response_json(response)["outcome"], "applied");

    let mut duplicate = json_request(Method::Post, "/replication/entry", entry.to_json().unwrap());
    let response = super::replication::response(
        &mut duplicate,
        "/replication/entry",
        &mut follower,
        &mut replica,
    )
    .unwrap();
    assert_eq!(response_json(response)["outcome"], "duplicate");

    let mut status_request = TestRequest::new()
        .with_method(Method::Get)
        .with_path("/replication/status")
        .into();
    let response = super::replication::response(
        &mut status_request,
        "/replication/status",
        &mut follower,
        &mut replica,
    )
    .unwrap();
    let status = response_json(response);
    assert_eq!(status["term"], 4);
    assert_eq!(status["last_index"], 1);
    assert_eq!(status["last_transaction_id"], 1);
    assert!(
        follower
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_some()
    );

    fs::remove_dir_all(leader_root).unwrap();
    fs::remove_dir_all(follower_root).unwrap();
}

#[test]
fn replication_http_installs_and_exports_snapshots() {
    let leader_root = temporary_catalog("snapshot-leader");
    let follower_root = temporary_catalog("snapshot-follower");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let mut authority = ReplicationLog::new(9).unwrap();
    authority.propose(&leader, vec![post(3, "Carol")]).unwrap();
    let snapshot = authority.snapshot(&leader).unwrap();
    let mut follower = Catalog::from_path(&follower_root).unwrap();
    let mut replica = ReplicationLog::new(9).unwrap();

    let mut request = json_request(
        Method::Post,
        "/replication/snapshot",
        snapshot.to_json().unwrap(),
    );
    let response = super::replication::response(
        &mut request,
        "/replication/snapshot",
        &mut follower,
        &mut replica,
    )
    .unwrap();
    assert_eq!(response_json(response)["outcome"], "snapshot_installed");
    assert!(
        follower
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_some()
    );

    let mut request = TestRequest::new()
        .with_method(Method::Get)
        .with_path("/replication/snapshot")
        .into();
    let response = super::replication::response(
        &mut request,
        "/replication/snapshot",
        &mut follower,
        &mut replica,
    )
    .unwrap();
    let exported: ReplicationSnapshot = serde_json::from_value(response_json(response)).unwrap();
    assert_eq!(exported.last_index, 1);
    assert_eq!(exported.last_transaction_id, 1);

    fs::remove_dir_all(leader_root).unwrap();
    fs::remove_dir_all(follower_root).unwrap();
}

#[test]
fn replication_http_rejects_unknown_entry_fields_before_commit() {
    let root = temporary_catalog("invalid");
    let mut catalog = Catalog::from_path(&root).unwrap();
    let mut replica = ReplicationLog::new(1).unwrap();
    let mut entry = json!({
        "version": 1,
        "term": 1,
        "index": 1,
        "transaction_id": 1,
        "schema_tag": "schema-v1",
        "operations": []
    });
    entry["unexpected"] = true.into();
    let mut request = json_request(
        Method::Post,
        "/replication/entry",
        serde_json::to_vec(&entry).unwrap(),
    );
    let response = super::replication::response(
        &mut request,
        "/replication/entry",
        &mut catalog,
        &mut replica,
    )
    .unwrap();
    assert_eq!(response.status_code(), StatusCode(422));
    assert_eq!(catalog.transaction_id().unwrap(), None);

    fs::remove_dir_all(root).unwrap();
}

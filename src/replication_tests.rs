use super::*;
use crate::xbase::OperationMethod;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> Vec<u8> {
    include_str!("../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn catalog_root(label: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "txbase-replication-{label}-{}-{id}",
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

#[path = "replication_tests/batch_delivery.rs"]
mod batch_delivery;
#[path = "replication_tests/compaction.rs"]
mod compaction;
#[path = "replication_tests/log.rs"]
mod log;
#[path = "replication_tests/snapshot.rs"]
mod snapshot;

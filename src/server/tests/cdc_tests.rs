use super::super::cdc;
use super::fixture;
use crate::catalog::Catalog;
use crate::dbf::DbfTable;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use tiny_http::StatusCode;

fn table_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "txbase-server-cdc-{}-{name}.dbf",
        std::process::id()
    ))
}

fn cleanup_table(path: &Path) {
    for extension in [
        "txbase.cdc",
        "txbase.state",
        "txbase.wal",
        "txbase.mvcc",
        "txbase.lock",
    ] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    let _ = fs::remove_file(path);
}

fn response_json(response: super::super::HttpResponse) -> Value {
    let mut body = String::new();
    response.into_reader().read_to_string(&mut body).unwrap();
    serde_json::from_str(&body).unwrap()
}

#[test]
fn table_cdc_http_route_pages_committed_events() {
    let path = table_path("page");
    cleanup_table(&path);
    DbfTable::from_bytes(&fixture())
        .unwrap()
        .save_to(&path)
        .unwrap();

    for name in ["Bob", "Carol"] {
        let mut table = DbfTable::from_path(&path).unwrap();
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": name})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        table.save_with_wal(&path).unwrap();
    }

    let response = cdc::table_response("/cdc?limit=1", &path);
    assert_eq!(response.status_code(), StatusCode(200));
    let body = response_json(response);
    assert_eq!(body["events"].as_array().unwrap().len(), 1);
    assert_eq!(body["events"][0]["transaction_id"], 1);
    assert_eq!(body["next_after"], 1);

    let response = cdc::table_response("/cdc?after=1&limit=2", &path);
    let body = response_json(response);
    assert_eq!(body["events"][0]["transaction_id"], 2);
    assert_eq!(body["next_after"], Value::Null);

    assert_eq!(
        cdc::table_response("/cdc?after=0", &path).status_code(),
        StatusCode(400)
    );
    assert_eq!(
        cdc::table_response("/cdc?limit=1001", &path).status_code(),
        StatusCode(400)
    );
    cleanup_table(&path);
}

#[test]
fn catalog_cdc_http_route_pages_atomic_events() {
    let root =
        std::env::temp_dir().join(format!("txbase-server-catalog-cdc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Post,
                path: "/users/records".into(),
                body: Some(serde_json::json!({
                    "ID": 3,
                    "NAME": "Carol",
                    "AGE": 42,
                    "ACTIVE": true
                })),
            }],
            None,
            None,
        )
        .unwrap();
    catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Patch,
                path: "/posts/records/1".into(),
                body: Some(serde_json::json!({"$inc": {"AGE": 1}})),
            }],
            None,
            None,
        )
        .unwrap();

    let response = cdc::catalog_response("/cdc?limit=1", &catalog);
    assert_eq!(response.status_code(), StatusCode(200));
    let body = response_json(response);
    assert_eq!(body["events"].as_array().unwrap().len(), 1);
    assert!(body["events"][0]["tables"].get("users").is_some());
    assert_eq!(body["next_after"], 1);

    let response = cdc::catalog_response("/cdc?after=1", &catalog);
    let body = response_json(response);
    assert!(body["events"][0]["tables"].get("posts").is_some());
    assert_eq!(body["next_after"], Value::Null);

    let _ = fs::remove_dir_all(root);
}

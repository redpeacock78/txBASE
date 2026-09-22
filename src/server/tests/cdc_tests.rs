use super::super::{JSON_QUERY_MEDIA_TYPE, cdc, header};
use super::fixture;
use crate::catalog::Catalog;
use crate::dbf::DbfTable;
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use tiny_http::{Method, StatusCode, TestRequest};

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

    for table in ["users", "posts"] {
        let body = match table {
            "users" => {
                r#"{"operations":[{"method":"PATCH","path":"/users/records/1","body":{"$inc":{"AGE":1}}}]}"#
            }
            "posts" => {
                r#"{"operations":[{"method":"PATCH","path":"/posts/records/2","body":{"$inc":{"AGE":1}}}]}"#
            }
            _ => unreachable!(),
        };
        let mut request = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/transaction")
            .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
            .with_body(body)
            .into();
        let response = super::super::catalog_transaction::response(&mut request, &catalog);
        assert_eq!(response.status_code(), StatusCode(200));
    }

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

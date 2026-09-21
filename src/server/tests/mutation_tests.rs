use super::super::merge_patch::apply_merge_patch;
use super::super::records::{delete_response, persist_mutation, post_response, update_response};
use super::super::{JSON_MERGE_PATCH_MEDIA_TYPE, JSON_PATCH_MEDIA_TYPE, header, transaction};
use super::{fixture, json_request};
use crate::dbf::DbfTable;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::Value;
use std::fs;
use tiny_http::{Method, StatusCode, TestRequest};

#[test]
fn mutation_endpoints_persist_and_delete_records() {
    let path = std::env::temp_dir().join(format!("txbase-server-test-{}.dbf", std::process::id()));
    let _ = fs::remove_file(&path);
    fs::write(&path, fixture()).unwrap();
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();

    let mut post = json_request(
        Method::Post,
        "/records",
        r#"{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":false}"#,
    );
    assert_eq!(
        post_response(&mut post, "/records", &mut table, &path).status_code(),
        StatusCode(201)
    );

    let mut patch = json_request(Method::Patch, "/records/3", r#"{"$inc":{"AGE":1}}"#);
    assert_eq!(
        update_response(&mut patch, "/records/3", &mut table, &path, false).status_code(),
        StatusCode(200)
    );

    let mut put = json_request(
        Method::Put,
        "/records/3",
        r#"{"ID":3,"NAME":"Carol","AGE":44,"ACTIVE":true}"#,
    );
    assert_eq!(
        update_response(&mut put, "/records/3", &mut table, &path, true).status_code(),
        StatusCode(200)
    );

    let delete = TestRequest::new()
        .with_method(Method::Delete)
        .with_path("/records/3")
        .into();
    assert_eq!(
        delete_response(&delete, "/records/3", &mut table, &path).status_code(),
        StatusCode(204)
    );
    let persisted = DbfTable::from_path(&path).unwrap();
    assert!(persisted.active_record(3).is_none());
    assert!(persisted.records()[2].deleted);
    assert!(!path.with_extension("txbase.wal").exists());
    fs::remove_file(path).unwrap();
}

#[test]
fn merge_patch_updates_known_fields_and_preserves_the_rest() {
    let path = std::env::temp_dir().join(format!(
        "txbase-server-merge-patch-{}.dbf",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    fs::write(&path, fixture()).unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();
    let mut request = TestRequest::new()
        .with_method(Method::Patch)
        .with_path("/records/1")
        .with_header(header("Content-Type", JSON_MERGE_PATCH_MEDIA_TYPE))
        .with_body(r#"{"NAME":"Alicia","AGE":null}"#)
        .into();

    assert_eq!(
        update_response(&mut request, "/records/1", &mut table, &path, false).status_code(),
        StatusCode(200)
    );
    let record = table.active_record(1).unwrap();
    assert_eq!(record.values["NAME"], "Alicia");
    assert_eq!(record.values["AGE"], Value::Null);
    assert_eq!(record.values["ACTIVE"], true);
    assert_eq!(
        DbfTable::from_path(&path)
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["NAME"],
        "Alicia"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn json_patch_updates_and_removes_fields() {
    let path = std::env::temp_dir().join(format!(
        "txbase-server-json-patch-{}.dbf",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    fs::write(&path, fixture()).unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();
    let mut request = TestRequest::new()
        .with_method(Method::Patch)
        .with_path("/records/1")
        .with_header(header("Content-Type", JSON_PATCH_MEDIA_TYPE))
        .with_body(
            r#"[
                {"op":"test","path":"/ID","value":1},
                {"op":"replace","path":"/NAME","value":"Alicia"},
                {"op":"remove","path":"/AGE"},
                {"op":"add","path":"/ACTIVE","value":false}
            ]"#,
        )
        .into();

    assert_eq!(
        update_response(&mut request, "/records/1", &mut table, &path, false).status_code(),
        StatusCode(200)
    );
    let record = table.active_record(1).unwrap();
    assert_eq!(record.values["NAME"], "Alicia");
    assert_eq!(record.values["AGE"], Value::Null);
    assert_eq!(record.values["ACTIVE"], false);
    assert_eq!(
        DbfTable::from_path(&path)
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["NAME"],
        "Alicia"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn json_patch_rejects_invalid_operations_without_mutation() {
    let path = std::env::temp_dir().join(format!(
        "txbase-server-json-patch-invalid-{}.dbf",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    fs::write(&path, fixture()).unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();
    let mut request = TestRequest::new()
        .with_method(Method::Patch)
        .with_path("/records/1")
        .with_header(header("Content-Type", JSON_PATCH_MEDIA_TYPE))
        .with_body(r#"[{"op":"remove","path":"/MISSING"}]"#)
        .into();

    assert_eq!(
        update_response(&mut request, "/records/1", &mut table, &path, false).status_code(),
        StatusCode(422)
    );
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "Alice");
    fs::remove_file(path).unwrap();
}

#[test]
fn merge_patch_recursively_merges_object_members() {
    let current = serde_json::json!({
        "NESTED": {"KEEP": 1, "REMOVE": 2},
        "ACTIVE": true
    });
    let patch = serde_json::json!({
        "NESTED": {"KEEP": 3, "REMOVE": null}
    });
    let result = apply_merge_patch(
        current.as_object().unwrap().clone(),
        patch.as_object().unwrap().clone(),
    );

    assert_eq!(
        result,
        serde_json::json!({
            "NESTED": {"KEEP": 3},
            "ACTIVE": true
        })
        .as_object()
        .unwrap()
        .clone()
    );
}

#[test]
fn merge_patch_requires_an_object_root_and_patch_media_type() {
    let path = std::env::temp_dir().join(format!(
        "txbase-server-merge-patch-boundary-{}.dbf",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    fs::write(&path, fixture()).unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();

    let mut scalar_root = TestRequest::new()
        .with_method(Method::Patch)
        .with_path("/records/1")
        .with_header(header("Content-Type", JSON_MERGE_PATCH_MEDIA_TYPE))
        .with_body("null")
        .into();
    assert_eq!(
        update_response(&mut scalar_root, "/records/1", &mut table, &path, false).status_code(),
        StatusCode(422)
    );
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "Alice");

    let mut post = TestRequest::new()
        .with_method(Method::Post)
        .with_path("/records")
        .with_header(header("Content-Type", JSON_MERGE_PATCH_MEDIA_TYPE))
        .with_body(r#"{"NAME":"never inserted"}"#)
        .into();
    assert_eq!(
        post_response(&mut post, "/records", &mut table, &path).status_code(),
        StatusCode(415)
    );

    fs::remove_file(path).unwrap();
}

#[test]
fn transaction_endpoint_commits_multiple_mutations_once() {
    let path = std::env::temp_dir().join(format!(
        "txbase-server-transaction-{}.dbf",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    fs::write(&path, fixture()).unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();
    let mut request = json_request(
        Method::Post,
        "/transaction",
        r#"{
            "operations": [
                {"method":"POST","path":"/records","body":{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":true}},
                {"method":"PATCH","path":"/records/3","body":{"$inc":{"AGE":1}}}
            ]
        }"#,
    );

    let response = transaction::response(&mut request, &mut table, &path);
    assert_eq!(response.status_code(), StatusCode(200));
    let persisted = DbfTable::from_path(&path).unwrap();
    assert_eq!(persisted.active_record(3).unwrap().values["AGE"], 43);
    assert!(!path.with_extension("txbase.wal").exists());
    fs::remove_file(path).unwrap();
}

#[test]
fn transaction_endpoint_discards_all_mutations_when_one_fails() {
    let path = std::env::temp_dir().join(format!(
        "txbase-server-transaction-rollback-{}.dbf",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    fs::write(&path, fixture()).unwrap();
    let before = fs::read(&path).unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();
    let mut request = json_request(
        Method::Post,
        "/transaction",
        r#"{
            "operations": [
                {"method":"POST","path":"/records","body":{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":true}},
                {"method":"PATCH","path":"/records/999","body":{"NAME":"never committed"}}
            ]
        }"#,
    );

    let response = transaction::response(&mut request, &mut table, &path);
    assert_eq!(response.status_code(), StatusCode(404));
    assert!(table.active_record(3).is_none());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_file(path).unwrap();
}

#[test]
fn reloads_disk_state_after_persistence_failure() {
    let path =
        std::env::temp_dir().join(format!("txbase-server-reload-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("dbt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = fixture();
    bytes[0] = 0x83;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let memo_start = record_start + 4;
    bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
    fs::write(&path, &bytes).unwrap();

    let mut memo = vec![0; 512 * 2];
    memo[512..524].copy_from_slice(b"memo before\x1a");
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "memo after"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let original = DbfTable::from_path(&path).unwrap();

    fs::remove_file(&memo_path).unwrap();
    let age_start = record_start + 14;
    bytes[age_start..age_start + 3].copy_from_slice(b" 30");
    fs::write(&path, bytes).unwrap();

    let operation = OperationIr {
        method: OperationMethod::Patch,
        path: "/records/1".into(),
        body: None,
    };
    let response = persist_mutation(&mut table, original, &path, &operation).unwrap_err();
    assert_eq!(response.status_code(), StatusCode(500));
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);

    fs::remove_file(path).unwrap();
}

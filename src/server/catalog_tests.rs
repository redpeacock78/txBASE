use super::*;
use crate::index::{IndexDefinition, IndexFile};
use std::fs;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use tiny_http::{Method, StatusCode, TestRequest};

static NEXT_CATALOG_ID: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn foreign_key_metadata() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {
            "ID": {"references": "users.ID"}
        }
    }))
    .unwrap()
}

fn temporary_catalog() -> std::path::PathBuf {
    let id = NEXT_CATALOG_ID.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("txbase-server-catalog-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir(&path).unwrap();
    path
}

fn json_request(method: Method, path: &str, body: &'static str) -> Request {
    TestRequest::new()
        .with_method(method)
        .with_path(path)
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_body(body)
        .into()
}

#[test]
fn catalog_server_query_join_executes_and_exposes_schema() {
    let root = temporary_catalog();
    fs::write(root.join("left.dbf"), fixture()).unwrap();
    fs::write(root.join("right.dbf"), fixture()).unwrap();
    let catalog = crate::catalog::Catalog::from_path(&root).unwrap();

    let schema_request = TestRequest::new()
        .with_method(Method::Get)
        .with_path("/catalog")
        .into();
    let response = super::catalog::schema_response(&schema_request, &catalog);
    assert_eq!(response.status_code(), StatusCode(200));
    let mut schema_body = String::new();
    response
        .into_reader()
        .read_to_string(&mut schema_body)
        .unwrap();
    assert!(schema_body.contains("txbase-catalog"));

    let mut join_request = TestRequest::new()
        .with_method("QUERY".parse().unwrap())
        .with_path("/join")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_body(
            r#"{"from":"left","join":{"type":"inner","table":"right","on":{"left.ID":{"$eq":{"$field":"right.ID"}}}},"projection":{"left.NAME":1,"right.NAME":1}}"#,
        )
        .into();
    let response = super::catalog::join_response(&mut join_request, &catalog);
    assert_eq!(response.status_code(), StatusCode(200));
    let mut body = String::new();
    response.into_reader().read_to_string(&mut body).unwrap();
    assert!(body.contains("left.NAME"));
    assert!(body.contains("right.NAME"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_server_reads_named_tables_through_record_routes() {
    let root = temporary_catalog();
    let left_path = root.join("left.dbf");
    fs::write(&left_path, fixture()).unwrap();
    IndexFile::build(&left_path, vec![IndexDefinition::named("by_age", "AGE")])
        .unwrap()
        .save(&left_path)
        .unwrap();
    let catalog = crate::catalog::Catalog::from_path(&root).unwrap();

    let get = TestRequest::new()
        .with_method(Method::Get)
        .with_path("/left/records")
        .into();
    let response = super::catalog::table_response(&get, "/left/records", &catalog);
    assert_eq!(response.status_code(), StatusCode(200));
    let mut body = String::new();
    response.into_reader().read_to_string(&mut body).unwrap();
    assert!(body.contains("Alice"));

    let mut stream_query = TestRequest::new()
        .with_method("QUERY".parse().unwrap())
        .with_path("/left/records/stream")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_body(r#"{"projection":{"NAME":1},"limit":1}"#)
        .into();
    let response =
        super::catalog::table_query_response(&mut stream_query, "/left/records/stream", &catalog);
    assert_eq!(response.status_code(), StatusCode(200));
    assert_eq!(
        response
            .headers()
            .iter()
            .find(|header| header.field.equiv("Content-Type"))
            .map(|header| header.value.as_str()),
        Some("application/x-ndjson")
    );
    let mut stream_body = String::new();
    response
        .into_reader()
        .read_to_string(&mut stream_body)
        .unwrap();
    assert_eq!(stream_body.lines().count(), 1);

    let get_record = TestRequest::new()
        .with_method(Method::Get)
        .with_path("/left/records/1")
        .into();
    assert_eq!(
        super::catalog::table_response(&get_record, "/left/records/1", &catalog).status_code(),
        StatusCode(200)
    );

    let mut query = TestRequest::new()
        .with_method("QUERY".parse().unwrap())
        .with_path("/left/records")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_body(r#"{"filter":{"AGE":{"$gte":20}}}"#)
        .into();
    let response = super::catalog::table_query_response(&mut query, "/left/records", &catalog);
    assert_eq!(response.status_code(), StatusCode(200));

    let mut explain = TestRequest::new()
        .with_method("QUERY".parse().unwrap())
        .with_path("/left/explain")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_body(r#"{"filter":{"AGE":7}}"#)
        .into();
    let response = super::catalog::table_explain_response(&mut explain, "/left/explain", &catalog);
    assert_eq!(response.status_code(), StatusCode(200));
    let mut explain_body = String::new();
    response
        .into_reader()
        .read_to_string(&mut explain_body)
        .unwrap();
    assert!(explain_body.contains(r#""kind":"equality_index""#));
    assert!(explain_body.contains("by_age"));

    let missing = TestRequest::new()
        .with_method(Method::Get)
        .with_path("/missing/records")
        .into();
    assert_eq!(
        super::catalog::table_response(&missing, "/missing/records", &catalog).status_code(),
        StatusCode(404)
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_server_mutates_named_tables_with_single_table_semantics() {
    let root = temporary_catalog();
    fs::write(root.join("left.dbf"), fixture()).unwrap();
    let catalog = crate::catalog::Catalog::from_path(&root).unwrap();

    let mut post = json_request(
        Method::Post,
        "/left/records",
        r#"{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":true}"#,
    );
    let response = super::catalog::table_mutation_response(&mut post, "/left/records", &catalog);
    assert_eq!(response.status_code(), StatusCode(201));
    assert_eq!(
        response
            .headers()
            .iter()
            .find(|header| header.field.equiv("Location"))
            .map(|header| header.value.as_str()),
        Some("/left/records/3")
    );
    assert!(
        catalog
            .open_table("left")
            .unwrap()
            .active_record(3)
            .is_some()
    );

    let mut patch = json_request(Method::Patch, "/left/records/3", r#"{"$inc":{"AGE":1}}"#);
    let response = super::catalog::table_mutation_response(&mut patch, "/left/records/3", &catalog);
    assert_eq!(response.status_code(), StatusCode(200));
    assert_eq!(
        catalog
            .open_table("left")
            .unwrap()
            .active_record(3)
            .unwrap()
            .values["AGE"],
        43
    );

    let mut delete = TestRequest::new()
        .with_method(Method::Delete)
        .with_path("/left/records/3")
        .into();
    let response =
        super::catalog::table_mutation_response(&mut delete, "/left/records/3", &catalog);
    assert_eq!(response.status_code(), StatusCode(204));
    assert!(
        catalog
            .open_table("left")
            .unwrap()
            .active_record(3)
            .is_none()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_server_rejects_orphan_foreign_key_mutations() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.txschema.json"), foreign_key_metadata()).unwrap();
    let catalog = crate::catalog::Catalog::from_path(&root).unwrap();

    let mut post = json_request(
        Method::Post,
        "/posts/records",
        r#"{"ID":3,"NAME":"Orphan","AGE":42,"ACTIVE":true}"#,
    );
    let response = super::catalog::table_mutation_response(&mut post, "/posts/records", &catalog);
    assert_eq!(response.status_code(), StatusCode(422));
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(3)
            .is_none()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_server_transaction_commits_multiple_named_tables() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    let catalog = crate::catalog::Catalog::from_path(&root).unwrap();
    let mut request = json_request(
        Method::Post,
        "/transaction",
        r#"{
            "operations": [
                {"method":"POST","path":"/users/records","body":{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":true}},
                {"method":"PATCH","path":"/posts/records/1","body":{"$inc":{"AGE":1}}}
            ]
        }"#,
    );

    let response = super::catalog_transaction::response(&mut request, &catalog);
    assert_eq!(response.status_code(), StatusCode(200));
    assert_eq!(
        response
            .headers()
            .iter()
            .find(|header| header.field.equiv("X-Txbase-Transaction-Id"))
            .map(|header| header.value.as_str()),
        Some("1")
    );
    let mut body = String::new();
    response.into_reader().read_to_string(&mut body).unwrap();
    assert!(body.contains(r#""transaction_id":1"#));
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_some()
    );
    assert_eq!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["AGE"],
        30
    );
    assert!(!root.join(".txbase.catalog.txn").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_etag_guards_schema_reads_and_transactions() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    let catalog = crate::catalog::Catalog::from_path(&root).unwrap();

    let schema_request = TestRequest::new()
        .with_method(Method::Get)
        .with_path("/catalog")
        .into();
    let response = super::catalog::schema_response(&schema_request, &catalog);
    assert_eq!(response.status_code(), StatusCode(200));
    let tag = response
        .headers()
        .iter()
        .find(|header| header.field.equiv("ETag"))
        .map(|header| header.value.as_str().to_owned())
        .expect("catalog schema ETag");

    let conditional_request = TestRequest::new()
        .with_method(Method::Get)
        .with_path("/catalog")
        .with_header(header("If-None-Match", &format!("W/{tag}")))
        .into();
    let response = super::catalog::schema_response(&conditional_request, &catalog);
    assert_eq!(response.status_code(), StatusCode(304));
    assert!(response.into_reader().into_inner().is_empty());

    let before = fs::read(root.join("users.dbf")).unwrap();
    let body = r#"{"operations":[{"method":"PATCH","path":"/users/records/1","body":{"$inc":{"AGE":1}}}]}"#;
    let mut stale_match = TestRequest::new()
        .with_method(Method::Post)
        .with_path("/transaction")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_header(header("If-Match", "\"stale\""))
        .with_body(body)
        .into();
    let response = super::catalog_transaction::response(&mut stale_match, &catalog);
    assert_eq!(response.status_code(), StatusCode(412));
    assert_eq!(fs::read(root.join("users.dbf")).unwrap(), before);

    let mut matching = TestRequest::new()
        .with_method(Method::Post)
        .with_path("/transaction")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_header(header("If-None-Match", &tag))
        .with_body(body)
        .into();
    let response = super::catalog_transaction::response(&mut matching, &catalog);
    assert_eq!(response.status_code(), StatusCode(412));
    assert_eq!(
        response
            .headers()
            .iter()
            .find(|header| header.field.equiv("ETag"))
            .map(|header| header.value.as_str()),
        Some(tag.as_str())
    );
    assert_eq!(fs::read(root.join("users.dbf")).unwrap(), before);

    let mut stale = TestRequest::new()
        .with_method(Method::Post)
        .with_path("/transaction")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_header(header("If-None-Match", "\"stale\""))
        .with_body(body)
        .into();
    let response = super::catalog_transaction::response(&mut stale, &catalog);
    assert_eq!(response.status_code(), StatusCode(200));
    let next_tag = response
        .headers()
        .iter()
        .find(|header| header.field.equiv("ETag"))
        .map(|header| header.value.as_str().to_owned())
        .expect("committed catalog ETag");
    assert_ne!(next_tag, tag);
    assert_eq!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["AGE"],
        30
    );

    let mut current_match = TestRequest::new()
        .with_method(Method::Post)
        .with_path("/transaction")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_header(header("If-Match", &next_tag))
        .with_body(body)
        .into();
    let response = super::catalog_transaction::response(&mut current_match, &catalog);
    assert_eq!(response.status_code(), StatusCode(200));
    assert_ne!(
        response
            .headers()
            .iter()
            .find(|header| header.field.equiv("ETag"))
            .map(|header| header.value.as_str()),
        Some(next_tag.as_str())
    );
    assert_eq!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["AGE"],
        31
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_server_transaction_rolls_back_when_a_named_operation_fails() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    let catalog = crate::catalog::Catalog::from_path(&root).unwrap();
    let mut request = json_request(
        Method::Post,
        "/transaction",
        r#"{
            "operations": [
                {"method":"POST","path":"/users/records","body":{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":true}},
                {"method":"PATCH","path":"/posts/records/999","body":{"NAME":"never committed"}}
            ]
        }"#,
    );

    let response = super::catalog_transaction::response(&mut request, &catalog);
    assert_eq!(response.status_code(), StatusCode(422));
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_none()
    );
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(1)
            .is_some()
    );
    fs::remove_dir_all(root).unwrap();
}

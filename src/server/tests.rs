use super::*;
use crate::index::{IndexDefinition, IndexFile};
use crate::xbase::{OperationIr, OperationMethod};
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

fn query_request(body: &'static str, content_type: Option<&'static str>) -> Request {
    let request = TestRequest::new()
        .with_method("QUERY".parse().unwrap())
        .with_path("/records")
        .with_body(body);
    match content_type {
        Some(content_type) => request
            .with_header(header("Content-Type", content_type))
            .into(),
        None => request.into(),
    }
}

fn ranged_query_request(body: &'static str, range: &'static str) -> Request {
    TestRequest::new()
        .with_method("QUERY".parse().unwrap())
        .with_path("/records")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_header(header("Range", range))
        .with_body(body)
        .into()
}

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

    let mut delete = TestRequest::new()
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

    let response = super::transaction::response(&mut request, &mut table, &path);
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

    let response = super::transaction::response(&mut request, &mut table, &path);
    assert_eq!(response.status_code(), StatusCode(404));
    assert!(table.active_record(3).is_none());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_file(path).unwrap();
}

#[test]
fn query_endpoint_enforces_json_boundary_and_executes() {
    let table = DbfTable::from_bytes(&fixture()).unwrap();

    let mut missing_content_type = query_request("{}", None);
    let response = query_response(&mut missing_content_type, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(400));

    let mut unsupported_content_type = query_request("{}", Some("text/plain"));
    let response = query_response(&mut unsupported_content_type, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(415));

    let mut invalid_json = query_request("{", Some(JSON_QUERY_MEDIA_TYPE));
    let response = query_response(&mut invalid_json, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(422));

    let mut unknown_field = query_request(r#"{"filtre":{"AGE":29}}"#, Some(JSON_QUERY_MEDIA_TYPE));
    let response = query_response(&mut unknown_field, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(422));

    let mut valid = query_request(
        r#"{"filter":{"AGE":{"$gte":20}}}"#,
        Some("application/json; charset=utf-8"),
    );
    let response = query_response(&mut valid, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(200));
}

#[test]
fn catalog_server_query_join_executes_and_exposes_schema() {
    let root = temporary_catalog();
    fs::write(root.join("left.dbf"), fixture()).unwrap();
    fs::write(root.join("right.dbf"), fixture()).unwrap();
    let catalog = crate::catalog::Catalog::from_path(&root).unwrap();

    let response = super::catalog::schema_response(&catalog);
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
    fs::write(root.join("left.dbf"), fixture()).unwrap();
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
    assert!(explain_body.contains("table_scan"));

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

    let delete = TestRequest::new()
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
fn explain_endpoint_reports_scan_and_index_plans() {
    let path =
        std::env::temp_dir().join(format!("txbase-server-explain-{}.dbf", std::process::id()));
    let sidecar = crate::index::sidecar_path(&path);
    let lock = path.with_extension("txbase.lock");
    let wal = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&sidecar);
    let _ = fs::remove_file(&lock);
    let _ = fs::remove_file(&wal);
    fs::write(&path, fixture()).unwrap();

    let mut scan_request = TestRequest::new()
        .with_method("QUERY".parse().unwrap())
        .with_path("/explain")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_body(r#"{"filter":{"NAME":"Alice"}}"#)
        .into();
    let response = super::explain::response(&mut scan_request, &path);
    assert_eq!(response.status_code(), StatusCode(200));
    let mut scan_body = String::new();
    response
        .into_reader()
        .read_to_string(&mut scan_body)
        .unwrap();
    assert!(scan_body.contains(r#""kind":"table_scan""#));

    IndexFile::build(&path, vec![IndexDefinition::named("by_name", "NAME")])
        .unwrap()
        .save(&path)
        .unwrap();
    let mut index_request = TestRequest::new()
        .with_method("QUERY".parse().unwrap())
        .with_path("/explain")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_body(r#"{"filter":{"NAME":"Alice"}}"#)
        .into();
    let response = super::explain::response(&mut index_request, &path);
    assert_eq!(response.status_code(), StatusCode(200));
    let mut index_body = String::new();
    response
        .into_reader()
        .read_to_string(&mut index_body)
        .unwrap();
    assert!(index_body.contains(r#""kind":"equality_index""#));
    assert!(index_body.contains("by_name"));

    let _ = fs::remove_file(path);
    let _ = fs::remove_file(sidecar);
    let _ = fs::remove_file(lock);
    let _ = fs::remove_file(wal);
}

#[test]
fn query_endpoint_returns_cursor_pages() {
    let table = DbfTable::from_bytes(&fixture()).unwrap();
    let mut first = query_request(r#"{"page_size":1}"#, Some(JSON_QUERY_MEDIA_TYPE));
    let response = query_response(&mut first, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(200));
    let mut body = String::new();
    response.into_reader().read_to_string(&mut body).unwrap();
    assert!(body.contains(r#""records""#));
    assert!(body.contains(r#""cursor":null"#));
}

#[test]
fn query_endpoint_handles_single_byte_ranges() {
    let table = DbfTable::from_bytes(&fixture()).unwrap();
    let full = Value::Array(table.active_json()).to_string().into_bytes();

    let mut first = ranged_query_request("{}", "bytes=0-9");
    let response = query_response(&mut first, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(206));
    assert_eq!(
        response
            .headers()
            .iter()
            .find(|header| header.field.equiv("Content-Range"))
            .map(|header| header.value.as_str()),
        Some(format!("bytes 0-9/{}", full.len()).as_str())
    );
    assert_eq!(response.into_reader().into_inner(), full[..10]);

    let mut suffix = ranged_query_request("{}", "bytes=-5");
    let response = query_response(&mut suffix, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(206));
    assert_eq!(response.into_reader().into_inner(), full[full.len() - 5..]);

    let mut unsatisfiable = ranged_query_request("{}", "bytes=999-");
    let response = query_response(&mut unsatisfiable, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(416));
    assert_eq!(
        response
            .headers()
            .iter()
            .find(|header| header.field.equiv("Content-Range"))
            .map(|header| header.value.as_str()),
        Some(format!("bytes */{}", full.len()).as_str())
    );

    let mut multiple = ranged_query_request("{}", "bytes=0-1,3-4");
    let response = query_response(&mut multiple, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(200));
    assert_eq!(response.into_reader().into_inner(), full);
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
    let response =
        super::records::persist_mutation(&mut table, original, &path, &operation).unwrap_err();
    assert_eq!(response.status_code(), StatusCode(500));
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);

    fs::remove_file(path).unwrap();
}

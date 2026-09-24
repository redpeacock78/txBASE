use super::super::{JSON_QUERY_MEDIA_TYPE, explain, header, options_response, query_response};
use super::{fixture, query_request, ranged_query_request, stream_query_request};
use crate::dbf::DbfTable;
use crate::index::{IndexDefinition, IndexFile};
use serde_json::Value;
use std::fs;
use std::io::Read;
use tiny_http::{HTTPVersion, StatusCode, TestRequest};

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
fn query_endpoint_rejects_an_oversized_json_body_before_parsing() {
    let body = Box::leak(" ".repeat(crate::MAX_JSON_INPUT_BYTES + 1).into_boxed_str());
    let mut request = query_request(body, Some(JSON_QUERY_MEDIA_TYPE));
    let table = DbfTable::from_bytes(&fixture()).unwrap();
    let response = query_response(&mut request, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(413));
}

#[test]
fn query_stream_endpoint_returns_chunked_ndjson() {
    let table = DbfTable::from_bytes(&fixture()).unwrap();
    let mut request =
        stream_query_request(r#"{"filter":{"NAME":"Alice"},"projection":{"NAME":1}}"#);
    let response = query_response(&mut request, "/records/stream", &table);
    assert_eq!(response.status_code(), StatusCode(200));
    assert_eq!(response.data_length(), None);
    assert_eq!(
        response
            .headers()
            .iter()
            .find(|header| header.field.equiv("Content-Type"))
            .map(|header| header.value.as_str()),
        Some("application/x-ndjson")
    );

    let mut wire = Vec::new();
    response
        .raw_print(&mut wire, HTTPVersion(1, 1), &[], false, None)
        .unwrap();
    let wire = String::from_utf8(wire).unwrap();
    assert!(wire.contains("Transfer-Encoding: chunked\r\n"));
    assert!(wire.ends_with("{\"NAME\":\"Alice\"}\n\r\n0\r\n\r\n"));
}

#[test]
fn query_stream_endpoint_rejects_blocking_controls_before_streaming() {
    let table = DbfTable::from_bytes(&fixture()).unwrap();
    let mut request = stream_query_request(r#"{"sort":{"AGE":1}}"#);
    let response = query_response(&mut request, "/records/stream", &table);
    assert_eq!(response.status_code(), StatusCode(422));
}

#[test]
fn options_advertises_supported_methods_and_query_media_type() {
    let response = options_response("GET, HEAD, OPTIONS, POST, PUT, PATCH, DELETE, QUERY");
    assert_eq!(response.status_code(), StatusCode(204));
    assert_eq!(
        response
            .headers()
            .iter()
            .find(|header| header.field.equiv("Allow"))
            .map(|header| header.value.as_str()),
        Some("GET, HEAD, OPTIONS, POST, PUT, PATCH, DELETE, QUERY")
    );
    assert_eq!(
        response
            .headers()
            .iter()
            .find(|header| header.field.equiv("Accept-Query"))
            .map(|header| header.value.as_str()),
        Some("\"application/json\"")
    );
    assert_eq!(
        response
            .headers()
            .iter()
            .find(|header| header.field.equiv("Accept-Patch"))
            .map(|header| header.value.as_str()),
        Some("application/json, application/merge-patch+json, application/json-patch+json")
    );
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
    let mut bytes = fixture();
    bytes[179] = b' ';
    fs::write(&path, bytes).unwrap();

    let mut scan_request = TestRequest::new()
        .with_method("QUERY".parse().unwrap())
        .with_path("/explain")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_body(r#"{"filter":{"NAME":"Alice"}}"#)
        .into();
    let response = explain::response(&mut scan_request, &path);
    assert_eq!(response.status_code(), StatusCode(200));
    let mut scan_body = String::new();
    response
        .into_reader()
        .read_to_string(&mut scan_body)
        .unwrap();
    assert!(scan_body.contains(r#""kind":"table_scan""#));
    assert!(!scan_body.contains(r#""cost""#));

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
    let response = explain::response(&mut index_request, &path);
    assert_eq!(response.status_code(), StatusCode(200));
    let mut index_body = String::new();
    response
        .into_reader()
        .read_to_string(&mut index_body)
        .unwrap();
    assert!(index_body.contains(r#""kind":"equality_index""#));
    assert!(index_body.contains("by_name"));
    assert!(index_body.contains(r#""candidate_rows":1"#));
    assert!(index_body.contains(r#""index_traversal""#));
    assert!(index_body.contains("index_page_reads"));
    assert!(index_body.contains(r#""record_reads":1"#));
    assert!(index_body.contains("record_page_reads"));
    assert!(index_body.contains(r#""filter_evaluations":1"#));
    assert!(index_body.contains(r#""sort_work":0"#));
    assert!(index_body.contains(r#""total""#));

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

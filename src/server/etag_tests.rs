use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use tiny_http::{Method, Request, StatusCode, TestRequest};

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn test_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("txbase-etag-{name}-{}.dbf", std::process::id()))
}

fn prepare(path: &Path) -> DbfTable {
    let _ = fs::remove_file(path);
    fs::write(path, fixture()).unwrap();
    DbfTable::from_path(path).unwrap()
}

fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("txbase.wal"));
    let _ = fs::remove_file(path.with_extension("txbase.lock"));
}

fn header_value(response: &super::HttpResponse, name: &str) -> String {
    response
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.as_str().to_owned())
        .expect("response header is present")
}

fn patch_request(tag: &str) -> Request {
    TestRequest::new()
        .with_method(Method::Patch)
        .with_path("/records/1")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_header(header("If-Match", tag))
        .with_body(r#"{"$inc":{"AGE":1}}"#)
        .into()
}

fn transaction_request(tag: &str) -> Request {
    TestRequest::new()
        .with_method(Method::Post)
        .with_path("/transaction")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_header(header("If-Match", tag))
        .with_body(
            r#"{"operations":[{"method":"PATCH","path":"/records/1","body":{"$inc":{"AGE":1}}}]}"#,
        )
        .into()
}

#[test]
fn mutation_etag_prevents_lost_update() {
    let path = test_path("mutation");
    let mut table = prepare(&path);
    let response = get_response("/records", &table);
    assert_eq!(response.status_code(), StatusCode(200));
    let current = header_value(&response, "ETag");
    let before = fs::read(&path).unwrap();

    let mut stale = patch_request("\"stale\"");
    let response = update_response(&mut stale, "/records/1", &mut table, &path, false);
    assert_eq!(response.status_code(), StatusCode(412));
    assert_eq!(header_value(&response, "ETag"), current);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 29);

    let mut matching = patch_request(&current);
    let response = update_response(&mut matching, "/records/1", &mut table, &path, false);
    assert_eq!(response.status_code(), StatusCode(200));
    assert_ne!(header_value(&response, "ETag"), current);
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);

    cleanup(&path);
}

#[test]
fn if_match_requires_strong_tags_and_supports_existing_wildcard() {
    let path = test_path("matching");
    let mut table = prepare(&path);
    let current = header_value(&get_response("/records", &table), "ETag");

    let mut weak = patch_request(&format!("W/{current}"));
    let response = update_response(&mut weak, "/records/1", &mut table, &path, false);
    assert_eq!(response.status_code(), StatusCode(412));

    let mut wildcard = patch_request("*");
    let response = update_response(&mut wildcard, "/records/1", &mut table, &path, false);
    assert_eq!(response.status_code(), StatusCode(200));
    let current = header_value(&response, "ETag");

    let list = format!("\"stale\", {current}");
    let mut matching_list = patch_request(&list);
    let response = update_response(&mut matching_list, "/records/1", &mut table, &path, false);
    assert_eq!(response.status_code(), StatusCode(200));

    let current = header_value(&response, "ETag");
    let mut mixed_wildcard = patch_request(&format!("*, {current}"));
    let response = update_response(&mut mixed_wildcard, "/records/1", &mut table, &path, false);
    assert_eq!(response.status_code(), StatusCode(412));
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 31);

    cleanup(&path);
}

#[test]
fn transaction_etag_precondition_is_atomic() {
    let path = test_path("transaction");
    let mut table = prepare(&path);
    let current = header_value(&get_response("/records", &table), "ETag");
    let before = fs::read(&path).unwrap();

    let mut stale = transaction_request("\"stale\"");
    let response = super::transaction::response(&mut stale, &mut table, &path);
    assert_eq!(response.status_code(), StatusCode(412));
    assert_eq!(fs::read(&path).unwrap(), before);

    let mut matching = transaction_request(&current);
    let response = super::transaction::response(&mut matching, &mut table, &path);
    assert_eq!(response.status_code(), StatusCode(200));
    assert_ne!(header_value(&response, "ETag"), current);
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);

    cleanup(&path);
}

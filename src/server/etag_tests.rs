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
    let _ = fs::remove_file(path.with_extension("txbase.state"));
}

fn header_value(response: &super::HttpResponse, name: &'static str) -> String {
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

fn patch_if_none_match_request(tag: &str) -> Request {
    TestRequest::new()
        .with_method(Method::Patch)
        .with_path("/records/1")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_header(header("If-None-Match", tag))
        .with_body(r#"{"$inc":{"AGE":1}}"#)
        .into()
}

fn post_request(tag: &str) -> Request {
    TestRequest::new()
        .with_method(Method::Post)
        .with_path("/records")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_header(header("If-Match", tag))
        .with_body(r#"{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":true}"#)
        .into()
}

fn put_request(tag: &str) -> Request {
    TestRequest::new()
        .with_method(Method::Put)
        .with_path("/records/3")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_header(header("If-Match", tag))
        .with_body(r#"{"ID":3,"NAME":"Caroline","AGE":43,"ACTIVE":true}"#)
        .into()
}

fn delete_request(tag: &str) -> Request {
    TestRequest::new()
        .with_method(Method::Delete)
        .with_path("/records/3")
        .with_header(header("If-Match", tag))
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

fn transaction_if_none_match_request(tag: &str) -> Request {
    TestRequest::new()
        .with_method(Method::Post)
        .with_path("/transaction")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_header(header("If-None-Match", tag))
        .with_body(
            r#"{"operations":[{"method":"PATCH","path":"/records/1","body":{"$inc":{"AGE":1}}}]}"#,
        )
        .into()
}

fn get_request(path: &str, tag: Option<&str>) -> Request {
    let request = TestRequest::new().with_method(Method::Get).with_path(path);
    match tag {
        Some(tag) => request.with_header(header("If-None-Match", tag)).into(),
        None => request.into(),
    }
}

#[test]
fn mutation_etag_prevents_lost_update() {
    let path = test_path("mutation");
    let mut table = prepare(&path);
    let request = get_request("/records", None);
    let response = get_response(&request, "/records", &table);
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
    assert_eq!(header_value(&response, "X-Txbase-Transaction-Id"), "1");
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);

    cleanup(&path);
}

#[test]
fn post_put_and_delete_honor_current_etag() {
    let path = test_path("all-mutations");
    let mut table = prepare(&path);
    let request = get_request("/records", None);
    let current = header_value(&get_response(&request, "/records", &table), "ETag");

    let mut post = post_request(&current);
    let response = post_response(&mut post, "/records", &mut table, &path);
    assert_eq!(response.status_code(), StatusCode(201));
    let current = header_value(&response, "ETag");

    let mut put = put_request(&current);
    let response = update_response(&mut put, "/records/3", &mut table, &path, true);
    assert_eq!(response.status_code(), StatusCode(200));
    let current = header_value(&response, "ETag");

    let stale_delete = delete_request("\"stale\"");
    let response = delete_response(&stale_delete, "/records/3", &mut table, &path);
    assert_eq!(response.status_code(), StatusCode(412));
    assert!(table.active_record(3).is_some());

    let delete = delete_request(&current);
    let response = delete_response(&delete, "/records/3", &mut table, &path);
    assert_eq!(response.status_code(), StatusCode(204));
    assert!(table.active_record(3).is_none());

    cleanup(&path);
}

#[test]
fn if_match_requires_strong_tags_and_supports_existing_wildcard() {
    let path = test_path("matching");
    let mut table = prepare(&path);
    let request = get_request("/records", None);
    let current = header_value(&get_response(&request, "/records", &table), "ETag");

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
    let request = get_request("/records", None);
    let current = header_value(&get_response(&request, "/records", &table), "ETag");
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

#[test]
fn mutation_if_none_match_rejects_current_representation_without_writing() {
    let path = test_path("none-match-mutation");
    let mut table = prepare(&path);
    let request = get_request("/records", None);
    let current = header_value(&get_response(&request, "/records", &table), "ETag");
    let before = fs::read(&path).unwrap();

    let mut matching = patch_if_none_match_request(&current);
    let response = update_response(&mut matching, "/records/1", &mut table, &path, false);
    assert_eq!(response.status_code(), StatusCode(412));
    assert_eq!(header_value(&response, "ETag"), current);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 29);

    let mut weak = patch_if_none_match_request(&format!("W/{current}"));
    assert_eq!(
        update_response(&mut weak, "/records/1", &mut table, &path, false).status_code(),
        StatusCode(412)
    );

    let mut wildcard = patch_if_none_match_request("*");
    assert_eq!(
        update_response(&mut wildcard, "/records/1", &mut table, &path, false).status_code(),
        StatusCode(412)
    );

    let mut stale = patch_if_none_match_request("\"stale\"");
    assert_eq!(
        update_response(&mut stale, "/records/1", &mut table, &path, false).status_code(),
        StatusCode(200)
    );
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);

    cleanup(&path);
}

#[test]
fn transaction_if_none_match_rejects_current_representation_atomically() {
    let path = test_path("none-match-transaction");
    let mut table = prepare(&path);
    let request = get_request("/records", None);
    let current = header_value(&get_response(&request, "/records", &table), "ETag");
    let before = fs::read(&path).unwrap();

    let mut request = transaction_if_none_match_request(&current);
    let response = super::transaction::response(&mut request, &mut table, &path);
    assert_eq!(response.status_code(), StatusCode(412));
    assert_eq!(header_value(&response, "ETag"), current);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 29);

    cleanup(&path);
}

#[test]
fn if_none_match_returns_not_modified_for_current_representation() {
    let path = test_path("cache");
    let table = prepare(&path);
    let request = get_request("/records", None);
    let current = header_value(&get_response(&request, "/records", &table), "ETag");

    let request = get_request("/records", Some(&current));
    let response = get_response(&request, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(304));
    assert_eq!(header_value(&response, "ETag"), current);
    assert!(response.into_reader().into_inner().is_empty());

    let weak = format!("W/{current}");
    let request = get_request("/records/1", Some(&weak));
    let response = get_response(&request, "/records/1", &table);
    assert_eq!(response.status_code(), StatusCode(304));

    let request = get_request("/records", Some("\"stale\""));
    let response = get_response(&request, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(200));

    cleanup(&path);
}

#[test]
fn head_reuses_record_headers_and_conditional_status() {
    let path = test_path("head");
    let table = prepare(&path);
    let request = get_request("/records", None);
    let current = header_value(&get_response(&request, "/records", &table), "ETag");

    let request = TestRequest::new()
        .with_method(Method::Head)
        .with_path("/records")
        .into();
    let response = get_response(&request, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(200));
    assert_eq!(header_value(&response, "ETag"), current);

    let request = TestRequest::new()
        .with_method(Method::Head)
        .with_path("/records")
        .with_header(header("If-None-Match", &current))
        .into();
    let response = get_response(&request, "/records", &table);
    assert_eq!(response.status_code(), StatusCode(304));
    assert_eq!(header_value(&response, "ETag"), current);

    cleanup(&path);
}

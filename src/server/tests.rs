use super::{JSON_QUERY_MEDIA_TYPE, header};
use tiny_http::{Method, Request, TestRequest};

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
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

fn stream_query_request(body: &'static str) -> Request {
    TestRequest::new()
        .with_method("QUERY".parse().unwrap())
        .with_path("/records/stream")
        .with_header(header("Content-Type", JSON_QUERY_MEDIA_TYPE))
        .with_body(body)
        .into()
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

mod cdc_tests;
mod mutation_tests;
mod query_tests;

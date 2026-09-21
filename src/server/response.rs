use super::{HttpResponse, ServerBody};
use crate::dbf::DbfError;
use serde_json::{Value, json};
use std::io::Cursor;
use tiny_http::{Header, StatusCode};

pub fn dbf_error_response(dbf_error: DbfError) -> HttpResponse {
    match dbf_error {
        DbfError::Invalid(message) if message == "record not found" => {
            json_response(404, error("not_found", "record not found"), false)
        }
        DbfError::Invalid(message) => json_response(422, error("invalid_record", &message), false),
        DbfError::Io(io_error) => {
            json_response(500, error("storage_error", &io_error.to_string()), false)
        }
    }
}

pub fn error(code: &str, message: &str) -> Value {
    json!({"error": {"code": code, "message": message}})
}

pub fn json_response(status: u16, body: Value, accept_query: bool) -> HttpResponse {
    json_bytes_response(status, body.to_string().into_bytes(), accept_query)
}

pub fn json_bytes_response(status: u16, body: Vec<u8>, accept_query: bool) -> HttpResponse {
    let body_length = body.len();
    let mut response = tiny_http::Response::new(
        StatusCode(status),
        vec![header("Content-Type", "application/json")],
        ServerBody::Buffered(Cursor::new(body)),
        Some(body_length),
        None,
    );
    if accept_query {
        response = response.with_header(header("Accept-Query", "\"application/json\""));
    }
    response
}

pub fn options_response(allow: &str) -> HttpResponse {
    empty_response(204)
        .with_header(header("Allow", allow))
        .with_header(header("Accept-Query", "\"application/json\""))
        .with_header(header(
            "Accept-Patch",
            "application/json, application/merge-patch+json, application/json-patch+json",
        ))
}

pub fn empty_response(status: u16) -> HttpResponse {
    tiny_http::Response::new(
        StatusCode(status),
        Vec::new(),
        ServerBody::Buffered(Cursor::new(Vec::new())),
        Some(0),
        None,
    )
}

pub fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static HTTP header is valid")
}

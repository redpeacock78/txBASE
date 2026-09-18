use crate::dbf::{DbfRecord, DbfTable};
use crate::query::{self, JSON_QUERY_MEDIA_TYPE};
use serde_json::{Value, json};
use std::io::Read;
use tiny_http::{Header, Method, Request, Response, Server};

const MAX_QUERY_BODY: u64 = 1024 * 1024;

pub fn serve(table: DbfTable, bind: &str) -> Result<(), String> {
    let server = Server::http(bind).map_err(|error| format!("cannot bind {bind}: {error}"))?;
    eprintln!("listening on http://{bind}");
    for request in server.incoming_requests() {
        handle_request(request, &table);
    }
    Ok(())
}

fn handle_request(mut request: Request, table: &DbfTable) {
    let path = request.url().split('?').next().unwrap_or("/").to_owned();
    let is_get = matches!(request.method(), Method::Get);
    let is_query = request.method().as_str() == "QUERY";
    let response = if is_get {
        get_response(&path, table)
    } else if is_query {
        query_response(&mut request, &path)
    } else {
        json_response(
            405,
            json!({"error": {"code": "method_not_allowed", "message": "only GET and QUERY are available"}}),
            true,
        )
        .with_header(header("Allow", "GET, QUERY"))
    };
    if let Err(error) = request.respond(response) {
        eprintln!("failed to send HTTP response: {error}");
    }
}

fn get_response(path: &str, table: &DbfTable) -> Response<std::io::Cursor<Vec<u8>>> {
    if path == "/records" {
        return json_response(200, Value::Array(table.active_json()), true);
    }
    let Some(raw_id) = path.strip_prefix("/records/") else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    let Ok(id) = raw_id.parse::<usize>() else {
        return json_response(
            404,
            error("not_found", "record id must be a positive integer"),
            false,
        );
    };
    match table.active_record(id) {
        Some(record) => json_response(200, record_json(record), true),
        None => json_response(404, error("not_found", "record not found"), false),
    }
}

fn query_response(request: &mut Request, path: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    if path != "/records" {
        return json_response(404, error("not_found", "resource not found"), false);
    }
    let Some(content_type) = content_type(request) else {
        return json_response(
            400,
            error(
                "missing_content_type",
                "QUERY requires Content-Type: application/json",
            ),
            true,
        );
    };
    if !content_type.split(';').next().is_some_and(|media_type| {
        media_type
            .trim()
            .eq_ignore_ascii_case(JSON_QUERY_MEDIA_TYPE)
    }) {
        return json_response(
            415,
            error(
                "unsupported_media_type",
                "only application/json QUERY content is supported",
            ),
            true,
        );
    }

    let mut body = Vec::new();
    let read_result = request
        .as_reader()
        .take(MAX_QUERY_BODY + 1)
        .read_to_end(&mut body);
    if read_result.is_err() {
        return json_response(
            400,
            error("invalid_body", "could not read request content"),
            true,
        );
    }
    if body.len() as u64 > MAX_QUERY_BODY {
        return json_response(
            413,
            error("body_too_large", "QUERY body exceeds 1 MiB"),
            true,
        );
    }
    if let Err(query_error) = query::parse(&body) {
        return json_response(422, error("invalid_query", &query_error.to_string()), true);
    }

    json_response(
        501,
        error(
            "query_not_implemented",
            "QUERY routing and validation are ready; execution is not implemented",
        ),
        true,
    )
}

fn content_type(request: &Request) -> Option<&str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Content-Type"))
        .map(|header| header.value.as_str())
}

fn record_json(record: &DbfRecord) -> Value {
    Value::Object(record.values.clone())
}

fn error(code: &str, message: &str) -> Value {
    json!({"error": {"code": code, "message": message}})
}

fn json_response(
    status: u16,
    body: Value,
    accept_query: bool,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut response = Response::from_string(body.to_string())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json"));
    if accept_query {
        response = response.with_header(header("Accept-Query", "\"application/json\""));
    }
    response
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static HTTP header is valid")
}

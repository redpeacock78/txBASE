use crate::dbf::{DbfError, DbfTable};
use crate::query::{self, JSON_QUERY_MEDIA_TYPE};
use serde_json::{Map, Value, json};
use std::io::{Cursor, Read};
use std::path::Path;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

mod catalog;
mod catalog_transaction;
mod etag;
mod explain;
mod range;
mod records;
mod stream;
mod transaction;

const MAX_BODY: usize = 1024 * 1024;

pub(super) enum ServerBody {
    Buffered(Cursor<Vec<u8>>),
    Stream(Box<dyn Read + Send>),
}

impl ServerBody {
    #[cfg(test)]
    fn into_inner(self) -> Vec<u8> {
        match self {
            Self::Buffered(body) => body.into_inner(),
            Self::Stream(mut body) => {
                let mut bytes = Vec::new();
                let _ = body.read_to_end(&mut bytes);
                bytes
            }
        }
    }
}

impl Read for ServerBody {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Buffered(body) => body.read(buffer),
            Self::Stream(body) => body.read(buffer),
        }
    }
}

type HttpResponse = Response<ServerBody>;

use range::query_result_response;
use records::{delete_response, get_response, post_response, update_response};

pub fn serve(mut table: DbfTable, dbf_path: impl AsRef<Path>, bind: &str) -> Result<(), String> {
    let dbf_path = dbf_path.as_ref();
    let server = Server::http(bind).map_err(|error| format!("cannot bind {bind}: {error}"))?;
    eprintln!("listening on http://{bind}");
    for request in server.incoming_requests() {
        handle_request(request, &mut table, dbf_path);
    }
    Ok(())
}

pub fn serve_catalog(root: impl AsRef<Path>, bind: &str) -> Result<(), String> {
    catalog::serve(root, bind)
}

fn handle_request(mut request: Request, table: &mut DbfTable, dbf_path: &Path) {
    let path = request.url().split('?').next().unwrap_or("/").to_owned();
    let is_query = request.method().as_str() == "QUERY";
    let response = if request.method().as_str() == "OPTIONS" {
        options_response("GET, HEAD, OPTIONS, POST, PUT, PATCH, DELETE, QUERY")
    } else if matches!(request.method(), Method::Get | Method::Head) {
        get_response(&request, &path, table)
    } else if is_query {
        if path == "/explain" {
            explain::response(&mut request, dbf_path)
        } else {
            query_response_at(&mut request, &path, table, dbf_path)
        }
    } else if matches!(request.method(), Method::Post) {
        if path == "/transaction" {
            transaction::response(&mut request, table, dbf_path)
        } else {
            post_response(&mut request, &path, table, dbf_path)
        }
    } else if matches!(request.method(), Method::Put) {
        update_response(&mut request, &path, table, dbf_path, true)
    } else if matches!(request.method(), Method::Patch) {
        update_response(&mut request, &path, table, dbf_path, false)
    } else if matches!(request.method(), Method::Delete) {
        delete_response(&request, &path, table, dbf_path)
    } else {
        json_response(
            405,
            error(
                "method_not_allowed",
                "only GET, HEAD, OPTIONS, POST, PUT, PATCH, DELETE, and QUERY are available",
            ),
            true,
        )
        .with_header(header(
            "Allow",
            "GET, HEAD, OPTIONS, POST, PUT, PATCH, DELETE, QUERY",
        ))
    };
    if let Err(error) = request.respond(response) {
        eprintln!("failed to send HTTP response: {error}");
    }
}

#[cfg(test)]
fn query_response(request: &mut Request, path: &str, table: &DbfTable) -> HttpResponse {
    query_response_with_path(request, path, table, None)
}

pub(super) fn query_response_at(
    request: &mut Request,
    path: &str,
    table: &DbfTable,
    dbf_path: &Path,
) -> HttpResponse {
    query_response_with_path(request, path, table, Some(dbf_path))
}

fn query_response_with_path(
    request: &mut Request,
    path: &str,
    table: &DbfTable,
    dbf_path: Option<&Path>,
) -> HttpResponse {
    if path == "/records/stream" {
        return stream::response(request, table);
    }
    if path != "/records" {
        return json_response(404, error("not_found", "resource not found"), false);
    }
    let body = match read_json_body(request, "QUERY", true) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let query = match query::parse(&body) {
        Ok(query) => query,
        Err(query_error) => {
            return json_response(422, error("invalid_query", &query_error.to_string()), true);
        }
    };
    let result = match dbf_path {
        Some(dbf_path) => query::execute_query_at_page(table, dbf_path, &query),
        None => query::execute_query_page(table, &query),
    };
    match result {
        Ok(page) => {
            let body = if query.page_size.is_some() || query.cursor.is_some() {
                json!({
                    "records": page.records,
                    "cursor": page.next_cursor,
                })
            } else {
                Value::Array(page.records)
            };
            query_result_response(request, body)
        }
        Err(query_error) => {
            json_response(422, error("invalid_query", &query_error.to_string()), true)
        }
    }
}

fn read_json_object(
    request: &mut Request,
    operation: &str,
    accept_query: bool,
) -> Result<Map<String, Value>, HttpResponse> {
    let body = read_json_body(request, operation, accept_query)?;
    let value = serde_json::from_slice::<Value>(&body).map_err(|parse_error| {
        json_response(
            422,
            error("invalid_json", &format!("invalid JSON body: {parse_error}")),
            accept_query,
        )
    })?;
    value.as_object().cloned().ok_or_else(|| {
        json_response(
            422,
            error("invalid_json", "JSON body must be an object"),
            accept_query,
        )
    })
}

fn read_json_body(
    request: &mut Request,
    operation: &str,
    accept_query: bool,
) -> Result<Vec<u8>, HttpResponse> {
    let Some(content_type) = content_type(request) else {
        return Err(json_response(
            400,
            error(
                "missing_content_type",
                &format!("{operation} requires Content-Type: application/json"),
            ),
            accept_query,
        ));
    };
    if !content_type.split(';').next().is_some_and(|media_type| {
        media_type
            .trim()
            .eq_ignore_ascii_case(JSON_QUERY_MEDIA_TYPE)
    }) {
        return Err(json_response(
            415,
            error(
                "unsupported_media_type",
                "only application/json request content is supported",
            ),
            accept_query,
        ));
    }
    let mut body = Vec::new();
    if request
        .as_reader()
        .take((MAX_BODY + 1) as u64)
        .read_to_end(&mut body)
        .is_err()
    {
        return Err(json_response(
            400,
            error("invalid_body", "could not read request content"),
            accept_query,
        ));
    }
    if body.len() > MAX_BODY {
        return Err(json_response(
            413,
            error("body_too_large", "request body exceeds 1 MiB"),
            accept_query,
        ));
    }
    Ok(body)
}

fn dbf_error_response(dbf_error: DbfError) -> HttpResponse {
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

fn request_header<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.as_str())
}

fn content_type(request: &Request) -> Option<&str> {
    request_header(request, "Content-Type")
}

fn error(code: &str, message: &str) -> Value {
    json!({"error": {"code": code, "message": message}})
}

fn json_response(status: u16, body: Value, accept_query: bool) -> HttpResponse {
    json_bytes_response(status, body.to_string().into_bytes(), accept_query)
}

fn json_bytes_response(status: u16, body: Vec<u8>, accept_query: bool) -> HttpResponse {
    let body_length = body.len();
    let mut response = Response::new(
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

pub(super) fn options_response(allow: &str) -> HttpResponse {
    empty_response(204)
        .with_header(header("Allow", allow))
        .with_header(header("Accept-Query", "\"application/json\""))
}

pub(super) fn empty_response(status: u16) -> HttpResponse {
    Response::new(
        StatusCode(status),
        Vec::new(),
        ServerBody::Buffered(Cursor::new(Vec::new())),
        Some(0),
        None,
    )
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static HTTP header is valid")
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod catalog_tests;

#[cfg(test)]
mod etag_tests;

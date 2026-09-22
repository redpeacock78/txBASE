use crate::dbf::DbfTable;
use crate::query::{self, JSON_QUERY_MEDIA_TYPE};
use serde_json::{Value, json};
use std::io::{Cursor, Read};
use std::path::Path;
use tiny_http::{Method, Request, Response, Server};

mod body;
mod catalog;
mod catalog_transaction;
mod cdc;
mod etag;
mod explain;
mod json_patch;
mod merge_patch;
mod range;
mod records;
mod response;
mod stream;
mod transaction;

const MAX_BODY: usize = 1024 * 1024;
const JSON_MERGE_PATCH_MEDIA_TYPE: &str = "application/merge-patch+json";
const JSON_PATCH_MEDIA_TYPE: &str = "application/json-patch+json";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PatchMediaType {
    Json,
    MergePatch,
    JsonPatch,
}

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

pub(super) use body::{read_json_body, read_json_object, read_json_patch_document, request_header};
pub(super) use response::{
    dbf_error_response, empty_response, error, header, json_bytes_response, json_response,
    options_response,
};

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
    let url = request.url().to_owned();
    let path = url.split('?').next().unwrap_or("/").to_owned();
    let is_query = request.method().as_str() == "QUERY";
    let response = if request.method().as_str() == "OPTIONS" {
        options_response("GET, HEAD, OPTIONS, POST, PUT, PATCH, DELETE, QUERY")
    } else if matches!(request.method(), Method::Get | Method::Head) {
        if path == "/cdc" {
            cdc::table_response(&url, dbf_path)
        } else {
            get_response(&request, &path, table)
        }
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
    query_response_without_path(request, path, table)
}

pub(super) fn query_response_without_path(
    request: &mut Request,
    path: &str,
    table: &DbfTable,
) -> HttpResponse {
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod catalog_tests;

#[cfg(test)]
mod etag_tests;

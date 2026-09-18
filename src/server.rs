use crate::dbf::{DbfError, DbfRecord, DbfTable};
use crate::query::{self, JSON_QUERY_MEDIA_TYPE};
use serde_json::{Map, Value, json};
use std::io::{Cursor, Read};
use std::path::Path;
use tiny_http::{Header, Method, Request, Response, Server};

const MAX_BODY: usize = 1024 * 1024;
type HttpResponse = Response<Cursor<Vec<u8>>>;

pub fn serve(mut table: DbfTable, dbf_path: impl AsRef<Path>, bind: &str) -> Result<(), String> {
    let dbf_path = dbf_path.as_ref();
    let server = Server::http(bind).map_err(|error| format!("cannot bind {bind}: {error}"))?;
    eprintln!("listening on http://{bind}");
    for request in server.incoming_requests() {
        handle_request(request, &mut table, dbf_path);
    }
    Ok(())
}

fn handle_request(mut request: Request, table: &mut DbfTable, dbf_path: &Path) {
    let path = request.url().split('?').next().unwrap_or("/").to_owned();
    let is_query = request.method().as_str() == "QUERY";
    let response = if matches!(request.method(), Method::Get) {
        get_response(&path, table)
    } else if is_query {
        query_response(&mut request, &path, table)
    } else if matches!(request.method(), Method::Post) {
        post_response(&mut request, &path, table, dbf_path)
    } else if matches!(request.method(), Method::Put) {
        update_response(&mut request, &path, table, dbf_path, true)
    } else if matches!(request.method(), Method::Patch) {
        update_response(&mut request, &path, table, dbf_path, false)
    } else if matches!(request.method(), Method::Delete) {
        delete_response(&path, table, dbf_path)
    } else {
        json_response(
            405,
            error(
                "method_not_allowed",
                "only GET, POST, PUT, PATCH, DELETE, and QUERY are available",
            ),
            true,
        )
        .with_header(header("Allow", "GET, POST, PUT, PATCH, DELETE, QUERY"))
    };
    if let Err(error) = request.respond(response) {
        eprintln!("failed to send HTTP response: {error}");
    }
}

fn get_response(path: &str, table: &DbfTable) -> HttpResponse {
    if path == "/records" {
        return json_response(200, Value::Array(table.active_json()), true);
    }
    let Ok(id) = record_id(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    match table.active_record(id) {
        Some(record) => json_response(200, record_json(record), true),
        None => json_response(404, error("not_found", "record not found"), false),
    }
}

fn query_response(request: &mut Request, path: &str, table: &DbfTable) -> HttpResponse {
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
    match query::execute_query(table, &query) {
        Ok(records) => json_response(200, Value::Array(records), true),
        Err(query_error) => {
            json_response(422, error("invalid_query", &query_error.to_string()), true)
        }
    }
}

fn post_response(
    request: &mut Request,
    path: &str,
    table: &mut DbfTable,
    dbf_path: &Path,
) -> HttpResponse {
    if path != "/records" {
        return json_response(404, error("not_found", "resource not found"), false);
    }
    let values = match read_json_object(request, "POST", false) {
        Ok(values) => values,
        Err(response) => return response,
    };
    let original = table.clone();
    let id = match table.insert_record(values) {
        Ok(id) => id,
        Err(error) => return dbf_error_response(error),
    };
    if let Err(response) = persist_mutation(table, original, dbf_path) {
        return response;
    }
    let Some(record) = table.active_record(id) else {
        return json_response(
            500,
            error("storage_error", "inserted record is unavailable"),
            false,
        );
    };
    json_response(201, record_json(record), false)
        .with_header(header("Location", &format!("/records/{id}")))
}

fn update_response(
    request: &mut Request,
    path: &str,
    table: &mut DbfTable,
    dbf_path: &Path,
    replace: bool,
) -> HttpResponse {
    let Ok(id) = record_id(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    let values = match read_json_object(request, if replace { "PUT" } else { "PATCH" }, false) {
        Ok(values) => values,
        Err(response) => return response,
    };
    let original = table.clone();
    let result = if replace {
        table.replace_record(id, values)
    } else {
        table.patch_record(id, values)
    };
    if let Err(error) = result {
        return dbf_error_response(error);
    }
    if let Err(response) = persist_mutation(table, original, dbf_path) {
        return response;
    }
    match table.active_record(id) {
        Some(record) => json_response(200, record_json(record), false),
        None => json_response(
            500,
            error("storage_error", "updated record is unavailable"),
            false,
        ),
    }
}

fn delete_response(path: &str, table: &mut DbfTable, dbf_path: &Path) -> HttpResponse {
    let Ok(id) = record_id(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    let original = table.clone();
    if let Err(error) = table.delete_record(id) {
        return dbf_error_response(error);
    }
    if let Err(response) = persist_mutation(table, original, dbf_path) {
        return response;
    }
    Response::from_string(String::new()).with_status_code(204)
}

fn persist_mutation(
    table: &mut DbfTable,
    original: DbfTable,
    dbf_path: &Path,
) -> Result<(), HttpResponse> {
    match table.save_with_wal(dbf_path) {
        Ok(()) => Ok(()),
        Err(dbf_error) => {
            *table = original;
            Err(json_response(
                500,
                error("storage_error", &dbf_error.to_string()),
                false,
            ))
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

fn record_id(path: &str) -> Result<usize, ()> {
    let Some(raw_id) = path.strip_prefix("/records/") else {
        return Err(());
    };
    let id = raw_id.parse::<usize>().map_err(|_| ())?;
    (id > 0).then_some(id).ok_or(())
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

fn json_response(status: u16, body: Value, accept_query: bool) -> HttpResponse {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tiny_http::{Method, StatusCode, TestRequest};

    fn fixture() -> Vec<u8> {
        include_str!("../tests/fixtures/users.dbf.hex")
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

    #[test]
    fn mutation_endpoints_persist_and_delete_records() {
        let path =
            std::env::temp_dir().join(format!("txbase-server-test-{}.dbf", std::process::id()));
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

        let mut patch = json_request(Method::Patch, "/records/3", r#"{"AGE":43}"#);
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

        assert_eq!(
            delete_response("/records/3", &mut table, &path).status_code(),
            StatusCode(204)
        );
        let persisted = DbfTable::from_path(&path).unwrap();
        assert!(persisted.active_record(3).is_none());
        assert!(persisted.records()[2].deleted);
        assert!(!path.with_extension("txbase.wal").exists());
        fs::remove_file(path).unwrap();
    }
}

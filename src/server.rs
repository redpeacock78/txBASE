use crate::dbf::{DbfError, DbfRecord, DbfTable};
use crate::query::{self, JSON_QUERY_MEDIA_TYPE};
use serde_json::{Map, Value, json};
use std::io::{Cursor, Read};
use std::path::Path;
use tiny_http::{Header, Method, Request, Response, Server};

mod range;

const MAX_BODY: usize = 1024 * 1024;
type HttpResponse = Response<Cursor<Vec<u8>>>;

use range::query_result_response;

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
        Ok(records) => query_result_response(request, Value::Array(records)),
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
            *table = DbfTable::from_path(dbf_path).unwrap_or(original);
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

fn record_json(record: &DbfRecord) -> Value {
    Value::Object(record.values.clone())
}

fn error(code: &str, message: &str) -> Value {
    json!({"error": {"code": code, "message": message}})
}

fn json_response(status: u16, body: Value, accept_query: bool) -> HttpResponse {
    json_bytes_response(status, body.to_string().into_bytes(), accept_query)
}

fn json_bytes_response(status: u16, body: Vec<u8>, accept_query: bool) -> HttpResponse {
    let mut response = Response::from_data(body)
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

        let mut unknown_field =
            query_request(r#"{"filtre":{"AGE":29}}"#, Some(JSON_QUERY_MEDIA_TYPE));
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

        let response = persist_mutation(&mut table, original, &path).unwrap_err();
        assert_eq!(response.status_code(), StatusCode(500));
        assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);

        fs::remove_file(path).unwrap();
    }
}

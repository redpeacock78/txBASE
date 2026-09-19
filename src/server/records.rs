use super::{
    DbfTable, HttpResponse, Response, dbf_error_response, error, etag, header, json_response,
    read_json_object,
};
use crate::dbf::DbfRecord;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::Value;
use std::path::Path;
use tiny_http::Request;

pub(super) fn get_response(request: &Request, path: &str, table: &DbfTable) -> HttpResponse {
    if path == "/records" {
        if let Some(response) = etag::not_modified(request, table, true) {
            return response;
        }
        return etag::with_current(
            json_response(200, Value::Array(table.active_json()), true),
            table,
        );
    }
    let Ok(id) = record_id(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    match table.active_record(id) {
        Some(record) => {
            if let Some(response) = etag::not_modified(request, table, true) {
                response
            } else {
                etag::with_current(json_response(200, record_json(record), true), table)
            }
        }
        None => json_response(404, error("not_found", "record not found"), false),
    }
}

pub(super) fn post_response(
    request: &mut Request,
    path: &str,
    table: &mut DbfTable,
    dbf_path: &Path,
) -> HttpResponse {
    if path != "/records" {
        return json_response(404, error("not_found", "resource not found"), false);
    }
    if let Err(response) = etag::require_if_match(request, table, true) {
        return response;
    }
    let values = match read_json_object(request, "POST", false) {
        Ok(values) => values,
        Err(response) => return response,
    };
    let operation = OperationIr {
        method: OperationMethod::Post,
        path: path.to_owned(),
        body: Some(Value::Object(values.clone())),
    };
    let original = table.clone();
    let id = match table.insert_record(values) {
        Ok(id) => id,
        Err(error) => return dbf_error_response(error),
    };
    if let Err(response) = persist_mutation(table, original, dbf_path, &operation) {
        return response;
    }
    let Some(record) = table.active_record(id) else {
        return json_response(
            500,
            error("storage_error", "inserted record is unavailable"),
            false,
        );
    };
    etag::with_current(
        json_response(201, record_json(record), false)
            .with_header(header("Location", &format!("/records/{id}"))),
        table,
    )
}

pub(super) fn update_response(
    request: &mut Request,
    path: &str,
    table: &mut DbfTable,
    dbf_path: &Path,
    replace: bool,
) -> HttpResponse {
    let Ok(id) = record_id(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    if table.active_record(id).is_none() {
        return json_response(404, error("not_found", "record not found"), false);
    }
    if let Err(response) = etag::require_if_match(request, table, true) {
        return response;
    }
    let values = match read_json_object(request, if replace { "PUT" } else { "PATCH" }, false) {
        Ok(values) => values,
        Err(response) => return response,
    };
    let operation = OperationIr {
        method: if replace {
            OperationMethod::Put
        } else {
            OperationMethod::Patch
        },
        path: path.to_owned(),
        body: Some(Value::Object(values.clone())),
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
    if let Err(response) = persist_mutation(table, original, dbf_path, &operation) {
        return response;
    }
    match table.active_record(id) {
        Some(record) => etag::with_current(json_response(200, record_json(record), false), table),
        None => json_response(
            500,
            error("storage_error", "updated record is unavailable"),
            false,
        ),
    }
}

pub(super) fn delete_response(
    request: &Request,
    path: &str,
    table: &mut DbfTable,
    dbf_path: &Path,
) -> HttpResponse {
    let Ok(id) = record_id(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    if table.active_record(id).is_none() {
        return json_response(404, error("not_found", "record not found"), false);
    }
    if let Err(response) = etag::require_if_match(request, table, true) {
        return response;
    }
    let operation = OperationIr {
        method: OperationMethod::Delete,
        path: path.to_owned(),
        body: None,
    };
    let original = table.clone();
    if let Err(error) = table.delete_record(id) {
        return dbf_error_response(error);
    }
    if let Err(response) = persist_mutation(table, original, dbf_path, &operation) {
        return response;
    }
    etag::with_current(
        Response::from_string(String::new()).with_status_code(204),
        table,
    )
}

pub(super) fn persist_mutation(
    table: &mut DbfTable,
    original: DbfTable,
    dbf_path: &Path,
    operation: &OperationIr,
) -> Result<(), HttpResponse> {
    match table.save_with_operation(dbf_path, operation) {
        Ok(()) => Ok(()),
        Err(dbf_error) => {
            let encoding = original.effective_encoding_override().map(str::to_owned);
            *table = DbfTable::from_path_with_encoding(dbf_path, encoding.as_deref())
                .unwrap_or(original);
            Err(json_response(
                500,
                error("storage_error", &dbf_error.to_string()),
                false,
            ))
        }
    }
}

fn record_id(path: &str) -> Result<usize, ()> {
    let Some(raw_id) = path.strip_prefix("/records/") else {
        return Err(());
    };
    let id = raw_id.parse::<usize>().map_err(|_| ())?;
    (id > 0).then_some(id).ok_or(())
}

fn record_json(record: &DbfRecord) -> Value {
    Value::Object(record.values.clone())
}

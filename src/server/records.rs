use super::{
    DbfTable, HttpResponse, dbf_error_response, empty_response, error, etag, header, json_response,
    read_json_object, read_json_object_with_merge_patch,
};
use crate::dbf::DbfRecord;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::{Map, Value};
use std::path::Path;
use tiny_http::Request;

type MutationValidator<'a> = dyn Fn(&DbfTable) -> Result<(), String> + 'a;

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
    post_response_at(request, path, path, table, dbf_path)
}

pub(super) fn post_response_at(
    request: &mut Request,
    operation_path: &str,
    location_path: &str,
    table: &mut DbfTable,
    dbf_path: &Path,
) -> HttpResponse {
    post_response_at_with_validator(
        request,
        operation_path,
        location_path,
        table,
        dbf_path,
        None,
    )
}

pub(super) fn post_response_at_with_validator(
    request: &mut Request,
    operation_path: &str,
    location_path: &str,
    table: &mut DbfTable,
    dbf_path: &Path,
    validator: Option<&MutationValidator<'_>>,
) -> HttpResponse {
    if operation_path != "/records" {
        return json_response(404, error("not_found", "resource not found"), false);
    }
    if let Err(response) = etag::require_mutation_preconditions(request, table, true) {
        return response;
    }
    let values = match read_json_object(request, "POST", false) {
        Ok(values) => values,
        Err(response) => return response,
    };
    let operation = OperationIr {
        method: OperationMethod::Post,
        path: operation_path.to_owned(),
        body: Some(Value::Object(values.clone())),
    };
    let original = table.clone();
    let id = match table.insert_record(values) {
        Ok(id) => id,
        Err(error) => return dbf_error_response(error),
    };
    if let Some(validate) = validator {
        if let Err(message) = validate(table) {
            return json_response(422, error("constraint_violation", &message), false);
        }
    }
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
    etag::with_transaction(
        json_response(201, record_json(record), false)
            .with_header(header("Location", &format!("{location_path}/{id}"))),
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
    update_response_with_validator(request, path, table, dbf_path, replace, None)
}

pub(super) fn update_response_with_validator(
    request: &mut Request,
    path: &str,
    table: &mut DbfTable,
    dbf_path: &Path,
    replace: bool,
    validator: Option<&MutationValidator<'_>>,
) -> HttpResponse {
    let Ok(id) = record_id(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    if table.active_record(id).is_none() {
        return json_response(404, error("not_found", "record not found"), false);
    }
    if let Err(response) = etag::require_mutation_preconditions(request, table, true) {
        return response;
    }
    let (values, is_merge_patch) = if replace {
        match read_json_object(request, "PUT", false) {
            Ok(values) => (values, false),
            Err(response) => return response,
        }
    } else {
        match read_json_object_with_merge_patch(request, "PATCH", false) {
            Ok(values) => values,
            Err(response) => return response,
        }
    };
    let values = if is_merge_patch {
        let Some(current) = table.active_record(id).map(|record| record.values.clone()) else {
            return json_response(
                500,
                error("storage_error", "record disappeared during update"),
                false,
            );
        };
        apply_merge_patch(current, values)
    } else {
        values
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
    if let Some(validate) = validator {
        if let Err(message) = validate(table) {
            return json_response(422, error("constraint_violation", &message), false);
        }
    }
    if let Err(response) = persist_mutation(table, original, dbf_path, &operation) {
        return response;
    }
    match table.active_record(id) {
        Some(record) => {
            etag::with_transaction(json_response(200, record_json(record), false), table)
        }
        None => json_response(
            500,
            error("storage_error", "updated record is unavailable"),
            false,
        ),
    }
}

fn apply_merge_patch(current: Map<String, Value>, patch: Map<String, Value>) -> Map<String, Value> {
    let known_fields = current.keys().cloned().collect::<Vec<_>>();
    let unknown_null_fields = patch
        .iter()
        .filter(|(key, value)| value.is_null() && !current.contains_key(*key))
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    let mut target = Value::Object(current);
    merge_patch_value(&mut target, Value::Object(patch));
    let mut target = match target {
        Value::Object(object) => object,
        Value::Array(_) | Value::Bool(_) | Value::Null | Value::Number(_) | Value::String(_) => {
            unreachable!("root merge patch remains an object")
        }
    };
    for key in known_fields {
        target.entry(key).or_insert(Value::Null);
    }
    for key in unknown_null_fields {
        target.entry(key).or_insert(Value::Null);
    }
    target
}

fn merge_patch_value(target: &mut Value, patch: Value) {
    let Value::Object(patch) = patch else {
        *target = patch;
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    let target = target.as_object_mut().expect("target is an object");
    for (key, value) in patch {
        if value.is_null() {
            target.remove(&key);
        } else {
            merge_patch_value(target.entry(key).or_insert(Value::Null), value);
        }
    }
}

pub(super) fn delete_response(
    request: &Request,
    path: &str,
    table: &mut DbfTable,
    dbf_path: &Path,
) -> HttpResponse {
    delete_response_with_validator(request, path, table, dbf_path, None)
}

pub(super) fn delete_response_with_validator(
    request: &Request,
    path: &str,
    table: &mut DbfTable,
    dbf_path: &Path,
    validator: Option<&MutationValidator<'_>>,
) -> HttpResponse {
    let Ok(id) = record_id(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    if table.active_record(id).is_none() {
        return json_response(404, error("not_found", "record not found"), false);
    }
    if let Err(response) = etag::require_mutation_preconditions(request, table, true) {
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
    if let Some(validate) = validator {
        if let Err(message) = validate(table) {
            return json_response(422, error("constraint_violation", &message), false);
        }
    }
    if let Err(response) = persist_mutation(table, original, dbf_path, &operation) {
        return response;
    }
    etag::with_transaction(empty_response(204), table)
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

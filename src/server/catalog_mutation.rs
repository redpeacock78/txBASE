use super::{
    HttpResponse, PatchMediaType, empty_response, error, etag, header, json_response,
    read_json_object, read_json_patch_document, request_header,
};
use crate::catalog::Catalog;
use crate::dbf::DbfTable;
use crate::replication::ReplicationLog;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::Value;
use tiny_http::{Method, Request};

pub(super) fn response(
    request: &mut Request,
    path: &str,
    catalog: &Catalog,
    replication: &mut ReplicationLog,
) -> HttpResponse {
    let Some((table_name, local_path)) = super::catalog::record_route(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    let table = match catalog.open_table(table_name) {
        Ok(table) => table,
        Err(catalog_error) => {
            return json_response(
                500,
                error("catalog_error", &catalog_error.to_string()),
                false,
            );
        }
    };
    if matches!(
        request.method(),
        Method::Put | Method::Patch | Method::Delete
    ) {
        let Ok(record_number) = record_id(&local_path) else {
            return json_response(404, error("not_found", "resource not found"), false);
        };
        if table.active_record(record_number).is_none() {
            return json_response(404, error("not_found", "record not found"), false);
        }
    }
    if let Err(response) = etag::require_mutation_preconditions(request, &table, true) {
        return response;
    }

    let record_number = table.records().len().saturating_add(1);
    let operation = match operation(request, path, &local_path, &table) {
        Ok(operation) => operation,
        Err(response) => return response,
    };
    let if_match = request_header(request, "If-Match").map(str::to_owned);
    let if_none_match = request_header(request, "If-None-Match").map(str::to_owned);
    let entry = match replication.propose_with_table_preconditions(
        catalog,
        table_name,
        vec![operation],
        if_match.as_deref(),
        if_none_match.as_deref(),
    ) {
        Ok(entry) => entry,
        Err(error) => return super::replication::replication_error_response(error),
    };
    let table = match catalog.open_table(table_name) {
        Ok(table) => table,
        Err(catalog_error) => {
            return json_response(
                500,
                error("catalog_error", &catalog_error.to_string()),
                false,
            );
        }
    };
    let response = match request.method() {
        Method::Post => match table.active_record(record_number) {
            Some(record) => json_response(201, Value::Object(record.values.clone()), false)
                .with_header(header("Location", &path_for_record(path, record_number))),
            None => {
                return json_response(
                    500,
                    error("storage_error", "inserted record is unavailable"),
                    false,
                );
            }
        },
        Method::Put | Method::Patch => match record_id(&local_path)
            .ok()
            .and_then(|number| table.active_record(number))
        {
            Some(record) => json_response(200, Value::Object(record.values.clone()), false),
            None => {
                return json_response(
                    500,
                    error("storage_error", "updated record is unavailable"),
                    false,
                );
            }
        },
        Method::Delete => empty_response(204),
        _ => {
            return json_response(
                405,
                error(
                    "method_not_allowed",
                    "only POST, PUT, PATCH, and DELETE table routes are available",
                ),
                true,
            )
            .with_header(header("Allow", "POST, PUT, PATCH, DELETE"));
        }
    };
    let response = response.with_header(header(
        "X-Txbase-Transaction-Id",
        &entry.transaction_id.to_string(),
    ));
    etag::with_current(response, &table)
}

fn operation(
    request: &mut Request,
    path: &str,
    local_path: &str,
    table: &DbfTable,
) -> Result<OperationIr, HttpResponse> {
    let (method, body) = match request.method() {
        Method::Post => {
            if local_path != "/records" {
                return Err(json_response(
                    404,
                    error("not_found", "resource not found"),
                    false,
                ));
            }
            (
                OperationMethod::Post,
                Some(Value::Object(read_json_object(request, "POST", false)?)),
            )
        }
        Method::Put => (
            OperationMethod::Put,
            Some(Value::Object(read_json_object(request, "PUT", false)?)),
        ),
        Method::Patch => {
            let (document, media_type) = read_json_patch_document(request, "PATCH", false)?;
            let record_number = record_id(local_path)
                .map_err(|_| json_response(404, error("not_found", "resource not found"), false))?;
            let current = table
                .active_record(record_number)
                .map(|record| record.values.clone())
                .ok_or_else(|| json_response(404, error("not_found", "record not found"), false))?;
            let values = match media_type {
                PatchMediaType::Json => document
                    .as_object()
                    .cloned()
                    .expect("validated JSON object"),
                PatchMediaType::MergePatch => super::merge_patch::apply_merge_patch(
                    current,
                    document
                        .as_object()
                        .cloned()
                        .expect("validated merge patch object"),
                ),
                PatchMediaType::JsonPatch => {
                    let patched = super::json_patch::apply(current.clone(), &document).map_err(
                        |message| json_response(422, error("invalid_json_patch", &message), false),
                    )?;
                    super::merge_patch::materialize_removed_fields(current, patched)
                }
            };
            (OperationMethod::Patch, Some(Value::Object(values)))
        }
        Method::Delete => (OperationMethod::Delete, None),
        _ => {
            return Err(json_response(
                405,
                error(
                    "method_not_allowed",
                    "only POST, PUT, PATCH, and DELETE table routes are available",
                ),
                true,
            )
            .with_header(header("Allow", "POST, PUT, PATCH, DELETE")));
        }
    };
    Ok(OperationIr {
        method,
        path: path.to_owned(),
        body,
    })
}

fn record_id(path: &str) -> Result<usize, ()> {
    let Some(raw_id) = path.strip_prefix("/records/") else {
        return Err(());
    };
    let id = raw_id.parse::<usize>().map_err(|_| ())?;
    (id > 0).then_some(id).ok_or(())
}

fn path_for_record(path: &str, record_number: usize) -> String {
    format!("{path}/{record_number}")
}

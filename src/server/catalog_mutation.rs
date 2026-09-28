use super::{
    HttpResponse, PatchMediaType, empty_response, error, etag, header, json_response,
    read_json_object, read_json_patch_document, request_header,
};
use crate::catalog::Catalog;
use crate::dbf::DbfTable;
use crate::replication::ReplicationLog;
use crate::replication::raft::RaftCommandPrecondition;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tiny_http::{Method, Request};

pub(super) fn response(
    request: &mut Request,
    path: &str,
    catalog: &Catalog,
    replication: &mut ReplicationLog,
) -> HttpResponse {
    response_with_writer(request, path, catalog, Some(replication), None)
}

pub(super) fn response_with_raft(
    request: &mut Request,
    path: &str,
    catalog: &Catalog,
    raft: &super::raft::RaftRuntime,
) -> HttpResponse {
    response_with_writer(request, path, catalog, None, Some(raft))
}

fn response_with_writer(
    request: &mut Request,
    path: &str,
    catalog: &Catalog,
    replication: Option<&mut ReplicationLog>,
    raft: Option<&super::raft::RaftRuntime>,
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
    let mut missing_record = false;
    if matches!(
        request.method(),
        Method::Put | Method::Patch | Method::Delete
    ) {
        let Ok(record_number) = record_id(&local_path) else {
            return json_response(404, error("not_found", "resource not found"), false);
        };
        missing_record = table.active_record(record_number).is_none();
        if missing_record && raft.is_none() {
            return json_response(404, error("not_found", "record not found"), false);
        }
    }
    if raft.is_none() {
        if let Err(response) = etag::require_mutation_preconditions(request, &table, true) {
            return response;
        }
    }

    let record_number = table.records().len().saturating_add(1);
    let plan = match operation(request, path, &local_path, &table, catalog, raft) {
        Ok(plan) => plan,
        Err(response) => return response,
    };
    let if_match = request_header(request, "If-Match").map(str::to_owned);
    let if_none_match = request_header(request, "If-None-Match").map(str::to_owned);
    let transaction_id = match plan {
        MutationPlan::Replay(transaction_id) => transaction_id,
        MutationPlan::Apply {
            operation,
            request_fingerprint,
        } => {
            let precondition = Some(RaftCommandPrecondition::Table {
                name: table_name.to_owned(),
                if_match: if_match.clone(),
                if_none_match: if_none_match.clone(),
            });
            if let Some(raft) = raft {
                let steps = vec![crate::xbase::TransactionStep::Mutation(operation)];
                if missing_record {
                    let fingerprint = match request_fingerprint {
                        Some(fingerprint) => fingerprint,
                        None => match crate::replication::raft::RaftCommand::request_fingerprint(
                            &precondition,
                            &steps,
                        ) {
                            Ok(fingerprint) => fingerprint,
                            Err(message) => {
                                return json_response(
                                    500,
                                    error("request_fingerprint_error", &message),
                                    false,
                                );
                            }
                        },
                    };
                    match raft.replayed_transaction(request, catalog, &fingerprint) {
                        Ok(Some(transaction_id)) => transaction_id,
                        Ok(None) => {
                            return json_response(
                                404,
                                error("not_found", "record not found"),
                                false,
                            );
                        }
                        Err(response) => return response,
                    }
                } else {
                    match raft.propose_request(
                        request,
                        catalog,
                        steps,
                        precondition,
                        request_fingerprint,
                    ) {
                        Ok(transaction_id) => transaction_id,
                        Err(response) => return response,
                    }
                }
            } else {
                if missing_record {
                    return json_response(404, error("not_found", "record not found"), false);
                }
                let Some(replication) = replication else {
                    return json_response(
                        500,
                        error("replication_error", "catalog writer is missing"),
                        false,
                    );
                };
                match replication.propose_with_table_preconditions(
                    catalog,
                    table_name,
                    vec![operation],
                    if_match.as_deref(),
                    if_none_match.as_deref(),
                ) {
                    Ok(entry) => entry.transaction_id,
                    Err(error) => return super::replication::replication_error_response(error),
                }
            }
        }
    };

    let snapshot = if raft.is_some() {
        match Catalog::from_path_at(catalog.root(), transaction_id) {
            Ok(snapshot) => Some(snapshot),
            Err(catalog_error) => {
                return json_response(
                    410,
                    error(
                        "raft_retry_snapshot_unavailable",
                        &format!(
                            "the committed command is retained but its response snapshot is unavailable: {catalog_error}"
                        ),
                    ),
                    false,
                );
            }
        }
    } else {
        None
    };
    let response_catalog = snapshot.as_ref().unwrap_or(catalog);
    let table = match response_catalog.open_table(table_name) {
        Ok(table) => table,
        Err(catalog_error) => {
            return json_response(
                500,
                error("catalog_error", &catalog_error.to_string()),
                false,
            );
        }
    };
    let record_number = if snapshot.is_some() {
        table.records().len()
    } else {
        record_number
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
        &transaction_id.to_string(),
    ));
    etag::with_current(response, &table)
}

fn operation(
    request: &mut Request,
    path: &str,
    local_path: &str,
    table: &DbfTable,
    catalog: &Catalog,
    raft: Option<&super::raft::RaftRuntime>,
) -> Result<MutationPlan, HttpResponse> {
    let mut request_fingerprint = None;
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
            let media_type_name = match media_type {
                PatchMediaType::Json => "application/json",
                PatchMediaType::MergePatch => "application/merge-patch+json",
                PatchMediaType::JsonPatch => "application/json-patch+json",
            };
            let encoded = serde_json::to_vec(&(
                "txbase-raft-http-patch-v1",
                path,
                media_type_name,
                request_header(request, "If-Match"),
                request_header(request, "If-None-Match"),
                &document,
            ))
            .map_err(|serialization_error| {
                json_response(
                    500,
                    error(
                        "request_fingerprint_error",
                        &serialization_error.to_string(),
                    ),
                    false,
                )
            })?;
            request_fingerprint = Some(Sha256::digest(encoded).to_vec());
            let record_number = record_id(local_path)
                .map_err(|_| json_response(404, error("not_found", "resource not found"), false))?;
            if let (Some(raft), Some(fingerprint)) = (raft, request_fingerprint.as_deref()) {
                if let Some(transaction_id) =
                    raft.replayed_transaction(request, catalog, fingerprint)?
                {
                    return Ok(MutationPlan::Replay(transaction_id));
                }
            }
            if table.active_record(record_number).is_none() {
                return Err(json_response(
                    404,
                    error("not_found", "record not found"),
                    false,
                ));
            }
            if raft.is_some() {
                etag::require_mutation_preconditions(request, table, true)?;
            }
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
    Ok(MutationPlan::Apply {
        operation: OperationIr {
            method,
            path: path.to_owned(),
            body,
        },
        request_fingerprint,
    })
}

enum MutationPlan {
    Apply {
        operation: OperationIr,
        request_fingerprint: Option<Vec<u8>>,
    },
    Replay(u64),
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

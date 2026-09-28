use super::{HttpResponse, error, etag, header, json_response, read_json_body, request_header};
use crate::catalog::{Catalog, CatalogError, CatalogTransactionError};
use crate::replication::raft::RaftCommandPrecondition;
use crate::replication::{ReplicationError, ReplicationLog};
use crate::xbase::{TransactionBatch, TransactionCommand, TransactionStep};
use serde_json::json;
use tiny_http::Request;

#[cfg(test)]
pub(super) fn response(request: &mut Request, catalog: &Catalog) -> HttpResponse {
    response_with_replication(request, catalog, None)
}

pub(super) fn response_with_replication(
    request: &mut Request,
    catalog: &Catalog,
    replication: Option<&mut ReplicationLog>,
) -> HttpResponse {
    response_with_writer(request, catalog, replication, None)
}

pub(super) fn response_with_raft(
    request: &mut Request,
    catalog: &Catalog,
    raft: &super::raft::RaftRuntime,
) -> HttpResponse {
    response_with_writer(request, catalog, None, Some(raft))
}

fn response_with_writer(
    request: &mut Request,
    catalog: &Catalog,
    replication: Option<&mut ReplicationLog>,
    raft: Option<&super::raft::RaftRuntime>,
) -> HttpResponse {
    if catalog.is_historical() {
        return json_response(
            405,
            error(
                "historical_snapshot_read_only",
                "historical catalog snapshots are read-only",
            ),
            false,
        )
        .with_header(header("Allow", "GET, HEAD, QUERY"));
    }
    let if_match = request_header(request, "If-Match").map(str::to_owned);
    let if_none_match = request_header(request, "If-None-Match").map(str::to_owned);
    let body = match read_json_body(request, "POST /transaction", false) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let transaction = match serde_json::from_slice::<TransactionBatch>(&body) {
        Ok(transaction) if !transaction.operations.is_empty() => transaction,
        Ok(_) => {
            return json_response(
                422,
                error("invalid_transaction", "operations must not be empty"),
                false,
            );
        }
        Err(parse_error) => {
            return json_response(
                422,
                error(
                    "invalid_transaction",
                    &format!("invalid transaction document: {parse_error}"),
                ),
                false,
            );
        }
    };

    let operation_count = transaction.operations.len();
    let steps = transaction.operations;
    if steps.iter().any(|step| {
        matches!(
            step,
            TransactionStep::Command(TransactionCommand::SetConstraints {
                table: None,
                all: false,
                ..
            })
        )
    }) {
        return json_response(
            422,
            error(
                "invalid_transaction",
                "table is required when setting named constraints in a catalog transaction",
            ),
            false,
        );
    }
    let raft_precondition = match (if_match.clone(), if_none_match.clone()) {
        (None, None) => None,
        (if_match, if_none_match) => Some(RaftCommandPrecondition::Catalog {
            if_match,
            if_none_match,
        }),
    };
    let result = if let Some(raft) = raft {
        raft.propose_request(request, catalog, steps, raft_precondition, None)
            .map_err(CommitError::Raft)
    } else {
        match replication {
            Some(replication) => replication
                .propose_steps_with_catalog_preconditions(
                    catalog,
                    steps,
                    if_match.as_deref(),
                    if_none_match.as_deref(),
                )
                .map(|entry| entry.transaction_id)
                .map_err(CommitError::Replication),
            None => catalog
                .commit_steps_with_preconditions(
                    &steps,
                    if_match.as_deref(),
                    if_none_match.as_deref(),
                )
                .map_err(CommitError::Catalog),
        }
    };
    match result {
        Ok(transaction_id) => {
            let response = json_response(
                200,
                json!({
                    "committed": true,
                    "operations": operation_count,
                    "transaction_id": transaction_id,
                }),
                false,
            )
            .with_header(header(
                "X-Txbase-Transaction-Id",
                &transaction_id.to_string(),
            ));
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
            match snapshot.as_ref().unwrap_or(catalog).schema_representation() {
                Ok((_, tag)) => etag::with_tag(response, &tag),
                Err(catalog_error) => json_response(
                    500,
                    error("catalog_error", &catalog_error.to_string()),
                    false,
                ),
            }
        }
        Err(error) => commit_error_response(error),
    }
}

enum CommitError {
    Catalog(CatalogTransactionError),
    Replication(ReplicationError),
    Raft(HttpResponse),
}

fn commit_error_response(commit_error: CommitError) -> HttpResponse {
    match commit_error {
        CommitError::Replication(error) => super::replication::replication_error_response(error),
        CommitError::Raft(response) => response,
        CommitError::Catalog(CatalogTransactionError::PreconditionFailed { tag }) => {
            etag::with_tag(
                json_response(
                    412,
                    error(
                        "precondition_failed",
                        "If-Match or If-None-Match does not permit the current catalog representation",
                    ),
                    false,
                ),
                &tag,
            )
        }
        CommitError::Catalog(CatalogTransactionError::Invalid(message)) => {
            json_response(422, error("invalid_transaction", &message), false)
        }
        CommitError::Catalog(CatalogTransactionError::TableSetChanged) => json_response(
            409,
            error(
                "catalog_changed",
                "the catalog table set changed during the transaction",
            ),
            false,
        ),
        CommitError::Catalog(CatalogTransactionError::CatalogTagChanged { .. }) => json_response(
            409,
            error(
                "catalog_changed",
                "the catalog schema changed during the transaction",
            ),
            false,
        ),
        CommitError::Catalog(CatalogTransactionError::SidecarPreconditionFailed { .. }) => {
            json_response(
                409,
                error(
                    "catalog_changed",
                    "a catalog sidecar changed during the transaction",
                ),
                false,
            )
        }
        CommitError::Catalog(CatalogTransactionError::TransactionPreconditionFailed { .. }) => {
            json_response(
                409,
                error(
                    "catalog_changed",
                    "the catalog position changed during the transaction",
                ),
                false,
            )
        }
        CommitError::Catalog(CatalogTransactionError::Catalog(CatalogError::Invalid(message))) => {
            json_response(422, error("invalid_transaction", &message), false)
        }
        CommitError::Catalog(CatalogTransactionError::Catalog(catalog_error)) => json_response(
            500,
            error("catalog_error", &catalog_error.to_string()),
            false,
        ),
    }
}

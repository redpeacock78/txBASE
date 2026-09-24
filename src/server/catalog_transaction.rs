use super::{HttpResponse, error, etag, header, json_response, read_json_body, request_header};
use crate::catalog::{Catalog, CatalogTransactionError};
use crate::replication::{ReplicationError, ReplicationLog};
use crate::xbase::OperationBatch;
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
    let transaction = match serde_json::from_slice::<OperationBatch>(&body) {
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
    let operations = transaction.operations;
    let result = match replication {
        Some(replication) => replication
            .propose_with_catalog_preconditions(
                catalog,
                operations,
                if_match.as_deref(),
                if_none_match.as_deref(),
            )
            .map(|entry| entry.transaction_id)
            .map_err(CommitError::Replication),
        None => catalog
            .commit_operations_with_preconditions(
                &operations,
                if_match.as_deref(),
                if_none_match.as_deref(),
            )
            .map_err(CommitError::Catalog),
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
            match catalog.schema_representation() {
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
}

fn commit_error_response(commit_error: CommitError) -> HttpResponse {
    match commit_error {
        CommitError::Replication(error) => super::replication::replication_error_response(error),
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
        CommitError::Catalog(CatalogTransactionError::Catalog(catalog_error)) => json_response(
            500,
            error("catalog_error", &catalog_error.to_string()),
            false,
        ),
    }
}

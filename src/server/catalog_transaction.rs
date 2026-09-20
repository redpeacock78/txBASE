use super::{HttpResponse, error, etag, header, json_response, read_json_body, request_header};
use crate::catalog::{Catalog, CatalogTransactionError};
use crate::xbase::OperationIr;
use serde::Deserialize;
use serde_json::json;
use tiny_http::Request;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransactionRequest {
    operations: Vec<OperationIr>,
}

pub(super) fn response(request: &mut Request, catalog: &Catalog) -> HttpResponse {
    let if_none_match = request_header(request, "If-None-Match");
    let body = match read_json_body(request, "POST /transaction", false) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let transaction = match serde_json::from_slice::<TransactionRequest>(&body) {
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

    match catalog.commit_operations_with_if_none_match(&transaction.operations, if_none_match) {
        Ok(transaction_id) => {
            let response = json_response(
                200,
                json!({
                    "committed": true,
                    "operations": transaction.operations.len(),
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
        Err(CatalogTransactionError::PreconditionFailed { tag }) => etag::with_tag(
            json_response(
                412,
                error(
                    "precondition_failed",
                    "If-None-Match matches the current catalog representation",
                ),
                false,
            ),
            &tag,
        ),
        Err(CatalogTransactionError::Invalid(message)) => {
            json_response(422, error("invalid_transaction", &message), false)
        }
        Err(CatalogTransactionError::Catalog(catalog_error)) => json_response(
            500,
            error("catalog_error", &catalog_error.to_string()),
            false,
        ),
    }
}

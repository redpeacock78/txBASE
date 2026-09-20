use super::{HttpResponse, error, header, json_response, read_json_body};
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

    match catalog.commit_operations(&transaction.operations) {
        Ok(transaction_id) => json_response(
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
        )),
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

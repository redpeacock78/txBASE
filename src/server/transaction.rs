use super::{HttpResponse, dbf_error_response, error, etag, json_response, read_json_body};
use crate::dbf::DbfTable;
use crate::xbase::OperationIr;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use tiny_http::Request;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransactionRequest {
    operations: Vec<OperationIr>,
}

pub(super) fn response(
    request: &mut Request,
    table: &mut DbfTable,
    dbf_path: &Path,
) -> HttpResponse {
    if let Err(response) = etag::require_mutation_preconditions(request, table, true) {
        return response;
    }
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

    let original = table.clone();
    let mut working = original.clone();
    for operation in &transaction.operations {
        if let Err(dbf_error) = working.apply_operation(operation) {
            return dbf_error_response(dbf_error);
        }
    }
    if let Err(dbf_error) = working.save_with_wal(dbf_path) {
        let encoding = original.effective_encoding_override().map(str::to_owned);
        *table =
            DbfTable::from_path_with_encoding(dbf_path, encoding.as_deref()).unwrap_or(original);
        return dbf_error_response(dbf_error);
    }
    *table = working;
    etag::with_transaction(
        json_response(
            200,
            json!({
                "committed": true,
                "operations": transaction.operations.len(),
                "transaction_id": table.transaction_id(),
            }),
            false,
        ),
        table,
    )
}

use super::{HttpResponse, dbf_error_response, error, etag, json_response, read_json_body};
use crate::dbf::{DbfTable, DbfTransaction};
use crate::xbase::OperationBatch;
use serde_json::json;
use std::path::Path;
use tiny_http::Request;

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

    let original = table.clone();
    let mut working = DbfTransaction::from_table(dbf_path, original.clone());
    for operation in &transaction.operations {
        if let Err(dbf_error) = working.apply(operation) {
            return dbf_error_response(dbf_error);
        }
    }
    let working = match working.commit() {
        Ok(table) => table,
        Err(dbf_error) => {
            let encoding = original.effective_encoding_override().map(str::to_owned);
            *table = DbfTable::from_path_with_encoding(dbf_path, encoding.as_deref())
                .unwrap_or(original);
            return dbf_error_response(dbf_error);
        }
    };
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

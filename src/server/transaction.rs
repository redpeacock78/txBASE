use super::{HttpResponse, dbf_error_response, error, etag, json_response, read_json_body};
use crate::dbf::{DbfTable, DbfTransaction};
use crate::xbase::{TransactionBatch, TransactionCommand, TransactionStep};
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

    let original = table.clone();
    let mut working = DbfTransaction::from_table(dbf_path, original.clone());
    for step in &transaction.operations {
        let result = match step {
            TransactionStep::Mutation(operation) => working.apply(operation),
            TransactionStep::Command(TransactionCommand::SetConstraints {
                table,
                all,
                names,
                mode,
            }) => {
                if table.is_some() {
                    return json_response(
                        422,
                        error(
                            "invalid_transaction",
                            "table is not valid for a single-table transaction",
                        ),
                        false,
                    );
                }
                if *all {
                    working.set_all_constraints(*mode)
                } else {
                    let names = names.iter().map(String::as_str).collect::<Vec<_>>();
                    working.set_constraints(&names, *mode)
                }
            }
        };
        if let Err(dbf_error) = result {
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

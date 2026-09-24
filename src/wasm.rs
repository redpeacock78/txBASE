use crate::dbf::{DbfError, DbfTable};
use crate::query::{self, QueryError};
use crate::xbase::{OperationBatch, OperationIr};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

pub const ABI_VERSION: u16 = 1;

#[derive(Debug)]
pub enum WasmError {
    Database(DbfError),
    Query(QueryError),
    InvalidJson(serde_json::Error),
    InvalidOperation(String),
    Serialization(serde_json::Error),
}

impl Display for WasmError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "WASM database error: {error}"),
            Self::Query(error) => write!(formatter, "WASM query error: {error}"),
            Self::InvalidJson(error) => write!(formatter, "invalid WASM operation JSON: {error}"),
            Self::InvalidOperation(message) => {
                write!(formatter, "invalid WASM operation: {message}")
            }
            Self::Serialization(error) => write!(formatter, "WASM JSON encoding failed: {error}"),
        }
    }
}

impl Error for WasmError {}

impl From<DbfError> for WasmError {
    fn from(error: DbfError) -> Self {
        Self::Database(error)
    }
}

impl From<QueryError> for WasmError {
    fn from(error: QueryError) -> Self {
        Self::Query(error)
    }
}

pub struct WasmCore {
    table: DbfTable,
}

impl WasmCore {
    pub fn open_dbf(bytes: &[u8]) -> Result<Self, WasmError> {
        Ok(Self {
            table: DbfTable::from_bytes(bytes)?,
        })
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.table.to_bytes()
    }

    pub fn query_json(&self, body: &[u8]) -> Result<Vec<u8>, WasmError> {
        let request = query::parse(body)?;
        let records = query::execute_query(&self.table, &request)?;
        serde_json::to_vec(&records).map_err(WasmError::Serialization)
    }

    pub fn apply_operation_json(&mut self, body: &[u8]) -> Result<Vec<u8>, WasmError> {
        let operation =
            serde_json::from_slice::<OperationIr>(body).map_err(WasmError::InvalidJson)?;
        self.apply_operation(operation)?;
        Ok(self.snapshot())
    }

    pub fn apply_operations_json(&mut self, body: &[u8]) -> Result<Vec<u8>, WasmError> {
        let batch =
            serde_json::from_slice::<OperationBatch>(body).map_err(WasmError::InvalidJson)?;
        if batch.operations.is_empty() {
            return Err(WasmError::InvalidOperation(
                "operations must not be empty".into(),
            ));
        }
        let mut working = self.table.clone();
        for operation in batch.operations {
            working.apply_operation(&operation)?;
        }
        self.table = working;
        Ok(self.snapshot())
    }

    pub fn apply_operation(&mut self, operation: OperationIr) -> Result<(), WasmError> {
        self.table.apply_operation(&operation).map_err(Into::into)
    }
}

#[cfg(target_arch = "wasm32")]
mod bindings {
    use super::{ABI_VERSION, WasmCore, WasmError};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct WasmDatabase {
        core: WasmCore,
    }

    #[wasm_bindgen]
    impl WasmDatabase {
        #[wasm_bindgen(constructor)]
        pub fn new(dbf: &[u8]) -> Result<WasmDatabase, JsValue> {
            WasmCore::open_dbf(dbf)
                .map(|core| Self { core })
                .map_err(to_js_error)
        }

        pub fn abi_version() -> u16 {
            ABI_VERSION
        }

        pub fn snapshot(&self) -> Vec<u8> {
            self.core.snapshot()
        }

        pub fn query_json(&self, body: &[u8]) -> Result<Vec<u8>, JsValue> {
            self.core.query_json(body).map_err(to_js_error)
        }

        pub fn apply_operation_json(&mut self, body: &[u8]) -> Result<Vec<u8>, JsValue> {
            self.core.apply_operation_json(body).map_err(to_js_error)
        }

        pub fn apply_operations_json(&mut self, body: &[u8]) -> Result<Vec<u8>, JsValue> {
            self.core.apply_operations_json(body).map_err(to_js_error)
        }
    }

    fn to_js_error(error: WasmError) -> JsValue {
        JsValue::from_str(&error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xbase::MAX_OPERATION_BATCH;
    use serde_json::{Value, json};

    fn fixture() -> Vec<u8> {
        include_str!("../tests/fixtures/users.dbf.hex")
            .split_whitespace()
            .map(|token| u8::from_str_radix(token, 16).unwrap())
            .collect()
    }

    #[test]
    fn wasm_core_reuses_query_and_mutation_contracts() {
        let mut core = WasmCore::open_dbf(&fixture()).unwrap();
        let query = serde_json::to_vec(&json!({"sort": {"ID": 1}})).unwrap();
        let rows: Vec<Value> = serde_json::from_slice(&core.query_json(&query).unwrap()).unwrap();
        assert_eq!(rows.len(), 1);

        let operation = serde_json::to_vec(&json!({
            "method": "PATCH",
            "path": "/records/1",
            "body": {"NAME": "wasm"}
        }))
        .unwrap();
        core.apply_operation_json(&operation).unwrap();
        let rows: Vec<Value> = serde_json::from_slice(&core.query_json(b"{}").unwrap()).unwrap();
        assert_eq!(rows[0]["NAME"], "wasm");
    }

    #[test]
    fn wasm_core_batch_mutation_commits_all_operations() {
        let mut core = WasmCore::open_dbf(&fixture()).unwrap();
        let batch = serde_json::to_vec(&json!({
            "operations": [
                {"method": "PATCH", "path": "/records/1", "body": {"NAME": "Carol"}},
                {"method": "POST", "path": "/records", "body": {
                    "ID": 3, "NAME": "Dave", "AGE": 42, "ACTIVE": true
                }}
            ]
        }))
        .unwrap();

        core.apply_operations_json(&batch).unwrap();
        let rows: Vec<Value> =
            serde_json::from_slice(&core.query_json(br#"{"sort":{"ID":1}}"#).unwrap()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["NAME"], "Carol");
        assert_eq!(rows[1]["NAME"], "Dave");
    }

    #[test]
    fn wasm_core_batch_mutation_is_atomic_on_error() {
        let mut core = WasmCore::open_dbf(&fixture()).unwrap();
        let batch = serde_json::to_vec(&json!({
            "operations": [
                {"method": "PATCH", "path": "/records/1", "body": {"NAME": "Carol"}},
                {"method": "DELETE", "path": "/records/0"}
            ]
        }))
        .unwrap();

        let error = core.apply_operations_json(&batch).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("operation record id must be positive")
        );
        let rows: Vec<Value> = serde_json::from_slice(&core.query_json(b"{}").unwrap()).unwrap();
        assert_eq!(rows[0]["NAME"], "Alice");
    }

    #[test]
    fn wasm_core_rejects_empty_operation_batches() {
        let mut core = WasmCore::open_dbf(&fixture()).unwrap();
        let error = core
            .apply_operations_json(br#"{"operations":[]}"#)
            .unwrap_err();
        assert!(error.to_string().contains("must not be empty"));
    }

    #[test]
    fn wasm_core_rejects_oversized_operation_batches_without_mutation() {
        let mut core = WasmCore::open_dbf(&fixture()).unwrap();
        let before = core.snapshot();
        let operations = (0..=MAX_OPERATION_BATCH)
            .map(|_| json!({"method": "DELETE", "path": "/records/1"}))
            .collect::<Vec<_>>();
        let body = serde_json::to_vec(&json!({"operations": operations})).unwrap();

        let error = core.apply_operations_json(&body).unwrap_err();
        assert!(error.to_string().contains("operation count"));
        assert_eq!(core.snapshot(), before);
    }
}

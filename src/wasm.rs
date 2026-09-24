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
    Cancelled,
    InvalidJson(serde_json::Error),
    InvalidOperation(String),
    Serialization(serde_json::Error),
}

impl Display for WasmError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "WASM database error: {error}"),
            Self::Query(error) => write!(formatter, "WASM query error: {error}"),
            Self::Cancelled => write!(formatter, "WASM query stream was cancelled"),
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

pub struct WasmQueryStream {
    stream: query::OwnedQuerySnapshotStream,
    cancelled: bool,
}

impl WasmQueryStream {
    fn new(stream: query::OwnedQuerySnapshotStream) -> Self {
        Self {
            stream,
            cancelled: false,
        }
    }

    pub fn next_json(&mut self) -> Result<Option<String>, WasmError> {
        if self.cancelled {
            return Err(WasmError::Cancelled);
        }
        self.stream
            .next()
            .map(|item| item.map_err(WasmError::Query))
            .transpose()?
            .map(|value| serde_json::to_string(&value).map_err(WasmError::Serialization))
            .transpose()
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
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

    pub fn query_stream_json(&self, body: &[u8]) -> Result<WasmQueryStream, WasmError> {
        let request = query::parse(body)?;
        let stream = query::stream_query_snapshot_owned(&self.table, &request)?;
        Ok(WasmQueryStream::new(stream))
    }

    pub fn apply_operation_json(&mut self, body: &[u8]) -> Result<Vec<u8>, WasmError> {
        validate_json_input_size(body)?;
        let operation =
            serde_json::from_slice::<OperationIr>(body).map_err(WasmError::InvalidJson)?;
        self.apply_operation(operation)?;
        Ok(self.snapshot())
    }

    pub fn apply_operations_json(&mut self, body: &[u8]) -> Result<Vec<u8>, WasmError> {
        validate_json_input_size(body)?;
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

fn validate_json_input_size(body: &[u8]) -> Result<(), WasmError> {
    if body.len() > crate::MAX_JSON_INPUT_BYTES {
        return Err(WasmError::InvalidOperation(format!(
            "JSON input exceeds {} bytes",
            crate::MAX_JSON_INPUT_BYTES
        )));
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
mod bindings {
    use super::{ABI_VERSION, WasmCore, WasmError, WasmQueryStream as CoreQueryStream};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct WasmDatabase {
        core: WasmCore,
    }

    #[wasm_bindgen]
    pub struct WasmQueryStream {
        core: CoreQueryStream,
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

        pub fn query_stream_json(&self, body: &[u8]) -> Result<WasmQueryStream, JsValue> {
            self.core
                .query_stream_json(body)
                .map(|core| WasmQueryStream { core })
                .map_err(to_js_error)
        }

        pub fn apply_operation_json(&mut self, body: &[u8]) -> Result<Vec<u8>, JsValue> {
            self.core.apply_operation_json(body).map_err(to_js_error)
        }

        pub fn apply_operations_json(&mut self, body: &[u8]) -> Result<Vec<u8>, JsValue> {
            self.core.apply_operations_json(body).map_err(to_js_error)
        }
    }

    #[wasm_bindgen]
    impl WasmQueryStream {
        pub fn next_json(&mut self) -> Result<JsValue, JsValue> {
            self.core
                .next_json()
                .map(|value| {
                    value
                        .map(|value| JsValue::from_str(&value))
                        .unwrap_or(JsValue::NULL)
                })
                .map_err(to_js_error)
        }

        pub fn cancel(&mut self) {
            self.core.cancel();
        }
    }

    fn to_js_error(error: WasmError) -> JsValue {
        JsValue::from_str(&error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_JSON_INPUT_BYTES;
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
    fn wasm_core_query_stream_keeps_a_snapshot_and_emits_one_json_record() {
        let mut core = WasmCore::open_dbf(&fixture()).unwrap();
        let query = serde_json::to_vec(&json!({"projection": {"NAME": 1}})).unwrap();
        let mut stream = core.query_stream_json(&query).unwrap();

        let operation = serde_json::to_vec(&json!({
            "method": "PATCH",
            "path": "/records/1",
            "body": {"NAME": "changed"}
        }))
        .unwrap();
        core.apply_operation_json(&operation).unwrap();

        assert_eq!(
            stream.next_json().unwrap(),
            Some(r#"{"NAME":"Alice"}"#.into())
        );
        assert_eq!(stream.next_json().unwrap(), None);
    }

    #[test]
    fn wasm_core_query_stream_rejects_blocking_controls_and_supports_cancel() {
        let core = WasmCore::open_dbf(&fixture()).unwrap();
        let sorted = serde_json::to_vec(&json!({"sort": {"ID": 1}})).unwrap();
        assert!(core.query_stream_json(&sorted).is_err());

        let query = serde_json::to_vec(&json!({})).unwrap();
        let mut stream = core.query_stream_json(&query).unwrap();
        stream.cancel();
        assert!(matches!(stream.next_json(), Err(WasmError::Cancelled)));
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

    #[test]
    fn wasm_core_rejects_oversized_json_inputs_without_mutation() {
        let mut core = WasmCore::open_dbf(&fixture()).unwrap();
        let before = core.snapshot();
        let body = vec![b' '; MAX_JSON_INPUT_BYTES + 1];

        let operation_error = core.apply_operation_json(&body).unwrap_err();
        assert!(operation_error.to_string().contains("JSON input exceeds"));
        assert_eq!(core.snapshot(), before);

        let batch_error = core.apply_operations_json(&body).unwrap_err();
        assert!(batch_error.to_string().contains("JSON input exceeds"));
        assert_eq!(core.snapshot(), before);

        let query_error = core.query_json(&body).unwrap_err();
        assert!(query_error.to_string().contains("query document exceeds"));
        assert_eq!(core.snapshot(), before);
    }
}

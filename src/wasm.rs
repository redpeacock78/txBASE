use crate::dbf::{DbfError, DbfTable};
use crate::query::{self, QueryError};
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::{Map, Value};
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

    pub fn apply_operation(&mut self, operation: OperationIr) -> Result<(), WasmError> {
        match operation.method {
            OperationMethod::Post => {
                require_path(&operation.path, "/records")?;
                let values = object_body(operation.body)?;
                self.table.insert_record(values)?;
            }
            OperationMethod::Put => {
                let id = record_id(&operation.path)?;
                let values = object_body(operation.body)?;
                self.table.replace_record(id, values)?;
            }
            OperationMethod::Patch => {
                let id = record_id(&operation.path)?;
                let patch = object_body(operation.body)?;
                self.table.patch_record(id, patch)?;
            }
            OperationMethod::Delete => {
                let id = record_id(&operation.path)?;
                if operation.body.is_some() {
                    return Err(WasmError::InvalidOperation(
                        "DELETE operation must not have a body".into(),
                    ));
                }
                self.table.delete_record(id)?;
            }
            OperationMethod::Get | OperationMethod::Query => {
                return Err(WasmError::InvalidOperation(
                    "read operations must use query_json".into(),
                ));
            }
        }
        Ok(())
    }
}

fn object_body(body: Option<Value>) -> Result<Map<String, Value>, WasmError> {
    body.and_then(|body| body.as_object().cloned())
        .ok_or_else(|| WasmError::InvalidOperation("operation body must be a JSON object".into()))
}

fn require_path(actual: &str, expected: &str) -> Result<(), WasmError> {
    if actual == expected {
        Ok(())
    } else {
        Err(WasmError::InvalidOperation(format!(
            "operation path must be {expected}"
        )))
    }
}

fn record_id(path: &str) -> Result<usize, WasmError> {
    let value = path
        .strip_prefix("/records/")
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .ok_or_else(|| WasmError::InvalidOperation("operation path must be /records/:id".into()))?;
    let id = value
        .parse::<usize>()
        .map_err(|_| WasmError::InvalidOperation("record id must be a positive integer".into()))?;
    if id == 0 {
        return Err(WasmError::InvalidOperation(
            "record id must be a positive integer".into(),
        ));
    }
    Ok(id)
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
    }

    fn to_js_error(error: WasmError) -> JsValue {
        JsValue::from_str(&error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}

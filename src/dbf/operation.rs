use super::{DbfError, DbfTable};
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::{Map, Value};

impl DbfTable {
    pub(crate) fn apply_operation(&mut self, operation: &OperationIr) -> Result<(), DbfError> {
        match operation.method {
            OperationMethod::Post => {
                if operation.path != "/records" {
                    return Err(DbfError::Invalid(
                        "operation POST path must be /records".into(),
                    ));
                }
                let values = operation_object(operation, "POST")?;
                self.insert_record(values)?;
            }
            OperationMethod::Put => {
                let number = operation_record_id(&operation.path)?;
                self.replace_record(number, operation_object(operation, "PUT")?)?;
            }
            OperationMethod::Patch => {
                let number = operation_record_id(&operation.path)?;
                self.patch_record(number, operation_object(operation, "PATCH")?)?;
            }
            OperationMethod::Delete => {
                if operation.body.is_some() {
                    return Err(DbfError::Invalid(
                        "operation DELETE body must be absent".into(),
                    ));
                }
                self.delete_record(operation_record_id(&operation.path)?)?;
            }
            OperationMethod::Get | OperationMethod::Query => {
                return Err(DbfError::Invalid(
                    "read operation cannot be replayed from a mutation WAL".into(),
                ));
            }
        }
        Ok(())
    }
}

fn operation_object(operation: &OperationIr, method: &str) -> Result<Map<String, Value>, DbfError> {
    operation
        .body
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| DbfError::Invalid(format!("operation {method} body must be an object")))
}

fn operation_record_id(path: &str) -> Result<usize, DbfError> {
    let Some(raw_id) = path.strip_prefix("/records/") else {
        return Err(DbfError::Invalid(
            "operation path must be /records/{id}".into(),
        ));
    };
    let id = raw_id
        .parse::<usize>()
        .map_err(|_| DbfError::Invalid("operation record id is invalid".into()))?;
    (id > 0)
        .then_some(id)
        .ok_or_else(|| DbfError::Invalid("operation record id must be positive".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Vec<u8> {
        include_str!("../../tests/fixtures/users.dbf.hex")
            .split_whitespace()
            .map(|token| u8::from_str_radix(token, 16).unwrap())
            .collect()
    }

    fn operation(method: OperationMethod, path: &str, body: Option<Value>) -> OperationIr {
        OperationIr {
            method,
            path: path.into(),
            body,
        }
    }

    #[test]
    fn applies_all_mutation_methods_through_one_contract() {
        let mut table = DbfTable::from_bytes(&fixture()).unwrap();
        let inserted_number = table.records().len() + 1;
        table
            .apply_operation(&operation(
                OperationMethod::Patch,
                "/records/1",
                Some(json!({"NAME": "Carol"})),
            ))
            .unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], "Carol");

        table
            .apply_operation(&operation(
                OperationMethod::Put,
                "/records/1",
                Some(json!({"ID": 1, "NAME": "Dora", "AGE": 31, "ACTIVE": true})),
            ))
            .unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], "Dora");

        table
            .apply_operation(&operation(
                OperationMethod::Post,
                "/records",
                Some(json!({"ID": 2, "NAME": "Eve", "AGE": 22, "ACTIVE": true})),
            ))
            .unwrap();
        assert_eq!(table.records().len(), inserted_number);

        table
            .apply_operation(&operation(
                OperationMethod::Delete,
                &format!("/records/{inserted_number}"),
                None,
            ))
            .unwrap();
        assert!(table.records()[inserted_number - 1].deleted);
    }

    #[test]
    fn rejects_invalid_operation_shapes_without_mutating_the_table() {
        let mut table = DbfTable::from_bytes(&fixture()).unwrap();
        let before = table.active_json();

        for operation in [
            operation(OperationMethod::Get, "/records", None),
            operation(OperationMethod::Post, "/wrong", Some(json!({}))),
            operation(OperationMethod::Delete, "/records/1", Some(json!({}))),
        ] {
            assert!(table.apply_operation(&operation).is_err());
        }

        assert_eq!(table.active_json(), before);
    }
}

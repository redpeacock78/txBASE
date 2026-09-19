use super::wal::{
    decode_operation_payload, decode_wal_payload, delta_payload, memo_snapshot_payload,
    snapshot_payload,
};
use super::{
    DbfError, DbfTable, MemoFile, PersistedState, find_memo_path, save_bytes_to, transaction_error,
};
use crate::transaction::{FileWal, Wal};
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

impl DbfTable {
    pub(super) fn load_path(path: &Path) -> Result<Self, DbfError> {
        let dbf = fs::read(path)?;
        let mut table = Self::from_bytes(&dbf)?;
        if table.has_sidecar_fields() {
            if let Some(memo_path) = find_memo_path(path) {
                let memo = MemoFile::open(&memo_path, table.header.version)?;
                table.resolve_memos(&memo)?;
                table.memo = Some(memo);
            }
        }
        table.source = Some(PersistedState {
            path: path.to_path_buf(),
            dbf,
            memo: table.memo.as_ref().map(|memo| memo.bytes.clone()),
        });
        Ok(table)
    }

    pub(super) fn recover_wal(path: &Path) -> Result<bool, DbfError> {
        let wal_path = path.with_extension("txbase.wal");
        if !wal_path.exists() {
            return Ok(false);
        }
        let mut wal = FileWal::open(&wal_path).map_err(transaction_error)?;
        if wal.records().is_empty() {
            finish_recovery(wal, &wal_path);
            return Ok(false);
        }
        let index_payload = wal.records().iter().rev().find_map(|(_, payload)| {
            crate::index::decode_snapshot_payload(payload)
                .ok()
                .flatten()
        });
        let snapshot =
            wal.records().iter().rev().find_map(|(_, payload)| {
                match decode_wal_payload(path, payload) {
                    Ok(Some(snapshot)) => Some(Ok(snapshot)),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                }
            });
        if let Some(snapshot) = snapshot {
            let snapshot = snapshot?;
            Self::from_bytes(&snapshot.dbf)?;
            if let Some(memo) = &snapshot.memo {
                let memo_path = find_memo_path(path)
                    .unwrap_or_else(|| path.with_extension(memo.format.extension()));
                save_bytes_to(&memo_path, &memo.bytes, "txbase.memo.tmp")?;
            }
            save_bytes_to(path, &snapshot.dbf, "txbase.tmp")?;
            if let Some(index_payload) = &index_payload {
                let _ = crate::index::apply_snapshot_payload(path, index_payload);
            }
            finish_recovery(wal, &wal_path);
            return Ok(true);
        }

        let operation = wal.records().iter().rev().find_map(|(_, payload)| {
            match decode_operation_payload(payload) {
                Ok(Some(operation)) => Some(Ok(operation)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            }
        });
        let Some(operation) = operation else {
            return Ok(false);
        };
        let operation = operation?;
        let mut table = Self::load_path(path)?;
        table.apply_operation(&operation)?;
        let memo_snapshot = table.apply_memo_updates(path)?;
        let full_payload = match &memo_snapshot {
            Some(memo) => memo_snapshot_payload(&table.bytes, memo)?,
            None => snapshot_payload(&table.bytes),
        };
        let payload = delta_payload(&table, path, memo_snapshot.as_ref(), full_payload.len())?
            .unwrap_or(full_payload);
        wal.append(&payload).map_err(transaction_error)?;
        wal.sync().map_err(transaction_error)?;
        if let Some(memo) = &memo_snapshot {
            let memo_path = find_memo_path(path)
                .unwrap_or_else(|| path.with_extension(memo.format.extension()));
            save_bytes_to(&memo_path, &memo.bytes, "txbase.memo.tmp")?;
        }
        save_bytes_to(path, &table.bytes, "txbase.tmp")?;
        finish_recovery(wal, &wal_path);
        Ok(true)
    }

    fn apply_operation(&mut self, operation: &OperationIr) -> Result<(), DbfError> {
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

fn finish_recovery(mut wal: FileWal, wal_path: &Path) {
    if wal.clear().is_ok() {
        drop(wal);
        let _ = fs::remove_file(wal_path);
    }
}

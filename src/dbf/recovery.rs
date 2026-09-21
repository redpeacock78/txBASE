use super::persistence::{next_transaction_id, read_transaction_state, write_transaction_state};
use super::schema_metadata::read_schema_metadata;
use super::wal::{
    decode_operation_payload, decode_transaction_id_payload, decode_wal_payload, delta_payload,
    memo_snapshot_payload, snapshot_payload, transaction_id_payload,
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
    pub(super) fn load_path_with_encoding(
        path: &Path,
        requested_encoding: Option<&str>,
    ) -> Result<Self, DbfError> {
        let dbf = fs::read(path)?;
        let transaction_id = read_transaction_state(path)?;
        let schema = read_schema_metadata(path)?;
        let encoding_override = requested_encoding.or_else(|| {
            schema
                .as_ref()
                .and_then(|(metadata, _)| metadata.encoding())
        });
        let mut table = Self::from_bytes_with_encoding(&dbf, encoding_override)?;
        if let Some((metadata, _)) = &schema {
            metadata.validate_fields(&table.fields)?;
            table.schema = Some(metadata.clone());
        }
        if table.has_sidecar_fields() {
            if let Some(memo_path) = find_memo_path(path) {
                let memo = MemoFile::open(&memo_path, table.header.version)?;
                table.resolve_memos(&memo)?;
                table.memo = Some(memo);
            }
        }
        if let Some(metadata) = &table.schema {
            metadata.validate_records(&table.records)?;
        }
        table.source = Some(PersistedState {
            path: path.to_path_buf(),
            dbf,
            memo: find_memo_path(path).map(fs::read).transpose()?,
            schema: schema.map(|(_, bytes)| bytes),
            transaction_id,
        });
        table.transaction_id = transaction_id;
        Ok(table)
    }

    pub(super) fn bind_schema_if_present(&mut self, path: &Path) -> Result<(), DbfError> {
        if self.source.is_some() || self.schema.is_some() {
            return Ok(());
        }
        let Some((metadata, _)) = read_schema_metadata(path)? else {
            return Ok(());
        };
        metadata.validate_fields(&self.fields)?;
        if self.has_sidecar_fields() {
            if let Some(memo_path) = find_memo_path(path) {
                let memo = MemoFile::open(&memo_path, self.header.version)?;
                self.resolve_memos(&memo)?;
                self.memo = Some(memo);
            }
        }
        metadata.validate_records(&self.records)?;
        self.schema = Some(metadata);
        self.encoding_override = self
            .schema
            .as_ref()
            .and_then(|schema| schema.encoding())
            .map(ToOwned::to_owned);
        Ok(())
    }

    pub(super) fn recover_wal_with_encoding(
        path: &Path,
        encoding_override: Option<&str>,
    ) -> Result<bool, DbfError> {
        let wal_path = path.with_extension("txbase.wal");
        if !wal_path.exists() {
            return Ok(false);
        }
        let mut wal = FileWal::open(&wal_path).map_err(transaction_error)?;
        if wal.records().is_empty() {
            finish_recovery(wal, &wal_path);
            return Ok(false);
        }
        let index_payload = find_index_payload(&wal)?;
        let transaction_id = find_transaction_id(&wal)?;
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
            Self::from_bytes_with_encoding(&snapshot.dbf, encoding_override)?;
            if let Some(memo) = &snapshot.memo {
                let memo_path = find_memo_path(path)
                    .unwrap_or_else(|| path.with_extension(memo.format.extension()));
                save_bytes_to(&memo_path, &memo.bytes, "txbase.memo.tmp")?;
            }
            save_bytes_to(path, &snapshot.dbf, "txbase.tmp")?;
            if let Some(index_payload) = &index_payload {
                crate::index::apply_snapshot_payload(path, index_payload)
                    .map_err(super::index_error)?;
            }
            if let Some(transaction_id) = transaction_id {
                persist_recovered_transaction_id(path, transaction_id)?;
                super::mvcc::commit_recovered_snapshot(
                    path,
                    transaction_id,
                    &snapshot.dbf,
                    snapshot.memo.as_ref(),
                )?;
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
            if transaction_id.is_some() {
                finish_recovery(wal, &wal_path);
            }
            return Ok(false);
        };
        let operation = operation?;
        let transaction_id = match transaction_id {
            Some(transaction_id) => transaction_id,
            None => next_transaction_id(read_transaction_state(path)?)?,
        };
        let transaction_id = read_transaction_state(path)?
            .map_or(transaction_id, |current| current.max(transaction_id));
        let mut table = Self::load_path_with_encoding(path, encoding_override)?;
        table.apply_operation(&operation)?;
        let memo_snapshot = table.apply_memo_updates(path)?;
        let full_payload = match &memo_snapshot {
            Some(memo) => memo_snapshot_payload(&table.bytes, memo)?,
            None => snapshot_payload(&table.bytes),
        };
        let payload = delta_payload(&table, path, memo_snapshot.as_ref(), full_payload.len())?
            .unwrap_or(full_payload);
        let index_payload = crate::index::pending_snapshot_payload(
            path,
            &table,
            &table.bytes,
            memo_snapshot.as_ref().map(|memo| memo.bytes.as_slice()),
        )
        .map_err(super::index_error)?;
        wal.append(&transaction_id_payload(transaction_id))
            .map_err(transaction_error)?;
        wal.sync().map_err(transaction_error)?;
        wal.append(&payload).map_err(transaction_error)?;
        wal.sync().map_err(transaction_error)?;
        if let Some(index_payload) = &index_payload {
            wal.append(index_payload).map_err(transaction_error)?;
            wal.sync().map_err(transaction_error)?;
        }
        if let Some(memo) = &memo_snapshot {
            let memo_path = find_memo_path(path)
                .unwrap_or_else(|| path.with_extension(memo.format.extension()));
            save_bytes_to(&memo_path, &memo.bytes, "txbase.memo.tmp")?;
        }
        save_bytes_to(path, &table.bytes, "txbase.tmp")?;
        if let Some(index_payload) = &index_payload {
            crate::index::apply_snapshot_payload(path, index_payload)
                .map_err(super::index_error)?;
        }
        write_transaction_state(path, transaction_id)?;
        super::mvcc::commit_recovered_snapshot(
            path,
            transaction_id,
            &table.bytes,
            memo_snapshot.as_ref(),
        )?;
        finish_recovery(wal, &wal_path);
        Ok(true)
    }

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

fn find_index_payload(wal: &FileWal) -> Result<Option<Vec<u8>>, DbfError> {
    for (_, payload) in wal.records().iter().rev() {
        if let Some(payload) =
            crate::index::decode_snapshot_payload(payload).map_err(super::index_error)?
        {
            return Ok(Some(payload));
        }
    }
    Ok(None)
}

fn find_transaction_id(wal: &FileWal) -> Result<Option<u64>, DbfError> {
    for (_, payload) in wal.records().iter().rev() {
        if let Some(transaction_id) = decode_transaction_id_payload(payload)? {
            return Ok(Some(transaction_id));
        }
    }
    Ok(None)
}

fn persist_recovered_transaction_id(path: &Path, transaction_id: u64) -> Result<(), DbfError> {
    let transaction_id =
        read_transaction_state(path)?.map_or(transaction_id, |current| current.max(transaction_id));
    write_transaction_state(path, transaction_id)
}

fn finish_recovery(mut wal: FileWal, wal_path: &Path) {
    if wal.clear().is_ok() {
        drop(wal);
        let _ = fs::remove_file(wal_path);
    }
}

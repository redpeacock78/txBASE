use super::schema_metadata::schema_metadata_bytes;
use super::wal::{layout_change_payload, transaction_id_payload};
use super::*;

const TRANSACTION_STATE_MAGIC: &[u8; 4] = b"TXTS";
const TRANSACTION_STATE_VERSION: u8 = 1;

pub(super) fn transaction_state_path(path: &Path) -> PathBuf {
    path.with_extension("txbase.state")
}

pub(super) fn read_transaction_state(path: &Path) -> Result<Option<u64>, DbfError> {
    let state_path = transaction_state_path(path);
    let bytes = match fs::read(state_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let Some(body) = bytes.strip_prefix(TRANSACTION_STATE_MAGIC) else {
        return Err(DbfError::Invalid(
            "transaction state header is invalid".into(),
        ));
    };
    if body.len() != 9 {
        return Err(DbfError::Invalid(
            "transaction state payload is truncated".into(),
        ));
    }
    if body[0] != TRANSACTION_STATE_VERSION {
        return Err(DbfError::Invalid(format!(
            "unknown transaction state version {}",
            body[0]
        )));
    }
    let transaction_id = u64::from_le_bytes(
        body[1..9]
            .try_into()
            .expect("transaction state payload is fixed length"),
    );
    if transaction_id == 0 {
        return Err(DbfError::Invalid(
            "transaction state ID must be positive".into(),
        ));
    }
    Ok(Some(transaction_id))
}

pub(super) fn write_transaction_state(path: &Path, transaction_id: u64) -> Result<(), DbfError> {
    let bytes = transaction_state_bytes(transaction_id)?;
    save_bytes_to(&transaction_state_path(path), &bytes, "txbase.state.tmp")
}

pub(super) fn transaction_state_bytes(transaction_id: u64) -> Result<Vec<u8>, DbfError> {
    if transaction_id == 0 {
        return Err(DbfError::Invalid(
            "transaction state ID must be positive".into(),
        ));
    }
    let mut bytes = TRANSACTION_STATE_MAGIC.to_vec();
    bytes.push(TRANSACTION_STATE_VERSION);
    bytes.extend_from_slice(&transaction_id.to_le_bytes());
    Ok(bytes)
}

pub(super) fn next_transaction_id(current: Option<u64>) -> Result<u64, DbfError> {
    current
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| DbfError::Invalid("transaction ID exhausted".into()))
}

impl DbfTable {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn prepare_snapshot(&mut self, path: &Path) -> Result<PreparedSnapshot, DbfError> {
        self.bind_schema_if_present(path)?;
        self.ensure_source_current(path)?;
        let memo = self.apply_memo_updates(path)?.map(|memo| {
            let memo_path = find_memo_path(path)
                .unwrap_or_else(|| path.with_extension(memo.format.extension()));
            (memo_path, memo.bytes)
        });
        let index_payload = crate::index::pending_snapshot_payload(
            path,
            self,
            &self.bytes,
            memo.as_ref().map(|(_, bytes)| bytes.as_slice()),
        )
        .map_err(index_error)?;
        Ok(PreparedSnapshot {
            dbf: self.bytes.clone(),
            memo,
            index_payload,
        })
    }

    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<(), DbfError> {
        if !self.memo_updates.is_empty() {
            return Err(DbfError::Invalid(
                "memo updates require save_with_wal".into(),
            ));
        }
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        let _ = Self::recover_wal_with_encoding(path, self.encoding_override.as_deref())?;
        self.ensure_source_current(path)?;
        let index_payload = crate::index::pending_snapshot_payload(path, self, &self.bytes, None)
            .map_err(index_error)?;
        save_bytes_to(path, &self.bytes, "txbase.tmp")?;
        if let Some(index_payload) = &index_payload {
            crate::index::apply_snapshot_payload(path, index_payload).map_err(index_error)?;
        }
        Ok(())
    }

    fn ensure_source_current(&self, path: &Path) -> Result<(), DbfError> {
        if self.historical_snapshot {
            return Err(DbfError::Invalid(
                "historical MVCC snapshots are read-only".into(),
            ));
        }
        let Some(source) = &self.source else {
            return Ok(());
        };
        if source.path != path {
            return Ok(());
        }
        let current_dbf = fs::read(path)?;
        if current_dbf.as_slice() != source.dbf.as_slice() {
            return Err(DbfError::Invalid(
                "DBF changed since the table was loaded".into(),
            ));
        }
        if find_memo_path(path).map(fs::read).transpose()? != source.memo {
            return Err(DbfError::Invalid(
                "memo sidecar changed since the table was loaded".into(),
            ));
        }
        if schema_metadata_bytes(path)? != source.schema {
            return Err(DbfError::Invalid(
                "schema metadata changed since the table was loaded".into(),
            ));
        }
        if read_transaction_state(path)? != source.transaction_id {
            return Err(DbfError::Invalid(
                "transaction state changed since the table was loaded".into(),
            ));
        }
        Ok(())
    }

    pub fn save_with_wal(&mut self, path: impl AsRef<Path>) -> Result<(), DbfError> {
        self.save_with_wal_inner(path.as_ref(), None)
    }

    pub fn save_with_operation(
        &mut self,
        path: impl AsRef<Path>,
        operation: &OperationIr,
    ) -> Result<(), DbfError> {
        self.save_with_wal_inner(path.as_ref(), Some(operation))
    }

    fn save_with_wal_inner(
        &mut self,
        path: &Path,
        operation: Option<&OperationIr>,
    ) -> Result<(), DbfError> {
        let _lock = TableLock::acquire(path)?;
        let _ = Self::recover_wal_with_encoding(path, self.encoding_override.as_deref())?;
        let current_transaction_id = read_transaction_state(path)?;
        let before = Self::load_path_with_encoding(path, self.encoding_override.as_deref())?;
        self.bind_schema_if_present(path)?;
        self.ensure_source_current(path)?;
        let transaction_id = next_transaction_id(current_transaction_id)?;
        let mut prepared = self.clone();
        prepared.transaction_id = Some(transaction_id);
        let memo_snapshot = prepared.apply_memo_updates(path)?;
        let full_payload = match &memo_snapshot {
            Some(memo) => memo_snapshot_payload(&prepared.bytes, memo)?,
            None => snapshot_payload(&prepared.bytes),
        };
        let payload = delta_payload(&prepared, path, memo_snapshot.as_ref(), full_payload.len())?
            .unwrap_or(full_payload);
        let history_memo = memo_snapshot.clone().or_else(|| {
            prepared.memo.as_ref().map(|memo| MemoSnapshot {
                format: memo.format,
                bytes: memo.bytes.clone(),
            })
        });
        let schema_bytes = schema_metadata_bytes(path)?;
        let index_payload = crate::index::pending_snapshot_payload(
            path,
            &prepared,
            &prepared.bytes,
            memo_snapshot.as_ref().map(|memo| memo.bytes.as_slice()),
        )
        .map_err(index_error)?;
        let cdc_event =
            super::cdc::event_for_tables(transaction_id, &before, &prepared, self.layout_changed)?;
        let cdc_payload = super::cdc::payload(&cdc_event)?;
        let wal_path = path.with_extension("txbase.wal");
        let mut wal = FileWal::open(&wal_path).map_err(transaction_error)?;
        if self.layout_changed {
            wal.append(&layout_change_payload())
                .map_err(transaction_error)?;
            wal.sync().map_err(transaction_error)?;
        }
        if let Some(operation) = operation {
            wal.append(&operation_payload(operation)?)
                .map_err(transaction_error)?;
            wal.sync().map_err(transaction_error)?;
        }
        wal.append(&transaction_id_payload(transaction_id))
            .map_err(transaction_error)?;
        wal.sync().map_err(transaction_error)?;
        wal.append(&cdc_payload).map_err(transaction_error)?;
        wal.sync().map_err(transaction_error)?;
        wal.append(&payload).map_err(transaction_error)?;
        wal.sync().map_err(transaction_error)?;
        if let Some(index_payload) = &index_payload {
            wal.append(index_payload).map_err(transaction_error)?;
            wal.sync().map_err(transaction_error)?;
        }
        super::mvcc::prepare_snapshot(
            path,
            transaction_id,
            &prepared.bytes,
            history_memo.as_ref(),
            schema_bytes.as_deref(),
            self.layout_changed,
        )?;
        if let Some(memo) = &memo_snapshot {
            let memo_path = find_memo_path(path)
                .unwrap_or_else(|| path.with_extension(memo.format.extension()));
            save_bytes_to(&memo_path, &memo.bytes, "txbase.memo.tmp")?;
        }
        save_bytes_to(path, &prepared.bytes, "txbase.tmp")?;
        let index_result = if let Some(index_payload) = &index_payload {
            crate::index::apply_snapshot_payload(path, index_payload)
        } else {
            crate::index::refresh_if_present(path, &prepared)
        };
        index_result.map_err(index_error)?;
        write_transaction_state(path, transaction_id)?;
        super::mvcc::commit_snapshot(
            path,
            transaction_id,
            &prepared.bytes,
            history_memo.as_ref(),
            schema_bytes.as_deref(),
            self.layout_changed,
        )?;
        super::cdc::append(path, &cdc_event)?;
        wal.clear().map_err(transaction_error)?;
        drop(wal);
        fs::remove_file(wal_path)?;
        prepared.source = Some(PersistedState {
            path: path.to_path_buf(),
            dbf: prepared.bytes.clone(),
            memo: find_memo_path(path).map(fs::read).transpose()?,
            schema: schema_metadata_bytes(path)?,
            transaction_id: Some(transaction_id),
        });
        prepared.layout_changed = false;
        *self = prepared;
        Ok(())
    }
}

use super::*;

impl DbfTable {
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<(), DbfError> {
        if !self.memo_updates.is_empty() {
            return Err(DbfError::Invalid(
                "memo updates require save_with_wal".into(),
            ));
        }
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        let _ = Self::recover_wal(path)?;
        self.ensure_source_current(path)?;
        save_bytes_to(path, &self.bytes, "txbase.tmp")?;
        let _ = crate::index::refresh_if_present(path, self);
        Ok(())
    }

    fn ensure_source_current(&self, path: &Path) -> Result<(), DbfError> {
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
        if let Some(expected_memo) = &source.memo {
            let Some(memo_path) = find_memo_path(path) else {
                return Err(DbfError::Invalid(
                    "memo sidecar changed since the table was loaded".into(),
                ));
            };
            let current_memo = fs::read(memo_path)?;
            if current_memo.as_slice() != expected_memo.as_slice() {
                return Err(DbfError::Invalid(
                    "memo sidecar changed since the table was loaded".into(),
                ));
            }
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
        let _ = Self::recover_wal(path)?;
        self.ensure_source_current(path)?;
        let mut prepared = self.clone();
        let memo_snapshot = prepared.apply_memo_updates(path)?;
        let wal_path = path.with_extension("txbase.wal");
        let mut wal = FileWal::open(&wal_path).map_err(transaction_error)?;
        let full_payload = match &memo_snapshot {
            Some(memo) => memo_snapshot_payload(&prepared.bytes, memo)?,
            None => snapshot_payload(&prepared.bytes),
        };
        let payload = delta_payload(&prepared, path, memo_snapshot.as_ref(), full_payload.len())?
            .unwrap_or(full_payload);
        let index_payload = crate::index::pending_snapshot_payload(
            path,
            &prepared,
            &prepared.bytes,
            memo_snapshot.as_ref().map(|memo| memo.bytes.as_slice()),
        )
        .ok()
        .flatten();
        if let Some(operation) = operation {
            wal.append(&operation_payload(operation)?)
                .map_err(transaction_error)?;
            wal.sync().map_err(transaction_error)?;
        }
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
        save_bytes_to(path, &prepared.bytes, "txbase.tmp")?;
        let index_result = if let Some(index_payload) = &index_payload {
            crate::index::apply_snapshot_payload(path, index_payload)
        } else {
            crate::index::refresh_if_present(path, &prepared)
        };
        if index_result.is_ok() && wal.clear().is_ok() {
            drop(wal);
            let _ = fs::remove_file(wal_path);
        }
        prepared.source = Some(PersistedState {
            path: path.to_path_buf(),
            dbf: prepared.bytes.clone(),
            memo: prepared.memo.as_ref().map(|memo| memo.bytes.clone()),
        });
        *self = prepared;
        Ok(())
    }
}

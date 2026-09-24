use super::journal::{FileChange, commit_at};
use super::mvcc::{self, Snapshot};
use super::{Catalog, CatalogError, CatalogTransactionError};
use crate::dbf::DbfTable;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const TABLE_STATE_EXTENSIONS: [&str; 5] = [
    "txbase.mvcc",
    "txbase.cdc",
    "txbase.wal",
    "txbase.state",
    "txbase.idx",
];

impl Catalog {
    pub(crate) fn export_snapshot_at(&self, transaction_id: u64) -> Result<Vec<u8>, CatalogError> {
        let _lock = self.acquire_read_lock()?;
        mvcc::snapshot_bytes(&self.root, transaction_id)
    }

    pub(crate) fn export_current_snapshot(&self) -> Result<(u64, String, Vec<u8>), CatalogError> {
        let _lock = self.acquire_read_lock()?;
        let transaction_id = super::journal::read_transaction_id_locked(&self.root)?.unwrap_or(0);
        if transaction_id == 0 {
            return Err(CatalogError::Invalid(
                "cannot snapshot an empty catalog".into(),
            ));
        }
        let bytes = mvcc::snapshot_bytes(&self.root, transaction_id)?;
        let (_, schema_tag) = Self::snapshot_metadata(&bytes)?;
        Ok((transaction_id, schema_tag, bytes))
    }

    pub(crate) fn snapshot_metadata(bytes: &[u8]) -> Result<(u64, String), CatalogError> {
        let snapshot = mvcc::decode_single_snapshot(bytes)?;
        let schema = snapshot_schema_json(&snapshot)?;
        Ok((snapshot.transaction_id, super::representation_tag(&schema)))
    }

    pub(crate) fn install_snapshot_with_sidecar(
        &mut self,
        snapshot_bytes: &[u8],
        expected_current_transaction_id: u64,
        sidecar_name: &str,
        expected_sidecar: Option<Vec<u8>>,
        sidecar_after: Vec<u8>,
    ) -> Result<u64, CatalogTransactionError> {
        if self.is_historical() {
            return Err(CatalogTransactionError::Invalid(
                "historical catalog snapshots are read-only".into(),
            ));
        }
        let snapshot = mvcc::decode_single_snapshot(snapshot_bytes)
            .map_err(CatalogTransactionError::Catalog)?;
        let _ = snapshot_schema_json(&snapshot).map_err(CatalogTransactionError::Catalog)?;
        if snapshot.transaction_id <= expected_current_transaction_id {
            return Err(CatalogTransactionError::Invalid(format!(
                "catalog snapshot transaction ID {} is not newer than {}",
                snapshot.transaction_id, expected_current_transaction_id
            )));
        }

        let _lock = self
            .acquire_write_lock()
            .map_err(CatalogTransactionError::Catalog)?;
        let actual_current_transaction_id = super::journal::read_transaction_id_locked(&self.root)
            .map_err(CatalogTransactionError::Catalog)?
            .unwrap_or(0);
        if actual_current_transaction_id != expected_current_transaction_id {
            return Err(CatalogTransactionError::TransactionPreconditionFailed {
                expected: expected_current_transaction_id,
                actual: actual_current_transaction_id,
            });
        }

        let sidecar_path = super::transaction::sidecar_path(&self.root, sidecar_name)
            .map_err(CatalogTransactionError::Catalog)?;
        let actual_sidecar =
            read_optional(&sidecar_path).map_err(CatalogTransactionError::Catalog)?;
        if actual_sidecar != expected_sidecar {
            return Err(CatalogTransactionError::SidecarPreconditionFailed {
                name: sidecar_name.to_owned(),
            });
        }

        let mut names = self.table_names().into_iter().collect::<BTreeSet<_>>();
        names.extend(snapshot.tables.keys().cloned());
        let mut paths = BTreeSet::new();
        for entry in self.tables() {
            paths.extend(related_paths(&self.root, entry.name(), entry.path()));
        }
        for table in snapshot.tables.values() {
            let path = self.root.join(&table.file_name);
            paths.extend(related_paths(&self.root, &table_name(&path), &path));
        }
        paths.extend(
            fs::read_dir(&self.root)
                .map_err(|error| CatalogTransactionError::Catalog(CatalogError::Io(error)))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_file())
                .filter(|path| is_related_to_any(path, &names)),
        );

        let mut changes = BTreeMap::new();
        for path in paths {
            stage_change(&mut changes, path, None).map_err(CatalogTransactionError::Catalog)?;
        }
        for table in snapshot.tables.values() {
            let dbf_path = self.root.join(&table.file_name);
            stage_change(&mut changes, dbf_path.clone(), Some(table.dbf.clone()))
                .map_err(CatalogTransactionError::Catalog)?;
            if let Some((memo_tag, memo_bytes)) = &table.memo {
                stage_change(
                    &mut changes,
                    dbf_path.with_extension(
                        memo_extension(*memo_tag).map_err(CatalogTransactionError::Catalog)?,
                    ),
                    Some(memo_bytes.clone()),
                )
                .map_err(CatalogTransactionError::Catalog)?;
            }
            if let Some(schema) = &table.schema {
                stage_change(
                    &mut changes,
                    dbf_path.with_extension("txschema.json"),
                    Some(schema.clone()),
                )
                .map_err(CatalogTransactionError::Catalog)?;
            }
        }
        stage_change(
            &mut changes,
            mvcc::path_for(&self.root),
            Some(
                mvcc::encode_single_snapshot(&snapshot)
                    .map_err(CatalogTransactionError::Catalog)?,
            ),
        )
        .map_err(CatalogTransactionError::Catalog)?;
        stage_change(&mut changes, super::cdc::path_for(&self.root), None)
            .map_err(CatalogTransactionError::Catalog)?;
        stage_change(&mut changes, sidecar_path, Some(sidecar_after))
            .map_err(CatalogTransactionError::Catalog)?;

        let transaction_id = commit_at(
            &self.root,
            snapshot.transaction_id,
            changes.into_values().collect(),
        )
        .map_err(CatalogTransactionError::Catalog)?;
        self.tables = super::discovery::discover_tables(&self.root)
            .map_err(CatalogTransactionError::Catalog)?;
        Ok(transaction_id)
    }
}

fn snapshot_schema_json(snapshot: &Snapshot) -> Result<Value, CatalogError> {
    let mut tables = Vec::with_capacity(snapshot.tables.len());
    for (name, table) in &snapshot.tables {
        let loaded = DbfTable::from_catalog_snapshot(
            &table.dbf,
            table.memo.clone(),
            table.schema.as_deref(),
        )
        .map_err(|source| CatalogError::Table {
            name: name.clone(),
            source,
        })?;
        loaded.verify().map_err(|source| CatalogError::Table {
            name: name.clone(),
            source,
        })?;
        tables.push(json!({
            "name": name,
            "file": table.file_name,
            "schema": loaded.schema_json(),
        }));
    }
    Ok(json!({
        "format": "txbase-catalog",
        "transaction_id": snapshot.transaction_id,
        "tables": tables,
    }))
}

fn related_paths(root: &Path, name: &str, dbf_path: &Path) -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::new();
    paths.insert(dbf_path.to_path_buf());
    for extension in ["dbt", "DBT", "fpt", "FPT", "txschema.json"] {
        paths.insert(dbf_path.with_extension(extension));
    }
    for extension in TABLE_STATE_EXTENSIONS {
        paths.insert(dbf_path.with_extension(extension));
    }
    paths.extend(
        fs::read_dir(root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .filter(|path| is_related_to(path, name)),
    );
    paths
}

fn is_related_to_any(path: &Path, names: &BTreeSet<String>) -> bool {
    names.iter().any(|name| is_related_to(path, name))
}

fn is_related_to(path: &Path, name: &str) -> bool {
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let file_name = file_name.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    file_name == format!("{name}.dbf")
        || file_name == format!("{name}.dbt")
        || file_name == format!("{name}.fpt")
        || file_name == format!("{name}.txschema.json")
        || file_name.starts_with(&format!("{name}.txbase."))
}

fn table_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_owned()
}

fn memo_extension(tag: u8) -> Result<&'static str, CatalogError> {
    match tag {
        0 | 1 => Ok("dbt"),
        2 => Ok("fpt"),
        _ => Err(CatalogError::Invalid(format!(
            "unknown memo sidecar format tag {tag}"
        ))),
    }
}

fn stage_change(
    changes: &mut BTreeMap<PathBuf, FileChange>,
    path: PathBuf,
    after: Option<Vec<u8>>,
) -> Result<(), CatalogError> {
    let before = read_optional(&path)?;
    if before == after {
        return Ok(());
    }
    changes.insert(
        path.clone(),
        FileChange {
            target: path,
            before,
            after,
        },
    );
    Ok(())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, CatalogError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CatalogError::Io(error)),
    }
}

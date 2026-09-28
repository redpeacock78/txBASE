pub(crate) use super::journal::{CatalogReadLock, CatalogWriteLock, read_lock, write_lock};
use super::journal::{FileChange, commit, commit_files, next_transaction_id_locked};
use super::mvcc::{Snapshot, TableSnapshot};
use super::{Catalog, CatalogError, CatalogTransactionError};
use crate::dbf::DbfTable;
use crate::xbase::{OperationIr, OperationMethod, TransactionCommand, TransactionStep};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

struct SidecarChange {
    name: String,
    before: Option<Vec<u8>>,
    after: Box<dyn FnOnce(u64) -> Vec<u8> + Send>,
}

pub(super) struct PreparedSidecarChange {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Box<dyn FnOnce(u64) -> Vec<u8> + Send>,
}

#[derive(Clone, Copy)]
pub(crate) enum CommitPrecondition<'a> {
    Catalog {
        expected_transaction_id: Option<u64>,
        expected_catalog_tag: Option<&'a str>,
        if_match: Option<&'a str>,
        if_none_match: Option<&'a str>,
    },
    Table {
        expected_transaction_id: Option<u64>,
        expected_catalog_tag: Option<&'a str>,
        name: &'a str,
        if_match: Option<&'a str>,
        if_none_match: Option<&'a str>,
    },
}

impl Catalog {
    pub(crate) fn read_sidecar_bytes(
        &self,
        sidecar_name: &str,
    ) -> Result<Option<Vec<u8>>, CatalogError> {
        let path = sidecar_path(&self.root, sidecar_name)?;
        let _lock = self.acquire_read_lock()?;
        read_optional(&path)
    }

    pub(crate) fn replace_sidecar_without_transaction(
        &self,
        sidecar_name: &str,
        expected_sidecar: Option<Vec<u8>>,
        sidecar_after: Vec<u8>,
    ) -> Result<(), CatalogTransactionError> {
        self.update_sidecar_without_transaction(sidecar_name, expected_sidecar, Some(sidecar_after))
    }

    pub(crate) fn remove_sidecar_without_transaction(
        &self,
        sidecar_name: &str,
        expected_sidecar: Option<Vec<u8>>,
    ) -> Result<(), CatalogTransactionError> {
        self.update_sidecar_without_transaction(sidecar_name, expected_sidecar, None)
    }

    fn update_sidecar_without_transaction(
        &self,
        sidecar_name: &str,
        expected_sidecar: Option<Vec<u8>>,
        sidecar_after: Option<Vec<u8>>,
    ) -> Result<(), CatalogTransactionError> {
        self.update_sidecars_without_transaction(vec![(
            sidecar_name.to_owned(),
            expected_sidecar,
            sidecar_after,
        )])
    }

    pub(crate) fn update_sidecars_without_transaction(
        &self,
        sidecars: Vec<(String, Option<Vec<u8>>, Option<Vec<u8>>)>,
    ) -> Result<(), CatalogTransactionError> {
        self.update_sidecars_at_transaction(None, sidecars)
    }

    pub(crate) fn update_sidecars_at_transaction(
        &self,
        expected_transaction_id: Option<u64>,
        sidecars: Vec<(String, Option<Vec<u8>>, Option<Vec<u8>>)>,
    ) -> Result<(), CatalogTransactionError> {
        if self.is_historical() {
            return Err(CatalogTransactionError::Invalid(
                "historical catalog snapshots are read-only".into(),
            ));
        }
        let _lock = self
            .acquire_write_lock()
            .map_err(CatalogTransactionError::Catalog)?;
        if let Some(expected) = expected_transaction_id {
            let actual = super::journal::read_transaction_id_locked(&self.root)
                .map_err(CatalogTransactionError::Catalog)?
                .unwrap_or(0);
            if actual != expected {
                return Err(CatalogTransactionError::TransactionPreconditionFailed {
                    expected,
                    actual,
                });
            }
        }
        let mut changes = Vec::with_capacity(sidecars.len());
        let mut names = BTreeSet::new();
        for (name, expected, after) in sidecars {
            if !names.insert(name.clone()) {
                return Err(CatalogTransactionError::Invalid(format!(
                    "duplicate catalog sidecar: {name}"
                )));
            }
            let path = sidecar_path(&self.root, &name).map_err(CatalogTransactionError::Catalog)?;
            let actual = read_optional(&path).map_err(CatalogTransactionError::Catalog)?;
            if actual != expected {
                return Err(CatalogTransactionError::SidecarPreconditionFailed { name });
            }
            if actual != after {
                changes.push(FileChange {
                    target: path,
                    before: actual,
                    after,
                });
            }
        }
        if changes.is_empty() {
            return Ok(());
        }
        commit_files(&self.root, changes).map_err(CatalogTransactionError::Catalog)
    }

    #[cfg(test)]
    pub(crate) fn commit_operations_with_preconditions(
        &self,
        operations: &[OperationIr],
        if_match: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<u64, CatalogTransactionError> {
        self.commit_operations_internal(
            operations,
            Some(CommitPrecondition::Catalog {
                expected_transaction_id: None,
                expected_catalog_tag: None,
                if_match,
                if_none_match,
            }),
            None,
        )
    }

    pub(crate) fn commit_steps_with_preconditions(
        &self,
        steps: &[TransactionStep],
        if_match: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<u64, CatalogTransactionError> {
        self.commit_steps_internal(
            steps,
            Some(CommitPrecondition::Catalog {
                expected_transaction_id: None,
                expected_catalog_tag: None,
                if_match,
                if_none_match,
            }),
            None,
        )
        .map(|(transaction_id, _)| transaction_id)
    }

    pub(crate) fn commit_steps_with_sidecar(
        &self,
        steps: &[TransactionStep],
        sidecar_name: &str,
        expected_sidecar: Option<Vec<u8>>,
        sidecar_after: Vec<u8>,
    ) -> Result<u64, CatalogTransactionError> {
        self.commit_steps_internal(
            steps,
            None,
            Some(SidecarChange {
                name: sidecar_name.to_owned(),
                before: expected_sidecar,
                after: Box::new(move |_| sidecar_after),
            }),
        )
        .map(|(transaction_id, _)| transaction_id)
    }

    pub(crate) fn commit_steps_with_preconditions_and_sidecar(
        &self,
        steps: &[TransactionStep],
        if_match: Option<&str>,
        if_none_match: Option<&str>,
        sidecar_name: &str,
        expected_sidecar: Option<Vec<u8>>,
        sidecar_after: Vec<u8>,
    ) -> Result<u64, CatalogTransactionError> {
        self.commit_steps_internal(
            steps,
            Some(CommitPrecondition::Catalog {
                expected_transaction_id: None,
                expected_catalog_tag: None,
                if_match,
                if_none_match,
            }),
            Some(SidecarChange {
                name: sidecar_name.to_owned(),
                before: expected_sidecar,
                after: Box::new(move |_| sidecar_after),
            }),
        )
        .map(|(transaction_id, _)| transaction_id)
    }

    pub(crate) fn commit_steps_with_sidecar_factory<F>(
        &self,
        steps: &[TransactionStep],
        precondition: CommitPrecondition<'_>,
        sidecar_name: &str,
        expected_sidecar: Option<Vec<u8>>,
        sidecar_after: F,
    ) -> Result<(u64, Vec<u8>), CatalogTransactionError>
    where
        F: FnOnce(u64) -> Vec<u8> + Send + 'static,
    {
        let (transaction_id, sidecar_after) = self.commit_steps_internal(
            steps,
            Some(precondition),
            Some(SidecarChange {
                name: sidecar_name.to_owned(),
                before: expected_sidecar,
                after: Box::new(sidecar_after),
            }),
        )?;
        let Some(sidecar_after) = sidecar_after else {
            return Err(CatalogTransactionError::Invalid(
                "catalog transaction did not produce its state sidecar".into(),
            ));
        };
        Ok((transaction_id, sidecar_after))
    }

    pub(crate) fn commit_operations_with_sidecar_and_table_preconditions(
        &self,
        operations: &[OperationIr],
        table_name: &str,
        (if_match, if_none_match): (Option<&str>, Option<&str>),
        sidecar_name: &str,
        expected_sidecar: Option<Vec<u8>>,
        sidecar_after: Vec<u8>,
    ) -> Result<u64, CatalogTransactionError> {
        self.commit_operations_internal(
            operations,
            Some(CommitPrecondition::Table {
                expected_transaction_id: None,
                expected_catalog_tag: None,
                name: table_name,
                if_match,
                if_none_match,
            }),
            Some(SidecarChange {
                name: sidecar_name.to_owned(),
                before: expected_sidecar,
                after: Box::new(move |_| sidecar_after),
            }),
        )
    }

    fn commit_operations_internal(
        &self,
        operations: &[OperationIr],
        precondition: Option<CommitPrecondition<'_>>,
        sidecar: Option<SidecarChange>,
    ) -> Result<u64, CatalogTransactionError> {
        let steps = operations
            .iter()
            .cloned()
            .map(TransactionStep::Mutation)
            .collect::<Vec<_>>();
        self.commit_steps_internal(&steps, precondition, sidecar)
            .map(|(transaction_id, _)| transaction_id)
    }

    fn commit_steps_internal(
        &self,
        steps: &[TransactionStep],
        precondition: Option<CommitPrecondition<'_>>,
        sidecar: Option<SidecarChange>,
    ) -> Result<(u64, Option<Vec<u8>>), CatalogTransactionError> {
        if steps.is_empty() {
            return Err(CatalogTransactionError::Invalid(
                "operations must not be empty".into(),
            ));
        }
        if self.is_historical() {
            return Err(CatalogTransactionError::Invalid(
                "historical catalog snapshots are read-only".into(),
            ));
        }
        let _lock = self
            .acquire_write_lock()
            .map_err(CatalogTransactionError::Catalog)?;
        if let Some(precondition) = precondition {
            let expected_transaction_id = match precondition {
                CommitPrecondition::Catalog {
                    expected_transaction_id,
                    ..
                }
                | CommitPrecondition::Table {
                    expected_transaction_id,
                    ..
                } => expected_transaction_id,
            };
            if let Some(expected) = expected_transaction_id {
                let actual = super::journal::read_transaction_id_locked(&self.root)
                    .map_err(CatalogTransactionError::Catalog)?
                    .unwrap_or(0);
                if actual != expected {
                    return Err(CatalogTransactionError::TransactionPreconditionFailed {
                        expected,
                        actual,
                    });
                }
            }
        }
        let sidecar_change = sidecar
            .map(|sidecar| {
                let path = sidecar_path(&self.root, &sidecar.name)
                    .map_err(CatalogTransactionError::Catalog)?;
                let actual = read_optional(&path).map_err(CatalogTransactionError::Catalog)?;
                if actual != sidecar.before {
                    return Err(CatalogTransactionError::SidecarPreconditionFailed {
                        name: sidecar.name,
                    });
                }
                Ok(PreparedSidecarChange {
                    path,
                    before: actual,
                    after: sidecar.after,
                })
            })
            .transpose()?;
        let mut before = BTreeMap::<String, DbfTable>::new();
        for entry in self.tables() {
            before.insert(
                entry.name().to_owned(),
                self.open_table_unlocked(entry.name())
                    .map_err(CatalogTransactionError::Catalog)?,
            );
        }
        if let Some(precondition) = precondition {
            let expected_catalog_tag = match precondition {
                CommitPrecondition::Catalog {
                    expected_catalog_tag,
                    ..
                }
                | CommitPrecondition::Table {
                    expected_catalog_tag,
                    ..
                } => expected_catalog_tag,
            };
            let schema_tag = if expected_catalog_tag.is_some()
                || matches!(precondition, CommitPrecondition::Catalog { .. })
            {
                Some(
                    self.schema_representation_unlocked()
                        .map_err(CatalogTransactionError::Catalog)?
                        .1,
                )
            } else {
                None
            };
            if let (Some(expected), Some(actual)) = (expected_catalog_tag, schema_tag.as_deref()) {
                if expected != actual {
                    return Err(CatalogTransactionError::CatalogTagChanged {
                        expected: expected.to_owned(),
                        actual: actual.to_owned(),
                    });
                }
            }
            let (if_match, if_none_match, tag) = match precondition {
                CommitPrecondition::Catalog {
                    if_match,
                    if_none_match,
                    ..
                } => {
                    let tag = schema_tag.expect("catalog precondition loaded its schema tag");
                    (if_match, if_none_match, tag)
                }
                CommitPrecondition::Table {
                    name,
                    if_match,
                    if_none_match,
                    ..
                } => {
                    let table = before.get(name).ok_or_else(|| {
                        CatalogTransactionError::Invalid(format!("table not found: {name}"))
                    })?;
                    (
                        if_match,
                        if_none_match,
                        format!("\"txbase-{:016x}\"", table.representation_hash()),
                    )
                }
            };
            if if_match.is_some_and(|value| !matches_if_match(value, &tag))
                || if_none_match.is_some_and(|value| matches_if_none_match(value, &tag))
            {
                return Err(CatalogTransactionError::PreconditionFailed { tag });
            }
        }
        let mut tables = before.clone();
        let mut touched = BTreeSet::new();
        let mut deferred_constraints = super::constraints::initial_deferred_constraints(&tables)
            .map_err(CatalogTransactionError::Catalog)?;
        for step in steps {
            match step {
                TransactionStep::Mutation(operation) => apply_operation_to_tables(
                    self,
                    &mut tables,
                    &mut touched,
                    operation,
                    &deferred_constraints,
                )?,
                TransactionStep::Command(TransactionCommand::SetConstraints {
                    table,
                    all,
                    names,
                    mode,
                }) => {
                    deferred_constraints = super::constraints::change_constraint_modes(
                        &tables,
                        &deferred_constraints,
                        table.as_deref(),
                        names,
                        *all,
                        *mode,
                    )?;
                }
            }
        }
        self.commit_loaded_tables_locked(
            before,
            tables,
            touched,
            false,
            Vec::new(),
            sidecar_change,
            &deferred_constraints,
        )
    }
}

pub(super) fn apply_operation_to_tables(
    catalog: &Catalog,
    tables: &mut BTreeMap<String, DbfTable>,
    touched: &mut BTreeSet<String>,
    operation: &OperationIr,
    deferred_constraints: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), CatalogTransactionError> {
    let Some((name, local_path)) = transaction_operation_path(&operation.path) else {
        return Err(CatalogTransactionError::Invalid(format!(
            "operation path is not a named record route: {}",
            operation.path
        )));
    };
    if !matches!(
        operation.method,
        OperationMethod::Post
            | OperationMethod::Put
            | OperationMethod::Patch
            | OperationMethod::Delete
    ) {
        return Err(CatalogTransactionError::Invalid(format!(
            "operation method {} is not a mutation",
            operation.method.as_str()
        )));
    }
    if catalog.table_path(name).is_none() {
        return Err(CatalogTransactionError::Invalid(format!(
            "table not found: {name}"
        )));
    }
    let before_table = tables
        .get(name)
        .expect("catalog transaction table was inserted");
    let before_snapshot =
        super::constraint_actions::snapshot_referenced_table(name, before_table, tables)
            .map_err(CatalogTransactionError::Catalog)?;
    let table = tables
        .get_mut(name)
        .expect("catalog transaction table was inserted");
    table
        .apply_operation_with_deferred_constraints(
            &OperationIr {
                method: operation.method,
                path: local_path,
                body: operation.body.clone(),
            },
            deferred_constraints.get(name).unwrap_or(&BTreeSet::new()),
        )
        .map_err(|error| CatalogTransactionError::Invalid(format!("table {name}: {error}")))?;

    let before = before_snapshot
        .map(|records| BTreeMap::from([(name.to_owned(), records)]))
        .unwrap_or_default();
    let cascaded = super::constraint_actions::apply_actions(&before, tables, deferred_constraints)
        .map_err(CatalogTransactionError::Catalog)?;
    let mut changed_tables = BTreeSet::from([name.to_owned()]);
    changed_tables.extend(cascaded.iter().cloned());
    super::constraints::validate_statement_constraints(
        tables,
        &changed_tables,
        deferred_constraints,
    )
    .map_err(CatalogTransactionError::Catalog)?;
    touched.insert(name.to_owned());
    touched.extend(cascaded);
    Ok(())
}

impl Catalog {
    pub(super) fn commit_loaded_tables_locked(
        &self,
        before: BTreeMap<String, DbfTable>,
        mut tables: BTreeMap<String, DbfTable>,
        mut touched: BTreeSet<String>,
        reuse_loaded_tables: bool,
        extra_changes: Vec<FileChange>,
        sidecar: Option<PreparedSidecarChange>,
        deferred_constraints: &BTreeMap<String, BTreeSet<String>>,
    ) -> Result<(u64, Option<Vec<u8>>), CatalogTransactionError> {
        if touched.is_empty() {
            let transaction_id = super::journal::read_transaction_id_locked(&self.root)
                .map(|transaction_id| transaction_id.unwrap_or(0))
                .map_err(CatalogTransactionError::Catalog);
            let transaction_id = transaction_id?;
            let mut changes = extra_changes;
            let sidecar_after = sidecar.map(|sidecar| {
                let after = (sidecar.after)(transaction_id);
                changes.push(FileChange {
                    target: sidecar.path,
                    before: sidecar.before,
                    after: Some(after.clone()),
                });
                after
            });
            if !changes.is_empty() {
                commit_files(&self.root, changes).map_err(CatalogTransactionError::Catalog)?;
            }
            return Ok((transaction_id, sidecar_after));
        }
        let before_snapshots =
            super::constraint_actions::snapshot_referenced_tables(&before, &tables)
                .map_err(CatalogTransactionError::Catalog)?;
        touched.extend(
            super::constraint_actions::apply_actions(
                &before_snapshots,
                &mut tables,
                deferred_constraints,
            )
            .map_err(CatalogTransactionError::Catalog)?,
        );
        if reuse_loaded_tables {
            super::constraints::validate_loaded_tables(&tables)
                .map_err(CatalogTransactionError::Catalog)?;
        } else {
            let validation_replacements = tables
                .iter()
                .filter(|(name, _)| touched.contains(*name))
                .map(|(name, table)| (name.clone(), table.clone()))
                .collect::<BTreeMap<_, _>>();
            self.validate_replacements(&validation_replacements)
                .map_err(CatalogTransactionError::Catalog)?;
        }
        let mut replacements = tables
            .into_iter()
            .filter(|(name, _)| touched.contains(name))
            .collect::<BTreeMap<_, _>>();

        let transaction_id =
            next_transaction_id_locked(&self.root).map_err(CatalogTransactionError::Catalog)?;
        let mut changes = Vec::new();
        let sidecar_after = sidecar.map(|sidecar| {
            let after = (sidecar.after)(transaction_id);
            changes.push(FileChange {
                target: sidecar.path,
                before: sidecar.before,
                after: Some(after.clone()),
            });
            after
        });
        for (name, table) in &mut replacements {
            let path = self
                .table_path(name)
                .expect("catalog transaction table path exists");
            let prepared = table.prepare_snapshot(path).map_err(|error| {
                CatalogTransactionError::Catalog(CatalogError::Table {
                    name: name.clone(),
                    source: error,
                })
            })?;
            changes.push(FileChange {
                target: path.to_path_buf(),
                before: Some(
                    fs::read(path).map_err(|error| {
                        CatalogTransactionError::Catalog(CatalogError::Io(error))
                    })?,
                ),
                after: Some(prepared.dbf),
            });
            if let Some((memo_path, bytes)) = prepared.memo {
                changes.push(FileChange {
                    target: memo_path.clone(),
                    before: read_optional(&memo_path).map_err(CatalogTransactionError::Catalog)?,
                    after: Some(bytes),
                });
            }
            if let Some(index_payload) = prepared.index_payload {
                let index_bytes = crate::index::decode_snapshot_payload(&index_payload)
                    .map_err(|error| {
                        CatalogTransactionError::Catalog(CatalogError::Invalid(format!(
                            "index snapshot error: {error}"
                        )))
                    })?
                    .ok_or_else(|| {
                        CatalogTransactionError::Catalog(CatalogError::Invalid(
                            "index snapshot payload is missing".into(),
                        ))
                    })?;
                let index_path = crate::index::sidecar_path(path);
                changes.push(FileChange {
                    target: index_path.clone(),
                    before: read_optional(&index_path).map_err(CatalogTransactionError::Catalog)?,
                    after: Some(index_bytes),
                });
            }
        }
        let mut snapshot_tables = BTreeMap::new();
        for entry in self.tables() {
            let table = match replacements.get(entry.name()) {
                Some(table) => table.clone(),
                None if reuse_loaded_tables => before
                    .get(entry.name())
                    .cloned()
                    .expect("catalog transaction table was loaded before commit"),
                None => self
                    .open_table_unlocked(entry.name())
                    .map_err(CatalogTransactionError::Catalog)?,
            };
            let (dbf, memo, schema) =
                table
                    .catalog_snapshot_parts(entry.path())
                    .map_err(|error| {
                        CatalogTransactionError::Catalog(CatalogError::Table {
                            name: entry.name().to_owned(),
                            source: error,
                        })
                    })?;
            snapshot_tables.insert(
                entry.name().to_owned(),
                TableSnapshot {
                    file_name: entry.file_name().to_owned(),
                    dbf,
                    memo,
                    schema,
                },
            );
        }
        let history_path = super::mvcc::path_for(&self.root);
        let history_before =
            read_optional(&history_path).map_err(CatalogTransactionError::Catalog)?;
        let history_after = super::mvcc::append_snapshot(
            history_before.as_deref(),
            Snapshot {
                transaction_id,
                tables: snapshot_tables,
            },
        )
        .map_err(CatalogTransactionError::Catalog)?;
        changes.push(FileChange {
            target: history_path,
            before: history_before,
            after: Some(history_after),
        });
        let cdc_path = super::cdc::path_for(&self.root);
        let cdc_event = super::cdc::event_for_tables(transaction_id, &before, &replacements)
            .map_err(CatalogTransactionError::Catalog)?;
        let cdc_after = super::cdc::staged_bytes(&cdc_path, &cdc_event)
            .map_err(CatalogTransactionError::Catalog)?;
        let cdc_before = read_optional(&cdc_path).map_err(CatalogTransactionError::Catalog)?;
        changes.push(FileChange {
            target: cdc_path,
            before: cdc_before,
            after: Some(cdc_after),
        });
        changes.extend(extra_changes);
        commit(&self.root, changes)
            .map(|transaction_id| (transaction_id, sidecar_after))
            .map_err(CatalogTransactionError::Catalog)
    }
}

fn matches_if_none_match(value: &str, current: &str) -> bool {
    let tags = value.split(',').map(str::trim).collect::<Vec<_>>();
    if tags.len() == 1 && tags[0] == "*" {
        return true;
    }
    tags.iter()
        .any(|tag| tag.strip_prefix("W/").unwrap_or(tag) == current)
}

fn matches_if_match(value: &str, current: &str) -> bool {
    let tags = value.split(',').map(str::trim).collect::<Vec<_>>();
    if tags.len() == 1 && tags[0] == "*" {
        return true;
    }
    if tags.iter().any(|tag| *tag == "*" || tag.starts_with("W/")) {
        return false;
    }
    tags.contains(&current)
}

fn transaction_operation_path(path: &str) -> Option<(&str, String)> {
    let segments = path.strip_prefix('/')?.split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        [table, "records"] if !table.is_empty() => Some((table, String::from("/records"))),
        [table, "records", record] if !table.is_empty() && !record.is_empty() => {
            Some((table, format!("/records/{record}")))
        }
        _ => None,
    }
}

pub(super) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, CatalogError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CatalogError::Io(error)),
    }
}

pub(super) fn sidecar_path(root: &Path, name: &str) -> Result<PathBuf, CatalogError> {
    let path = Path::new(name);
    if !matches!(path.components().next(), Some(Component::Normal(_)))
        || path.components().count() != 1
    {
        return Err(CatalogError::Invalid(
            "catalog sidecar name must be one direct child".into(),
        ));
    }
    Ok(root.join(path))
}

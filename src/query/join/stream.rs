use super::{JoinError, JoinRequest, JoinSource};
use crate::catalog::Catalog;
use crate::dbf::DbfTable;
use crate::index::IndexFile;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread;

pub struct BoundedJoinStream {
    receiver: Receiver<Result<Value, JoinError>>,
}

pub fn stream_query_bounded(
    catalog: &Catalog,
    request: &JoinRequest,
    capacity: usize,
) -> Result<BoundedJoinStream, JoinError> {
    super::validate(request)?;
    if capacity == 0 {
        return Err(JoinError::Invalid(
            "bounded join stream capacity must be positive".into(),
        ));
    }

    let mut table_names = vec![request.from.clone(), request.join.table.clone()];
    table_names.extend(request.joins.iter().map(|join| join.table.clone()));
    table_names.sort_unstable();
    table_names.dedup();

    let _lock = catalog.acquire_read_lock()?;
    let mut tables = BTreeMap::new();
    let mut indexes = BTreeMap::new();
    let index_fields = request
        .joins
        .iter()
        .chain(std::iter::once(&request.join))
        .filter(|spec| !matches!(&spec.kind, super::JoinType::Cross | super::JoinType::Full))
        .filter_map(|spec| {
            let prefix = format!("{}.", spec.table);
            let fields = spec
                .on
                .values()
                .map(|condition| {
                    condition
                        .equality
                        .field
                        .strip_prefix(&prefix)
                        .map(str::to_owned)
                })
                .collect::<Option<Vec<_>>>()?;
            Some((spec.table.clone(), fields))
        })
        .collect::<BTreeMap<_, _>>();
    for name in table_names {
        let table = catalog.open_table_unlocked(&name)?;
        if !catalog.is_historical()
            && let Some(fields) = index_fields.get(&name)
            && let Some(index) =
                super::super::join_index::load_fields(catalog, &name, &table, fields)
        {
            indexes.insert(name.clone(), index);
        }
        tables.insert(name, table);
    }
    drop(_lock);

    let source = SnapshotJoinSource {
        tables: RefCell::new(tables),
        indexes: RefCell::new(indexes),
        historical: catalog.is_historical(),
    };
    let request = request.clone();
    let (sender, receiver) = sync_channel(capacity);
    thread::Builder::new()
        .name("txbase-join-stream".into())
        .spawn(move || {
            let stream = match super::super::join_pipeline::stream(&source, &request) {
                Ok(stream) => stream,
                Err(error) => {
                    let _ = sender.send(Err(error));
                    return;
                }
            };
            for item in stream {
                if sender.send(item).is_err() {
                    break;
                }
            }
        })
        .map_err(|error| JoinError::Invalid(format!("failed to spawn join stream: {error}")))?;

    Ok(BoundedJoinStream { receiver })
}

struct SnapshotJoinSource {
    tables: RefCell<BTreeMap<String, DbfTable>>,
    indexes: RefCell<BTreeMap<String, IndexFile>>,
    historical: bool,
}

impl JoinSource for SnapshotJoinSource {
    fn is_historical(&self) -> bool {
        self.historical
    }

    fn open_table(&self, name: &str) -> Result<DbfTable, JoinError> {
        self.tables
            .borrow_mut()
            .remove(name)
            .ok_or_else(|| JoinError::Invalid(format!("join snapshot is missing table: {name}")))
    }

    fn catalog(&self) -> Option<&Catalog> {
        None
    }

    fn load_index_for_fields(
        &self,
        table_name: &str,
        _table: &DbfTable,
        fields: &[String],
    ) -> Option<IndexFile> {
        let mut indexes = self.indexes.borrow_mut();
        if !indexes
            .get(table_name)?
            .has_exact_fields(&fields.iter().map(String::as_str).collect::<Vec<_>>())
        {
            return None;
        }
        indexes.remove(table_name)
    }
}

impl Iterator for BoundedJoinStream {
    type Item = Result<Value, JoinError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

use super::{JoinError, JoinRequest, JoinSource};
use crate::catalog::Catalog;
use crate::dbf::DbfTable;
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
    for name in table_names {
        tables.insert(name.clone(), catalog.open_table_unlocked(&name)?);
    }
    drop(_lock);

    let source = SnapshotJoinSource {
        tables: RefCell::new(tables),
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
}

impl Iterator for BoundedJoinStream {
    type Item = Result<Value, JoinError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

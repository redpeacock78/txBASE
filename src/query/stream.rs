use super::{QueryError, QueryRequest, matches_filter};
use crate::dbf::DbfTable;
use crate::query_path::{project, project_values};
use crate::xbf::{XbfField, XbfTable, XbfValue};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread;

pub struct QueryStream<'table, 'request> {
    records: std::slice::Iter<'table, crate::dbf::DbfRecord>,
    request: &'request QueryRequest,
    skip: u64,
    skipped: u64,
    limit: Option<u64>,
    yielded: u64,
    done: bool,
}

pub struct QuerySnapshotStream<'request> {
    table: DbfTable,
    request: &'request QueryRequest,
    state: SnapshotStreamState,
}

pub(crate) struct OwnedQuerySnapshotStream {
    table: SnapshotTable,
    request: QueryRequest,
    state: SnapshotStreamState,
}

enum SnapshotTable {
    Dbf(Box<DbfTable>),
    Xbf(XbfTable),
}

#[derive(Clone, Copy)]
enum SnapshotQuerySource<'table> {
    Dbf(&'table DbfTable),
    Xbf(&'table XbfTable),
}

struct SnapshotQueryRow<'row> {
    deleted: bool,
    values: SnapshotQueryValues<'row>,
}

enum SnapshotQueryValues<'row> {
    Dbf(&'row Map<String, Value>),
    Xbf(&'row [XbfField], &'row [XbfValue]),
}

impl SnapshotQuerySource<'_> {
    fn row(&self, position: usize) -> Option<SnapshotQueryRow<'_>> {
        match self {
            Self::Dbf(table) => table
                .records()
                .get(position)
                .map(|record| SnapshotQueryRow {
                    deleted: record.deleted,
                    values: SnapshotQueryValues::Dbf(&record.values),
                }),
            Self::Xbf(table) => table.records.get(position).map(|record| SnapshotQueryRow {
                deleted: record.deleted,
                values: SnapshotQueryValues::Xbf(&table.fields, &record.values),
            }),
        }
    }
}

impl<'row> SnapshotQueryValues<'row> {
    fn into_map(self) -> Result<Cow<'row, Map<String, Value>>, QueryError> {
        match self {
            Self::Dbf(values) => Ok(Cow::Borrowed(values)),
            Self::Xbf(fields, values) => crate::xbf::record_values(fields, values)
                .map(Cow::Owned)
                .map_err(|error| QueryError::Invalid(format!("cannot query XBF row: {error}"))),
        }
    }
}

struct SnapshotStreamState {
    position: usize,
    skip: u64,
    skipped: u64,
    limit: Option<u64>,
    yielded: u64,
    done: bool,
}

impl SnapshotStreamState {
    fn new(request: &QueryRequest) -> Self {
        Self {
            position: 0,
            skip: request.skip.unwrap_or_default(),
            skipped: 0,
            limit: request.limit,
            yielded: 0,
            done: false,
        }
    }

    fn next(
        &mut self,
        source: SnapshotQuerySource<'_>,
        request: &QueryRequest,
    ) -> Option<Result<Value, QueryError>> {
        if self.done || self.limit.is_some_and(|limit| self.yielded >= limit) {
            self.done = true;
            return None;
        }
        loop {
            let Some(record) = source.row(self.position) else {
                self.done = true;
                return None;
            };
            self.position += 1;
            if record.deleted {
                continue;
            }
            let values = match record.values.into_map() {
                Ok(values) => values,
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            };
            match matches_filter(values.as_ref(), &request.filter) {
                Ok(false) => continue,
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
                Ok(true) => {}
            }
            if self.skipped < self.skip {
                self.skipped += 1;
                continue;
            }
            self.yielded += 1;
            return Some(Ok(project_values(values.as_ref(), &request.projection)));
        }
    }
}

pub struct BoundedQueryStream {
    receiver: Receiver<Result<Value, QueryError>>,
}

pub fn stream_query<'table, 'request>(
    table: &'table DbfTable,
    request: &'request QueryRequest,
) -> Result<QueryStream<'table, 'request>, QueryError> {
    validate_stream_request(request)?;
    Ok(QueryStream {
        records: table.records().iter(),
        request,
        skip: request.skip.unwrap_or_default(),
        skipped: 0,
        limit: request.limit,
        yielded: 0,
        done: false,
    })
}

pub fn stream_query_snapshot<'request>(
    table: &DbfTable,
    request: &'request QueryRequest,
) -> Result<QuerySnapshotStream<'request>, QueryError> {
    validate_stream_request(request)?;
    Ok(QuerySnapshotStream {
        table: table.clone(),
        request,
        state: SnapshotStreamState::new(request),
    })
}

pub(crate) fn stream_query_snapshot_owned(
    table: &DbfTable,
    request: &QueryRequest,
) -> Result<OwnedQuerySnapshotStream, QueryError> {
    validate_stream_request(request)?;
    Ok(OwnedQuerySnapshotStream {
        table: SnapshotTable::Dbf(Box::new(table.clone())),
        request: request.clone(),
        state: SnapshotStreamState::new(request),
    })
}

pub(crate) fn stream_xbf_snapshot_owned(
    table: XbfTable,
    request: QueryRequest,
) -> Result<OwnedQuerySnapshotStream, QueryError> {
    validate_stream_request(&request)?;
    Ok(OwnedQuerySnapshotStream {
        table: SnapshotTable::Xbf(table),
        state: SnapshotStreamState::new(&request),
        request,
    })
}

pub fn stream_query_bounded(
    table: &DbfTable,
    request: &QueryRequest,
    capacity: usize,
) -> Result<BoundedQueryStream, QueryError> {
    validate_stream_request(request)?;
    if capacity == 0 {
        return Err(QueryError::Invalid(
            "bounded stream capacity must be positive".into(),
        ));
    }
    let table = table.clone();
    let request = request.clone();
    let (sender, receiver) = sync_channel(capacity);
    thread::spawn(move || {
        let stream = match stream_query_snapshot(&table, &request) {
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
    });
    Ok(BoundedQueryStream { receiver })
}

pub(crate) fn validate_stream_request(request: &QueryRequest) -> Result<(), QueryError> {
    super::validation::validate(request)?;
    if !request.sort.is_empty()
        || request.aggregate.is_some()
        || request.page_size.is_some()
        || request.cursor.is_some()
    {
        return Err(QueryError::Invalid(
            "streaming query supports filter, projection, skip, and limit only".into(),
        ));
    }
    Ok(())
}

impl<'table, 'request> Iterator for QueryStream<'table, 'request> {
    type Item = Result<Value, QueryError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.limit.is_some_and(|limit| self.yielded >= limit) {
            self.done = true;
            return None;
        }
        loop {
            let record = self.records.next()?;
            if record.deleted {
                continue;
            }
            match matches_filter(&record.values, &self.request.filter) {
                Ok(false) => continue,
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
                Ok(true) => {}
            }
            if self.skipped < self.skip {
                self.skipped += 1;
                continue;
            }
            self.yielded += 1;
            return Some(Ok(project(record, &self.request.projection)));
        }
    }
}

impl<'request> Iterator for QuerySnapshotStream<'request> {
    type Item = Result<Value, QueryError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.state
            .next(SnapshotQuerySource::Dbf(&self.table), self.request)
    }
}

impl Iterator for OwnedQuerySnapshotStream {
    type Item = Result<Value, QueryError>;

    fn next(&mut self) -> Option<Self::Item> {
        let source = match &self.table {
            SnapshotTable::Dbf(table) => SnapshotQuerySource::Dbf(table),
            SnapshotTable::Xbf(table) => SnapshotQuerySource::Xbf(table),
        };
        self.state.next(source, &self.request)
    }
}

impl Iterator for BoundedQueryStream {
    type Item = Result<Value, QueryError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

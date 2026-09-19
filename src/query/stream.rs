use super::{QueryError, QueryRequest, matches_filter};
use crate::dbf::DbfTable;
use crate::query_path::project;
use serde_json::Value;

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
    position: usize,
    skip: u64,
    skipped: u64,
    limit: Option<u64>,
    yielded: u64,
    done: bool,
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
        position: 0,
        skip: request.skip.unwrap_or_default(),
        skipped: 0,
        limit: request.limit,
        yielded: 0,
        done: false,
    })
}

fn validate_stream_request(request: &QueryRequest) -> Result<(), QueryError> {
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
        if self.done || self.limit.is_some_and(|limit| self.yielded >= limit) {
            self.done = true;
            return None;
        }
        loop {
            let Some(record) = self.table.records().get(self.position) else {
                self.done = true;
                return None;
            };
            self.position += 1;
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

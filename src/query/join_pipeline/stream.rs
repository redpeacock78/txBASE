use super::super::join::{JoinError, JoinRequest, JoinSource, JoinType, MAX_JOIN_ROWS};
use super::cost::load_rows;
use super::stage_fields;
use crate::query::matches_filter;
use crate::query_path::project_values;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

mod rows;

use rows::JoinRows;

type Row = Map<String, Value>;

pub(in crate::query) fn execute(
    source: &dyn JoinSource,
    request: &JoinRequest,
) -> Result<JoinRowStream, JoinError> {
    super::validate(request)?;
    let specs = std::iter::once(&request.join)
        .chain(request.joins.iter())
        .collect::<Vec<_>>();
    let (last, preceding) = specs
        .split_last()
        .expect("every join request has a first stage");
    let initial = load_rows(source, &request.from)?;
    let mut left = initial.values;
    let mut page_reads = initial.page_reads;
    for spec in preceding {
        (left, page_reads) = super::apply_stage(source, left, page_reads, spec)?;
    }

    let right = load_rows(source, &last.table)?;
    let (local_fields, foreign_fields) = stage_fields(last);
    JoinRowStream::new(
        left,
        right.values,
        last.kind.clone(),
        local_fields,
        foreign_fields,
        request.filter.clone(),
        request.projection.clone(),
        !request.joins.is_empty(),
    )
}

pub(in crate::query) struct JoinRowStream {
    rows: JoinRows,
    filter: Map<String, Value>,
    projection: BTreeMap<String, i8>,
    limit_stage_rows: bool,
    stage_rows: usize,
    output_rows: usize,
    done: bool,
}

impl JoinRowStream {
    fn new(
        left: Vec<Row>,
        right: Vec<Row>,
        kind: JoinType,
        local_fields: Vec<String>,
        foreign_fields: Vec<String>,
        filter: Map<String, Value>,
        projection: BTreeMap<String, i8>,
        limit_stage_rows: bool,
    ) -> Result<Self, JoinError> {
        Ok(Self {
            rows: JoinRows::new(left, right, kind, local_fields, foreign_fields)?,
            filter,
            projection,
            limit_stage_rows,
            stage_rows: 0,
            output_rows: 0,
            done: false,
        })
    }
}

impl Iterator for JoinRowStream {
    type Item = Result<Value, JoinError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            let row = match self.rows.next_row() {
                Ok(Some(row)) => row,
                Ok(None) => {
                    self.done = true;
                    return None;
                }
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            };

            if self.limit_stage_rows {
                if self.stage_rows >= MAX_JOIN_ROWS {
                    self.done = true;
                    return Some(Err(row_limit_error()));
                }
                self.stage_rows += 1;
            }
            match matches_filter(&row, &self.filter) {
                Ok(false) => continue,
                Err(error) => {
                    self.done = true;
                    return Some(Err(error.into()));
                }
                Ok(true) => {}
            }
            if !self.limit_stage_rows {
                if self.output_rows >= MAX_JOIN_ROWS {
                    self.done = true;
                    return Some(Err(row_limit_error()));
                }
                self.output_rows += 1;
            }
            return Some(Ok(project_values(&row, &self.projection)));
        }
    }
}

fn row_limit_error() -> JoinError {
    JoinError::Invalid(format!("join result exceeds {MAX_JOIN_ROWS} rows"))
}

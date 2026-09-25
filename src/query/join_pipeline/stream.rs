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
    let initial = load_rows(source, &request.from, None, None)?;
    let mut left = initial.values;
    let mut page_reads = initial.page_reads;
    for spec in preceding {
        (left, page_reads) = super::apply_stage(source, left, page_reads, spec)?;
    }

    let (_, foreign_fields) = stage_fields(last);
    let index_fields = (!matches!(&last.kind, JoinType::Cross | JoinType::Full))
        .then_some(foreign_fields.as_slice());
    let right = load_rows(source, &last.table, index_fields, Some(left.len()))?;
    let cost_input = super::stage_cost_input(&left, page_reads, &right, last)?;
    let plan = super::stages::plan(
        left.len(),
        right.values.len(),
        last,
        right.index,
        cost_input,
    );
    Ok(JoinRowStream {
        rows: JoinRows::planned(left, right.values, right.numbers, plan)?,
        filter: request.filter.clone(),
        projection: request.projection.clone(),
        limit_stage_rows: !request.joins.is_empty(),
        stage_rows: 0,
        output_rows: 0,
        done: false,
    })
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

#[cfg(test)]
mod tests {
    use super::{JoinRows, execute};
    use crate::catalog::Catalog;
    use crate::query::join::{self, JoinType, parse};
    use crate::query::join_pipeline::stages::JoinStagePlan;
    use crate::query::join_pipeline::{cost::load_rows, stage_cost_input, stages};
    use crate::query::join_strategy::JoinStrategy;
    use serde_json::Value;
    use std::fs;

    #[test]
    fn planner_selects_fresh_stream_index() {
        let root = crate::query::join_tests::catalog_with_many_indexed_posts();
        let catalog = Catalog::from_path(&root).unwrap();
        let request = parse(
            br#"{
                "from":"users",
                "join":{"type":"inner","table":"posts","on":{"users.ID":{"$eq":{"$field":"posts.ID"}}}},
                "projection":{"users.ID":1,"posts.NAME":1}
            }"#,
        )
        .unwrap();

        let initial = load_rows(&catalog, "users", None, None).unwrap();
        let (_, foreign_fields) = super::super::stage_fields(&request.join);
        let right = load_rows(
            &catalog,
            "posts",
            Some(&foreign_fields),
            Some(initial.values.len()),
        )
        .unwrap();
        assert!(right.index.is_some());
        let cost =
            stage_cost_input(&initial.values, initial.page_reads, &right, &request.join).unwrap();
        let plan = stages::plan(
            initial.values.len(),
            right.values.len(),
            &request.join,
            right.index,
            cost,
        );
        assert_eq!(plan.strategy, JoinStrategy::IndexNestedLoop);

        let planned = execute(&catalog, &request).unwrap();
        assert!(matches!(&planned.rows, JoinRows::Index(_)));
        let expected = join::execute(&catalog, &request).unwrap();
        let bounded = join::stream_query_bounded(&catalog, &request, 2).unwrap();
        let streamed = bounded.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(streamed, expected);
        assert_eq!(planned.collect::<Result<Vec<_>, _>>().unwrap(), expected);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn streams_index_join_types() {
        let root = crate::query::join_tests::catalog_with_many_indexed_posts();
        let catalog = Catalog::from_path(&root).unwrap();
        let mut request = parse(
            br#"{
                "from":"users",
                "join":{"type":"inner","table":"posts","on":{"users.ID":{"$eq":{"$field":"posts.ID"}}}}
            }"#,
        )
        .unwrap();

        for kind in [
            JoinType::Inner,
            JoinType::Left,
            JoinType::Semi,
            JoinType::Anti,
        ] {
            request.join.kind = kind.clone();
            let initial = load_rows(&catalog, "users", None, None).unwrap();
            let (local_fields, foreign_fields) = super::super::stage_fields(&request.join);
            let right = load_rows(
                &catalog,
                "posts",
                Some(&foreign_fields),
                Some(initial.values.len()),
            )
            .unwrap();
            let plan = JoinStagePlan {
                strategy: JoinStrategy::IndexNestedLoop,
                fallback_strategy: JoinStrategy::Hash,
                kind,
                local_fields,
                foreign_fields,
                index_fields: Some(vec!["ID".into()]),
                right_index: right.index.clone(),
                right_order: None,
            };
            let mut rows =
                JoinRows::planned(initial.values, right.values, right.numbers, plan).unwrap();
            assert!(matches!(&rows, JoinRows::Index(_)));
            let actual = std::iter::from_fn(|| rows.next_row().transpose())
                .map(|row| row.map(Value::Object))
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(actual, join::execute(&catalog, &request).unwrap());
        }
        fs::remove_dir_all(root).unwrap();
    }
}

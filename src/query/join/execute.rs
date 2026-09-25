use super::super::join_pipeline;
use super::super::matches_filter;
use super::{JoinError, JoinRequest, JoinType, MAX_JOIN_ROWS};
use crate::catalog::{Catalog, CatalogReadTransaction};
use crate::dbf::{DbfRecord, DbfTable};
use crate::index::IndexFile;
use crate::query_path::{field_value, project_values};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

mod direct;

pub(crate) trait JoinSource {
    fn is_historical(&self) -> bool;
    fn open_table(&self, name: &str) -> Result<crate::dbf::DbfTable, JoinError>;
    fn open_tables(
        &self,
        left: &str,
        right: &str,
    ) -> Result<(crate::dbf::DbfTable, crate::dbf::DbfTable), JoinError> {
        Ok((self.open_table(left)?, self.open_table(right)?))
    }
    fn catalog(&self) -> Option<&Catalog>;

    fn load_index_for_fields(
        &self,
        table_name: &str,
        table: &DbfTable,
        fields: &[String],
    ) -> Option<IndexFile> {
        if self.is_historical() {
            return None;
        }
        let catalog = self.catalog()?;
        super::super::join_index::load_fields(catalog, table_name, table, fields)
    }
}

impl JoinSource for Catalog {
    fn is_historical(&self) -> bool {
        self.is_historical()
    }

    fn open_table(&self, name: &str) -> Result<crate::dbf::DbfTable, JoinError> {
        self.open_table_unlocked(name).map_err(Into::into)
    }

    fn catalog(&self) -> Option<&Catalog> {
        Some(self)
    }
}

impl JoinSource for CatalogReadTransaction {
    fn is_historical(&self) -> bool {
        true
    }

    fn open_table(&self, name: &str) -> Result<crate::dbf::DbfTable, JoinError> {
        self.open_table(name).map_err(Into::into)
    }

    fn catalog(&self) -> Option<&Catalog> {
        None
    }
}

pub fn execute(catalog: &Catalog, request: &JoinRequest) -> Result<Vec<Value>, JoinError> {
    super::validate(request)?;
    let _lock = catalog.acquire_read_lock()?;
    execute_with_source(catalog, request)
}

pub(crate) fn execute_read_transaction(
    snapshot: &CatalogReadTransaction,
    request: &JoinRequest,
) -> Result<Vec<Value>, JoinError> {
    super::validate(request)?;
    execute_with_source(snapshot, request)
}

fn execute_with_source<S: JoinSource>(
    source: &S,
    request: &JoinRequest,
) -> Result<Vec<Value>, JoinError> {
    if !request.joins.is_empty() {
        return join_pipeline::execute(source, request);
    }
    direct::execute(source, request)
}

pub(crate) fn encoded_key(
    values: &Map<String, Value>,
    fields: &[String],
) -> Result<Option<String>, JoinError> {
    let mut key = Vec::with_capacity(fields.len());
    for field in fields {
        let Some(value) = field_value(values, field) else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        key.push(value);
    }
    serde_json::to_string(&key)
        .map(Some)
        .map_err(|error| JoinError::Invalid(format!("join key encoding failed: {error}")))
}

fn logical_page_reads(table: &DbfTable) -> usize {
    table
        .byte_len()
        .div_ceil(crate::index::COST_PAGE_SIZE)
        .max(1)
}

fn estimate_join_rows(
    left_records: &[&DbfRecord],
    right_records: &[&DbfRecord],
    local_fields: &[String],
    foreign_fields: &[String],
    join_type: &JoinType,
) -> Result<usize, JoinError> {
    if matches!(join_type, JoinType::Cross) {
        return Err(JoinError::Invalid(
            "cross joins do not use equality cardinality estimation".into(),
        ));
    }

    let mut left_counts = BTreeMap::<String, usize>::new();
    for record in left_records {
        if let Some(key) = encoded_key(&record.values, local_fields)? {
            let count = left_counts.entry(key).or_default();
            *count = count.saturating_add(1);
        }
    }
    let mut right_counts = BTreeMap::<String, usize>::new();
    for record in right_records {
        if let Some(key) = encoded_key(&record.values, foreign_fields)? {
            let count = right_counts.entry(key).or_default();
            *count = count.saturating_add(1);
        }
    }

    let mut matched_pairs = 0usize;
    let mut left_matched = 0usize;
    for (key, left_count) in &left_counts {
        if let Some(right_count) = right_counts.get(key) {
            matched_pairs = matched_pairs.saturating_add(left_count.saturating_mul(*right_count));
            left_matched = left_matched.saturating_add(*left_count);
        }
    }
    let mut right_matched = 0usize;
    for (key, right_count) in &right_counts {
        if left_counts.contains_key(key) {
            right_matched = right_matched.saturating_add(*right_count);
        }
    }
    let left_unmatched = left_records.len().saturating_sub(left_matched);
    let right_unmatched = right_records.len().saturating_sub(right_matched);

    Ok(match join_type {
        JoinType::Inner => matched_pairs,
        JoinType::Left => matched_pairs.saturating_add(left_unmatched),
        JoinType::Right => matched_pairs.saturating_add(right_unmatched),
        JoinType::Full => matched_pairs
            .saturating_add(left_unmatched)
            .saturating_add(right_unmatched),
        JoinType::Semi => left_matched,
        JoinType::Anti => left_unmatched,
        JoinType::Cross => unreachable!("cross join handled before cardinality estimation"),
    })
}

pub(crate) fn emit(
    output: &mut Vec<Value>,
    request: &JoinRequest,
    left: Option<&DbfRecord>,
    right: Option<&DbfRecord>,
) -> Result<(), JoinError> {
    let mut values = Map::new();
    if let Some(left) = left {
        add_qualified_values(&mut values, &request.from, &left.values);
    }
    if let Some(right) = right {
        add_qualified_values(&mut values, &request.join.table, &right.values);
    }
    if !matches_filter(&values, &request.filter)? {
        return Ok(());
    }
    if output.len() >= MAX_JOIN_ROWS {
        return Err(JoinError::Invalid(format!(
            "join result exceeds {MAX_JOIN_ROWS} rows"
        )));
    }
    output.push(project_values(&values, &request.projection));
    Ok(())
}

fn add_qualified_values(output: &mut Map<String, Value>, table: &str, values: &Map<String, Value>) {
    for (field, value) in values {
        output.insert(format!("{table}.{field}"), value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::{JoinType, estimate_join_rows};
    use crate::dbf::DbfRecord;
    use serde_json::{Map, json};

    fn record(number: usize, key: Option<i64>) -> DbfRecord {
        let mut values = Map::new();
        if let Some(key) = key {
            values.insert("ID".into(), json!(key));
        }
        DbfRecord {
            number,
            deleted: false,
            values,
        }
    }

    #[test]
    fn estimates_each_equality_join_shape() {
        let left = [
            record(1, Some(1)),
            record(2, Some(1)),
            record(3, Some(2)),
            record(4, None),
        ];
        let right = [record(1, Some(1)), record(2, Some(3))];
        let left = left.iter().collect::<Vec<_>>();
        let right = right.iter().collect::<Vec<_>>();
        let fields = vec!["ID".to_owned()];

        for (join_type, expected) in [
            (JoinType::Inner, 2),
            (JoinType::Left, 4),
            (JoinType::Right, 3),
            (JoinType::Full, 5),
            (JoinType::Semi, 2),
            (JoinType::Anti, 2),
        ] {
            assert_eq!(
                estimate_join_rows(&left, &right, &fields, &fields, &join_type).unwrap(),
                expected,
                "unexpected estimate for {join_type:?}"
            );
        }
    }

    #[test]
    fn rejects_cross_join_cardinality_estimation() {
        let records = [record(1, Some(1))];
        let records = records.iter().collect::<Vec<_>>();
        let fields = vec!["ID".to_owned()];

        assert!(
            estimate_join_rows(&records, &records, &fields, &fields, &JoinType::Cross).is_err()
        );
    }
}

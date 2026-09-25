use super::super::join::{JoinError, JoinSource, JoinType};
use super::{encoded_key, qualified_values};
use crate::index::IndexFile;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(super) struct LoadedRows {
    pub(super) values: Vec<Map<String, Value>>,
    pub(super) numbers: Vec<usize>,
    pub(super) page_reads: usize,
    pub(super) index: Option<IndexFile>,
}

pub(super) fn load_rows(
    source: &dyn JoinSource,
    table_name: &str,
    index_fields: Option<&[String]>,
    outer_rows: Option<usize>,
) -> Result<LoadedRows, JoinError> {
    let table = source.open_table(table_name)?;
    let active_rows = table.active_records().count();
    let index = index_fields
        .filter(|_| {
            outer_rows.is_some_and(|count| {
                count.saturating_mul(active_rows)
                    > super::super::join_strategy::NESTED_LOOP_PAIR_LIMIT
            })
        })
        .and_then(|fields| source.load_index_for_fields(table_name, &table, fields));
    let page_reads = table
        .byte_len()
        .div_ceil(crate::index::COST_PAGE_SIZE)
        .max(1);
    let mut values = Vec::new();
    let mut numbers = Vec::new();
    for record in table.active_records() {
        numbers.push(record.number);
        values.push(qualified_values(table_name, &record.values));
    }
    Ok(LoadedRows {
        values,
        numbers,
        page_reads,
        index,
    })
}

pub(super) fn output_columns(
    left: &[Map<String, Value>],
    right: &[Map<String, Value>],
    join_type: &JoinType,
) -> usize {
    let left_columns = left.iter().map(Map::len).max().unwrap_or_default();
    if matches!(join_type, JoinType::Semi | JoinType::Anti) {
        return left_columns.max(1);
    }
    left_columns
        .saturating_add(right.iter().map(Map::len).max().unwrap_or_default())
        .max(1)
}

pub(super) fn materialized_page_reads(rows: &[Map<String, Value>]) -> usize {
    let bytes = rows.iter().fold(0usize, |total, row| {
        total.saturating_add(
            serde_json::to_vec(row)
                .expect("join rows must be JSON serializable")
                .len(),
        )
    });
    bytes.div_ceil(crate::index::COST_PAGE_SIZE).max(1)
}

pub(super) fn estimate_join_rows(
    left_rows: &[Map<String, Value>],
    right_rows: &[Map<String, Value>],
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
    for row in left_rows {
        if let Some(key) = encoded_key(row, local_fields)? {
            let count = left_counts.entry(key).or_default();
            *count = count.saturating_add(1);
        }
    }
    let mut right_counts = BTreeMap::<String, usize>::new();
    for row in right_rows {
        if let Some(key) = encoded_key(row, foreign_fields)? {
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
    let left_unmatched = left_rows.len().saturating_sub(left_matched);
    let right_unmatched = right_rows.len().saturating_sub(right_matched);

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

#[cfg(test)]
mod tests {
    use super::super::super::join::JoinType;
    use super::{estimate_join_rows, materialized_page_reads, output_columns};
    use crate::index::COST_PAGE_SIZE;
    use serde_json::{Map, json};

    fn row(table: &str, key: Option<i64>) -> Map<String, serde_json::Value> {
        let mut values = Map::new();
        if let Some(key) = key {
            values.insert(format!("{table}.ID"), json!(key));
        }
        values
    }

    #[test]
    fn estimates_chained_stage_cardinality_from_qualified_keys() {
        let left = [
            row("left", Some(1)),
            row("left", Some(1)),
            row("left", Some(2)),
            row("left", None),
        ];
        let mut null = Map::new();
        null.insert("right.ID".to_owned(), serde_json::Value::Null);
        let right = [row("right", Some(1)), row("right", Some(3)), null];
        let local_fields = vec!["left.ID".to_owned()];
        let foreign_fields = vec!["right.ID".to_owned()];

        for (join_type, expected) in [
            (JoinType::Inner, 2),
            (JoinType::Left, 4),
            (JoinType::Right, 4),
            (JoinType::Full, 6),
            (JoinType::Semi, 2),
            (JoinType::Anti, 2),
        ] {
            assert_eq!(
                estimate_join_rows(&left, &right, &local_fields, &foreign_fields, &join_type,)
                    .unwrap(),
                expected
            );
        }

        assert_eq!(output_columns(&left, &right, &JoinType::Inner), 2);
        assert_eq!(output_columns(&left, &right, &JoinType::Semi), 1);
    }

    #[test]
    fn keeps_cost_boundaries_nonzero_for_empty_inputs() {
        let empty: [Map<String, serde_json::Value>; 0] = [];

        assert_eq!(output_columns(&empty, &empty, &JoinType::Inner), 1);
        assert_eq!(output_columns(&empty, &empty, &JoinType::Semi), 1);
        assert_eq!(materialized_page_reads(&empty), 1);
    }

    #[test]
    fn rounds_materialized_rows_up_to_the_next_page() {
        let large = json!({"payload": "x".repeat(COST_PAGE_SIZE)})
            .as_object()
            .expect("object fixture")
            .clone();

        assert_eq!(materialized_page_reads(std::slice::from_ref(&large)), 2);
    }

    #[test]
    fn rejects_cross_stage_cardinality_estimation() {
        let rows = [row("left", Some(1))];
        let fields = vec!["left.ID".to_owned()];

        assert!(estimate_join_rows(&rows, &rows, &fields, &fields, &JoinType::Cross).is_err());
    }
}

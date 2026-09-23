use super::super::join::{JoinError, JoinSource, JoinType};
use super::{encoded_key, qualified_values};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(super) struct LoadedRows {
    pub(super) values: Vec<Map<String, Value>>,
    pub(super) numbers: Vec<usize>,
    pub(super) page_reads: usize,
}

pub(super) fn load_rows(
    source: &dyn JoinSource,
    table_name: &str,
) -> Result<LoadedRows, JoinError> {
    let table = source.open_table(table_name)?;
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
    use super::{estimate_join_rows, output_columns};
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
        let right = [row("right", Some(1)), row("right", Some(3))];
        let local_fields = vec!["left.ID".to_owned()];
        let foreign_fields = vec!["right.ID".to_owned()];

        assert_eq!(
            estimate_join_rows(
                &left,
                &right,
                &local_fields,
                &foreign_fields,
                &JoinType::Left,
            )
            .unwrap(),
            4
        );
        assert_eq!(output_columns(&left, &right, &JoinType::Inner), 2);
        assert_eq!(output_columns(&left, &right, &JoinType::Semi), 1);
    }

    #[test]
    fn rejects_cross_stage_cardinality_estimation() {
        let rows = [row("left", Some(1))];
        let fields = vec!["left.ID".to_owned()];

        assert!(estimate_join_rows(&rows, &rows, &fields, &fields, &JoinType::Cross).is_err());
    }
}

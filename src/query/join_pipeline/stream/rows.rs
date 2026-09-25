use super::super::super::join::JoinError;
#[cfg(test)]
use super::super::super::join::JoinType;
use super::Row;
use serde_json::Map;

mod hash;
mod index;
mod merge;
mod nested;

use crate::query::join_pipeline::stages::JoinStagePlan;
use crate::query::join_strategy::JoinStrategy;
use hash::HashJoinRows;
use index::IndexJoinRows;
use merge::MergeJoinRows;
use nested::NestedJoinRows;

pub(super) enum JoinRows {
    Hash(HashJoinRows),
    Index(IndexJoinRows),
    Merge(MergeJoinRows),
    Nested(NestedJoinRows),
}

impl JoinRows {
    #[cfg(test)]
    pub(super) fn new(
        left: Vec<Row>,
        right: Vec<Row>,
        kind: JoinType,
        local_fields: Vec<String>,
        foreign_fields: Vec<String>,
    ) -> Result<Self, JoinError> {
        HashJoinRows::new(left, right, kind, local_fields, foreign_fields).map(Self::Hash)
    }

    pub(super) fn planned(
        left: Vec<Row>,
        right: Vec<Row>,
        right_numbers: Vec<usize>,
        plan: JoinStagePlan,
    ) -> Result<Self, JoinError> {
        let JoinStagePlan {
            strategy,
            fallback_strategy,
            kind,
            local_fields,
            foreign_fields,
            index_fields,
            right_index,
            right_order,
        } = plan;

        if matches!(strategy, JoinStrategy::Merge) {
            if let Some(right_order) = right_order {
                let ordered = super::super::stages::OrderedMergeRows::new(
                    &left,
                    &right,
                    &right_numbers,
                    &right_order,
                    kind.clone(),
                    &local_fields,
                    &foreign_fields,
                )?;
                return Ok(Self::Merge(MergeJoinRows::with_rows(left, right, ordered)));
            }
        }
        if matches!(strategy, JoinStrategy::IndexNestedLoop) {
            if let (Some(index), Some(index_fields)) = (right_index, index_fields) {
                if IndexJoinRows::can_use(&left, &kind, &local_fields, &index, &index_fields) {
                    return IndexJoinRows::new(
                        left,
                        right,
                        right_numbers,
                        kind,
                        local_fields,
                        index_fields,
                        index,
                    )
                    .map(Self::Index);
                }
            }
        }

        if matches!(strategy, JoinStrategy::NestedLoop)
            || matches!(fallback_strategy, JoinStrategy::NestedLoop)
        {
            return NestedJoinRows::new(left, right, kind, local_fields, foreign_fields)
                .map(Self::Nested);
        }
        HashJoinRows::new(left, right, kind, local_fields, foreign_fields).map(Self::Hash)
    }

    pub(super) fn next_row(&mut self) -> Result<Option<Row>, JoinError> {
        match self {
            Self::Hash(rows) => rows.next_row(),
            Self::Index(rows) => rows.next_row(),
            Self::Merge(rows) => rows.next_row(),
            Self::Nested(rows) => rows.next_row(),
        }
    }
}

pub(super) fn combine(left: Option<&Row>, right: Option<&Row>) -> Row {
    let mut row = Map::new();
    if let Some(left) = left {
        row.extend(left.clone());
    }
    if let Some(right) = right {
        row.extend(right.clone());
    }
    row
}

#[cfg(test)]
mod tests {
    use super::{JoinRows, JoinStagePlan};
    use crate::query::join::JoinType;
    use crate::query::join_strategy::JoinStrategy;
    use serde_json::{Map, Value, json};

    type TestRows = Vec<Map<String, Value>>;

    fn row(table: &str, key: Option<i64>, label: &str) -> Map<String, Value> {
        let mut row = Map::from_iter([(format!("{table}.LABEL"), json!(label))]);
        if let Some(key) = key {
            row.insert(format!("{table}.ID"), json!(key));
        }
        row
    }

    fn collect(kind: JoinType) -> Vec<Map<String, Value>> {
        let (left, right) = fixture_rows();
        let mut rows = JoinRows::new(
            left,
            right,
            kind,
            vec!["left.ID".into()],
            vec!["right.ID".into()],
        )
        .unwrap();
        collect_rows(&mut rows)
    }

    fn fixture_rows() -> (TestRows, TestRows) {
        let left = vec![
            row("left", Some(1), "l1"),
            row("left", Some(1), "l2"),
            row("left", Some(2), "l3"),
            row("left", None, "l4"),
        ];
        let right = vec![
            row("right", Some(1), "r1"),
            row("right", Some(1), "r2"),
            row("right", Some(3), "r3"),
            row("right", None, "r4"),
        ];
        (left, right)
    }

    fn collect_planned(kind: JoinType, strategy: JoinStrategy) -> Vec<Map<String, Value>> {
        let (left, right) = fixture_rows();
        let right_numbers = [10, 11, 12, 13];
        let plan = JoinStagePlan {
            strategy,
            fallback_strategy: JoinStrategy::Hash,
            kind: kind.clone(),
            local_fields: vec!["left.ID".into()],
            foreign_fields: vec!["right.ID".into()],
            index_fields: Some(vec!["ID".into()]),
            right_index: None,
            right_order: matches!(strategy, JoinStrategy::Merge).then(|| right_numbers.to_vec()),
        };
        let mut rows = JoinRows::planned(left, right, right_numbers.to_vec(), plan).unwrap();
        collect_rows(&mut rows)
    }

    fn collect_rows(rows: &mut JoinRows) -> Vec<Map<String, Value>> {
        let mut output = Vec::new();
        while let Some(row) = rows.next_row().unwrap() {
            output.push(row);
        }
        output
    }

    #[test]
    fn streams_all_join_types_in_the_existing_order() {
        let inner = collect(JoinType::Inner);
        assert_eq!(
            inner
                .iter()
                .map(|row| (
                    row["left.LABEL"].as_str().unwrap(),
                    row["right.LABEL"].as_str().unwrap()
                ))
                .collect::<Vec<_>>(),
            [("l1", "r1"), ("l1", "r2"), ("l2", "r1"), ("l2", "r2")]
        );

        let left = collect(JoinType::Left);
        assert_eq!(left.len(), 6);
        assert_eq!(left[4]["left.LABEL"], "l3");
        assert!(left[4].get("right.LABEL").is_none());
        assert_eq!(left[5]["left.LABEL"], "l4");

        let right = collect(JoinType::Right);
        assert_eq!(
            right
                .iter()
                .map(|row| (
                    row.get("left.LABEL").and_then(Value::as_str),
                    row["right.LABEL"].as_str().unwrap()
                ))
                .collect::<Vec<_>>(),
            [
                (Some("l1"), "r1"),
                (Some("l2"), "r1"),
                (Some("l1"), "r2"),
                (Some("l2"), "r2"),
                (None, "r3"),
                (None, "r4")
            ]
        );

        let full = collect(JoinType::Full);
        assert_eq!(full.len(), 8);
        assert_eq!(full[4]["left.LABEL"], "l3");
        assert_eq!(full[5]["left.LABEL"], "l4");
        assert_eq!(full[6]["right.LABEL"], "r3");
        assert_eq!(full[7]["right.LABEL"], "r4");
        assert_eq!(collect(JoinType::Semi).len(), 2);
        assert_eq!(collect(JoinType::Anti).len(), 2);
        assert_eq!(collect(JoinType::Cross).len(), 16);
    }

    #[test]
    fn streams_nested_join_types() {
        for kind in [
            JoinType::Inner,
            JoinType::Left,
            JoinType::Right,
            JoinType::Full,
            JoinType::Semi,
            JoinType::Anti,
            JoinType::Cross,
        ] {
            assert_eq!(
                collect_planned(kind.clone(), JoinStrategy::NestedLoop),
                collect(kind),
            );
        }
    }

    #[test]
    fn streams_merge_without_key_leaks() {
        for kind in [
            JoinType::Inner,
            JoinType::Left,
            JoinType::Right,
            JoinType::Semi,
            JoinType::Anti,
        ] {
            assert_eq!(
                collect_planned(kind.clone(), JoinStrategy::Merge),
                collect(kind),
            );
        }
    }
}

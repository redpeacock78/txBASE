use super::super::Row;
use super::combine;
use crate::query::join::JoinError;
use crate::query::join_pipeline::stages::OrderedMergeRows;

pub(in crate::query::join_pipeline::stream) struct MergeJoinRows {
    left: Vec<Row>,
    right: Vec<Row>,
    ordered: OrderedMergeRows,
}

impl MergeJoinRows {
    pub(super) fn with_rows(left: Vec<Row>, right: Vec<Row>, ordered: OrderedMergeRows) -> Self {
        Self {
            left,
            right,
            ordered,
        }
    }

    pub(super) fn next_row(&mut self) -> Result<Option<Row>, JoinError> {
        Ok(self.ordered.next_pair().map(|(left, right)| {
            combine(
                left.map(|position| &self.left[position]),
                right.map(|position| &self.right[position]),
            )
        }))
    }
}

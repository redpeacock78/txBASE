use super::super::Row;
use super::combine;
use crate::query::join::{JoinError, JoinType, MAX_JOIN_ROWS, encoded_key};

pub(in crate::query::join_pipeline::stream) struct NestedJoinRows {
    left: Vec<Row>,
    right: Vec<Row>,
    kind: JoinType,
    local_fields: Vec<String>,
    foreign_fields: Vec<String>,
    left_position: usize,
    right_position: usize,
    match_found: bool,
    matched_right: Vec<bool>,
    unmatched_right_position: usize,
}

impl NestedJoinRows {
    pub(super) fn new(
        left: Vec<Row>,
        right: Vec<Row>,
        kind: JoinType,
        local_fields: Vec<String>,
        foreign_fields: Vec<String>,
    ) -> Result<Self, JoinError> {
        if matches!(&kind, JoinType::Cross)
            && left
                .len()
                .checked_mul(right.len())
                .is_none_or(|pairs| pairs > MAX_JOIN_ROWS)
        {
            return Err(JoinError::Invalid(format!(
                "cross join candidate pairs exceed {MAX_JOIN_ROWS}"
            )));
        }
        let matched_right = if matches!(&kind, JoinType::Full) {
            vec![false; right.len()]
        } else {
            Vec::new()
        };
        Ok(Self {
            left,
            right,
            kind,
            local_fields,
            foreign_fields,
            left_position: 0,
            right_position: 0,
            match_found: false,
            matched_right,
            unmatched_right_position: 0,
        })
    }

    pub(super) fn next_row(&mut self) -> Result<Option<Row>, JoinError> {
        match &self.kind {
            JoinType::Right => self.next_right(),
            JoinType::Cross => Ok(self.next_cross()),
            JoinType::Inner | JoinType::Left | JoinType::Semi | JoinType::Anti | JoinType::Full => {
                self.next_left()
            }
        }
    }

    fn next_left(&mut self) -> Result<Option<Row>, JoinError> {
        loop {
            if self.left_position < self.left.len() {
                if self.right_position < self.right.len() {
                    let right_position = self.right_position;
                    self.right_position += 1;
                    if keys_match(
                        &self.left[self.left_position],
                        &self.right[right_position],
                        &self.local_fields,
                        &self.foreign_fields,
                    )? {
                        self.match_found = true;
                        if matches!(&self.kind, JoinType::Full) {
                            self.matched_right[right_position] = true;
                        }
                        if matches!(&self.kind, JoinType::Semi | JoinType::Anti) {
                            let row = matches!(&self.kind, JoinType::Semi)
                                .then(|| self.left[self.left_position].clone());
                            self.advance_left();
                            if row.is_some() {
                                return Ok(row);
                            }
                        } else {
                            return Ok(Some(combine(
                                Some(&self.left[self.left_position]),
                                Some(&self.right[right_position]),
                            )));
                        }
                    }
                    continue;
                }

                let emit_unmatched = match &self.kind {
                    JoinType::Left | JoinType::Full => !self.match_found,
                    JoinType::Anti => !self.match_found,
                    _ => false,
                };
                let row = emit_unmatched.then(|| {
                    if matches!(&self.kind, JoinType::Left | JoinType::Full) {
                        combine(Some(&self.left[self.left_position]), None)
                    } else {
                        self.left[self.left_position].clone()
                    }
                });
                self.advance_left();
                if row.is_some() {
                    return Ok(row);
                }
            } else if matches!(&self.kind, JoinType::Full) {
                while self.unmatched_right_position < self.right.len() {
                    let position = self.unmatched_right_position;
                    self.unmatched_right_position += 1;
                    if !self.matched_right[position] {
                        return Ok(Some(combine(None, Some(&self.right[position]))));
                    }
                }
                return Ok(None);
            } else {
                return Ok(None);
            }
        }
    }

    fn next_right(&mut self) -> Result<Option<Row>, JoinError> {
        loop {
            if self.right_position >= self.right.len() {
                return Ok(None);
            }
            if self.left_position < self.left.len() {
                let left_position = self.left_position;
                self.left_position += 1;
                if keys_match(
                    &self.left[left_position],
                    &self.right[self.right_position],
                    &self.local_fields,
                    &self.foreign_fields,
                )? {
                    self.match_found = true;
                    return Ok(Some(combine(
                        Some(&self.left[left_position]),
                        Some(&self.right[self.right_position]),
                    )));
                }
                continue;
            }

            let row =
                (!self.match_found).then(|| combine(None, Some(&self.right[self.right_position])));
            self.right_position += 1;
            self.left_position = 0;
            self.match_found = false;
            if row.is_some() {
                return Ok(row);
            }
        }
    }

    fn next_cross(&mut self) -> Option<Row> {
        if self.left_position >= self.left.len() || self.right.is_empty() {
            return None;
        }
        let row = combine(
            Some(&self.left[self.left_position]),
            Some(&self.right[self.right_position]),
        );
        self.right_position += 1;
        if self.right_position == self.right.len() {
            self.right_position = 0;
            self.left_position += 1;
        }
        Some(row)
    }

    fn advance_left(&mut self) {
        self.left_position += 1;
        self.right_position = 0;
        self.match_found = false;
    }
}

fn keys_match(
    left: &Row,
    right: &Row,
    local_fields: &[String],
    foreign_fields: &[String],
) -> Result<bool, JoinError> {
    let left_key = encoded_key(left, local_fields)?;
    let right_key = encoded_key(right, foreign_fields)?;
    Ok(left_key.is_some() && left_key == right_key)
}

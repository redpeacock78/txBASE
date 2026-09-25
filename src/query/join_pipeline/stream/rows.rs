use super::super::super::join::{JoinError, JoinType, MAX_JOIN_ROWS, encoded_key};
use super::Row;
use serde_json::Map;
use std::collections::BTreeMap;

pub(super) struct JoinRows {
    left: Vec<Row>,
    right: Vec<Row>,
    kind: JoinType,
    local_fields: Vec<String>,
    foreign_fields: Vec<String>,
    right_by_key: BTreeMap<String, Vec<usize>>,
    left_by_key: BTreeMap<String, Vec<usize>>,
    matched_right: Vec<bool>,
    left_position: usize,
    right_position: usize,
    match_position: usize,
    left_started: bool,
    left_key: Option<String>,
    right_started: bool,
    right_key: Option<String>,
    unmatched_right_position: usize,
    cross_right_position: usize,
}

impl JoinRows {
    pub(super) fn new(
        left: Vec<Row>,
        right: Vec<Row>,
        kind: JoinType,
        local_fields: Vec<String>,
        foreign_fields: Vec<String>,
    ) -> Result<Self, JoinError> {
        if matches!(&kind, JoinType::Cross) {
            let pair_count = left.len().checked_mul(right.len()).ok_or_else(|| {
                JoinError::Invalid("cross join candidate pair count overflows".into())
            })?;
            if pair_count > MAX_JOIN_ROWS {
                return Err(JoinError::Invalid(format!(
                    "cross join candidate pairs exceed {MAX_JOIN_ROWS}"
                )));
            }
        }

        let mut right_by_key = BTreeMap::new();
        if matches!(
            &kind,
            JoinType::Inner | JoinType::Left | JoinType::Semi | JoinType::Anti | JoinType::Full
        ) {
            for (position, row) in right.iter().enumerate() {
                if let Some(key) = encoded_key(row, &foreign_fields)? {
                    right_by_key
                        .entry(key)
                        .or_insert_with(Vec::new)
                        .push(position);
                }
            }
        }

        let mut left_by_key = BTreeMap::new();
        if matches!(&kind, JoinType::Right) {
            for (position, row) in left.iter().enumerate() {
                if let Some(key) = encoded_key(row, &local_fields)? {
                    left_by_key
                        .entry(key)
                        .or_insert_with(Vec::new)
                        .push(position);
                }
            }
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
            right_by_key,
            left_by_key,
            matched_right,
            left_position: 0,
            right_position: 0,
            match_position: 0,
            left_started: false,
            left_key: None,
            right_started: false,
            right_key: None,
            unmatched_right_position: 0,
            cross_right_position: 0,
        })
    }

    pub(super) fn next_row(&mut self) -> Result<Option<Row>, JoinError> {
        match &self.kind {
            JoinType::Inner | JoinType::Left | JoinType::Semi | JoinType::Anti | JoinType::Full => {
                self.next_left()
            }
            JoinType::Right => self.next_right(),
            JoinType::Cross => Ok(self.next_cross()),
        }
    }

    fn next_left(&mut self) -> Result<Option<Row>, JoinError> {
        loop {
            if self.left_position < self.left.len() {
                if !self.left_started {
                    self.left_key =
                        encoded_key(&self.left[self.left_position], &self.local_fields)?;
                    self.left_started = true;
                    self.match_position = 0;
                }
                let matches = self
                    .left_key
                    .as_ref()
                    .and_then(|key| self.right_by_key.get(key));
                match &self.kind {
                    JoinType::Semi => {
                        let has_match = matches.is_some_and(|rows| !rows.is_empty());
                        let output = has_match.then(|| self.left[self.left_position].clone());
                        self.advance_left();
                        if output.is_some() {
                            return Ok(output);
                        }
                    }
                    JoinType::Anti => {
                        let has_match = matches.is_some_and(|rows| !rows.is_empty());
                        let output = (!has_match).then(|| self.left[self.left_position].clone());
                        self.advance_left();
                        if output.is_some() {
                            return Ok(output);
                        }
                    }
                    JoinType::Inner | JoinType::Left | JoinType::Full => {
                        let right_position = matches
                            .and_then(|rows| rows.get(self.match_position))
                            .copied();
                        if let Some(right_position) = right_position {
                            self.match_position += 1;
                            if matches!(&self.kind, JoinType::Full) {
                                self.matched_right[right_position] = true;
                            }
                            return Ok(Some(combine(
                                Some(&self.left[self.left_position]),
                                Some(&self.right[right_position]),
                            )));
                        }
                        let has_match = matches.is_some_and(|rows| !rows.is_empty());
                        let output = (!has_match
                            && matches!(&self.kind, JoinType::Left | JoinType::Full))
                        .then(|| combine(Some(&self.left[self.left_position]), None));
                        self.advance_left();
                        if output.is_some() {
                            return Ok(output);
                        }
                    }
                    JoinType::Right | JoinType::Cross => unreachable!(),
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
            if !self.right_started {
                self.right_key =
                    encoded_key(&self.right[self.right_position], &self.foreign_fields)?;
                self.right_started = true;
                self.match_position = 0;
            }
            let matches = self
                .right_key
                .as_ref()
                .and_then(|key| self.left_by_key.get(key));
            let left_position = matches
                .and_then(|rows| rows.get(self.match_position))
                .copied();
            if let Some(left_position) = left_position {
                self.match_position += 1;
                return Ok(Some(combine(
                    Some(&self.left[left_position]),
                    Some(&self.right[self.right_position]),
                )));
            }
            let has_match = matches.is_some_and(|rows| !rows.is_empty());
            let output =
                (!has_match).then(|| combine(None, Some(&self.right[self.right_position])));
            self.advance_right();
            if output.is_some() {
                return Ok(output);
            }
        }
    }

    fn next_cross(&mut self) -> Option<Row> {
        if self.left_position >= self.left.len() || self.right.is_empty() {
            return None;
        }
        let output = combine(
            Some(&self.left[self.left_position]),
            Some(&self.right[self.cross_right_position]),
        );
        self.cross_right_position += 1;
        if self.cross_right_position == self.right.len() {
            self.cross_right_position = 0;
            self.left_position += 1;
        }
        Some(output)
    }

    fn advance_left(&mut self) {
        self.left_position += 1;
        self.left_started = false;
        self.left_key = None;
        self.match_position = 0;
    }

    fn advance_right(&mut self) {
        self.right_position += 1;
        self.right_started = false;
        self.right_key = None;
        self.match_position = 0;
    }
}

fn combine(left: Option<&Row>, right: Option<&Row>) -> Row {
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
    use super::JoinRows;
    use crate::query::join::JoinType;
    use serde_json::{Map, Value, json};

    fn row(table: &str, key: Option<i64>, label: &str) -> Map<String, Value> {
        let mut row = Map::from_iter([(format!("{table}.LABEL"), json!(label))]);
        if let Some(key) = key {
            row.insert(format!("{table}.ID"), json!(key));
        }
        row
    }

    fn collect(kind: JoinType) -> Vec<Map<String, Value>> {
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
        let mut rows = JoinRows::new(
            left,
            right,
            kind,
            vec!["left.ID".into()],
            vec!["right.ID".into()],
        )
        .unwrap();
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
}

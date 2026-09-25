use super::super::Row;
use super::combine;
use crate::index::IndexFile;
use crate::query::join::{JoinError, JoinType};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(in crate::query::join_pipeline::stream) struct IndexJoinRows {
    left: Vec<Row>,
    right: Vec<Row>,
    kind: JoinType,
    local_fields: Vec<String>,
    index_fields: Vec<String>,
    index: IndexFile,
    right_positions: BTreeMap<usize, usize>,
    left_position: usize,
    match_position: usize,
    current_matches: Vec<usize>,
    left_started: bool,
}

enum ProbeValues {
    Missing,
    Unsupported,
    Found(Vec<Value>),
}

impl IndexJoinRows {
    pub(super) fn can_use(
        left: &[Row],
        kind: &JoinType,
        local_fields: &[String],
        index: &IndexFile,
        index_fields: &[String],
    ) -> bool {
        if matches!(kind, JoinType::Right | JoinType::Full | JoinType::Cross) {
            return false;
        }
        let fields = index_fields.iter().map(String::as_str).collect::<Vec<_>>();
        index.has_exact_fields(&fields)
            && left
                .iter()
                .all(|row| !matches!(probe_values(row, local_fields), ProbeValues::Unsupported))
    }

    pub(super) fn new(
        left: Vec<Row>,
        right: Vec<Row>,
        right_numbers: Vec<usize>,
        kind: JoinType,
        local_fields: Vec<String>,
        index_fields: Vec<String>,
        index: IndexFile,
    ) -> Result<Self, JoinError> {
        if right.len() != right_numbers.len() {
            return Err(JoinError::Invalid(
                "join index row numbers do not match loaded rows".into(),
            ));
        }
        let right_positions = right_numbers
            .into_iter()
            .enumerate()
            .map(|(position, number)| (number, position))
            .collect();
        Ok(Self {
            left,
            right,
            kind,
            local_fields,
            index_fields,
            index,
            right_positions,
            left_position: 0,
            match_position: 0,
            current_matches: Vec::new(),
            left_started: false,
        })
    }

    pub(super) fn next_row(&mut self) -> Result<Option<Row>, JoinError> {
        loop {
            if self.left_position >= self.left.len() {
                return Ok(None);
            }
            if !self.left_started {
                self.current_matches =
                    match probe_values(&self.left[self.left_position], &self.local_fields) {
                        ProbeValues::Missing => Vec::new(),
                        ProbeValues::Unsupported => unreachable!("index probe was prevalidated"),
                        ProbeValues::Found(values) => {
                            let fields = self
                                .index_fields
                                .iter()
                                .map(String::as_str)
                                .collect::<Vec<_>>();
                            self.index
                                .lookup_eq_for_fields(&fields, &values)
                                .map_err(|error| {
                                    JoinError::Invalid(format!("join index lookup failed: {error}"))
                                })?
                                .ok_or_else(|| {
                                    JoinError::Invalid(
                                        "join index no longer matches planned fields".into(),
                                    )
                                })?
                                .1
                        }
                    };
                self.match_position = 0;
                self.left_started = true;
            }

            match &self.kind {
                JoinType::Semi => {
                    let matched = !self.current_matches.is_empty();
                    let row = matched.then(|| self.left[self.left_position].clone());
                    self.advance_left();
                    if row.is_some() {
                        return Ok(row);
                    }
                }
                JoinType::Anti => {
                    let unmatched = self.current_matches.is_empty();
                    let row = unmatched.then(|| self.left[self.left_position].clone());
                    self.advance_left();
                    if row.is_some() {
                        return Ok(row);
                    }
                }
                JoinType::Inner | JoinType::Left => {
                    if let Some(number) = self.current_matches.get(self.match_position).copied() {
                        self.match_position += 1;
                        let position =
                            self.right_positions.get(&number).copied().ok_or_else(|| {
                                JoinError::Invalid(format!(
                                    "join index references missing active row: {number}"
                                ))
                            })?;
                        return Ok(Some(combine(
                            Some(&self.left[self.left_position]),
                            Some(&self.right[position]),
                        )));
                    }
                    if matches!(&self.kind, JoinType::Left) && self.current_matches.is_empty() {
                        let row = combine(Some(&self.left[self.left_position]), None);
                        self.advance_left();
                        return Ok(Some(row));
                    }
                    self.advance_left();
                }
                JoinType::Right | JoinType::Full | JoinType::Cross => {
                    unreachable!("unsupported join type reached indexed nested loop")
                }
            }
        }
    }

    fn advance_left(&mut self) {
        self.left_position += 1;
        self.match_position = 0;
        self.current_matches.clear();
        self.left_started = false;
    }
}

fn probe_values(row: &Map<String, Value>, fields: &[String]) -> ProbeValues {
    let mut values = Vec::with_capacity(fields.len());
    for field in fields {
        let Some(value) = super::super::super::qualified_value(row, field) else {
            return ProbeValues::Missing;
        };
        if value.is_null() {
            return ProbeValues::Missing;
        }
        if !matches!(value, Value::Bool(_) | Value::Number(_) | Value::String(_)) {
            return ProbeValues::Unsupported;
        }
        values.push(value);
    }
    ProbeValues::Found(values)
}

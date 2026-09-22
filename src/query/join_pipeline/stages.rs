use super::super::join::JoinSource;
use super::super::join::{JoinError, JoinSpec, JoinType, MAX_JOIN_ROWS};
use super::{encoded_key, push_combined, stage_fields, unqualified_field};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(super) fn apply(
    source: &dyn JoinSource,
    left: Vec<Map<String, Value>>,
    right: &[Map<String, Value>],
    right_numbers: &[usize],
    spec: &JoinSpec,
) -> Result<Vec<Map<String, Value>>, JoinError> {
    let (local_fields, foreign_fields) = stage_fields(spec);
    if matches!(&spec.kind, JoinType::Cross) {
        let pair_count = left.len().checked_mul(right.len()).ok_or_else(|| {
            JoinError::Invalid("cross join candidate pair count overflows".into())
        })?;
        if pair_count > MAX_JOIN_ROWS {
            return Err(JoinError::Invalid(format!(
                "cross join candidate pairs exceed {MAX_JOIN_ROWS}"
            )));
        }
        let mut output = Vec::with_capacity(pair_count);
        for left_row in &left {
            for right_row in right {
                push_combined(&mut output, Some(left_row), Some(right_row))?;
            }
        }
        return Ok(output);
    }

    if matches!(&spec.kind, JoinType::Full) {
        return execute_full(left, right, &local_fields, &foreign_fields);
    }

    let index_fields = foreign_fields
        .iter()
        .map(|field| unqualified_field(field, &spec.table).map(str::to_owned))
        .collect::<Option<Vec<_>>>();
    let right_index = if !source.is_historical() {
        if let Some(index_fields) = index_fields.as_deref() {
            let should_try = if matches!(&spec.kind, JoinType::Right) {
                local_fields.len() == index_fields.len()
            } else {
                matches!(
                    super::super::join_strategy::choose(left.len(), right.len(), false),
                    super::super::join_strategy::JoinStrategy::Hash
                )
            };
            source
                .catalog()
                .and_then(|catalog| {
                    should_try.then(|| {
                        super::super::join_index::load_fields(catalog, &spec.table, index_fields)
                    })
                })
                .flatten()
        } else {
            None
        }
    } else {
        None
    };
    if matches!(
        super::super::join_strategy::choose_with_probe_cost(
            left.len(),
            right.len(),
            right_index.as_ref().and_then(|index| {
                index_fields.as_deref().and_then(|fields| {
                    super::super::join_index::equality_probe_cost(index, right.len(), fields)
                })
            }),
        ),
        super::super::join_strategy::JoinStrategy::IndexNestedLoop
    ) {
        if let Some(index) = right_index.as_ref() {
            let output = if matches!(&spec.kind, JoinType::Right) {
                if let Some(index_fields) = index_fields.as_deref() {
                    super::super::join_index::execute_right_stage(
                        &left,
                        right,
                        right_numbers,
                        index,
                        &local_fields,
                        index_fields,
                    )?
                } else {
                    None
                }
            } else {
                match index_fields.as_deref() {
                    Some(index_fields) => super::super::join_index::execute_stage(
                        &left,
                        right,
                        right_numbers,
                        spec,
                        index,
                        &local_fields,
                        index_fields,
                    )?,
                    None => None,
                }
            };
            if let Some(output) = output {
                return Ok(output);
            }
        }
    }

    if matches!(&spec.kind, JoinType::Right) {
        if matches!(
            super::super::join_strategy::choose(left.len(), right.len(), false),
            super::super::join_strategy::JoinStrategy::NestedLoop
        ) {
            return super::super::join_nested::execute_right_stage(
                &left,
                right,
                spec,
                &local_fields,
                &foreign_fields,
            );
        }
        let mut left_by_key = BTreeMap::<String, Vec<usize>>::new();
        for (index, left_row) in left.iter().enumerate() {
            let Some(key) = encoded_key(left_row, &local_fields)? else {
                continue;
            };
            left_by_key.entry(key).or_default().push(index);
        }

        let mut output = Vec::new();
        for right_row in right {
            let matches =
                encoded_key(right_row, &foreign_fields)?.and_then(|key| left_by_key.get(&key));
            if let Some(matches) = matches {
                for &index in matches {
                    push_combined(&mut output, Some(&left[index]), Some(right_row))?;
                }
            } else {
                push_combined(&mut output, None, Some(right_row))?;
            }
        }
        return Ok(output);
    }

    if matches!(
        super::super::join_strategy::choose(left.len(), right.len(), false),
        super::super::join_strategy::JoinStrategy::NestedLoop
    ) {
        return super::super::join_nested::execute_stage(
            &left,
            right,
            spec,
            &local_fields,
            &foreign_fields,
        );
    }

    let mut right_by_key = BTreeMap::<String, Vec<usize>>::new();
    for (index, right_row) in right.iter().enumerate() {
        let Some(key) = encoded_key(right_row, &foreign_fields)? else {
            continue;
        };
        right_by_key.entry(key).or_default().push(index);
    }

    let mut output = Vec::new();
    for left_row in &left {
        let matches = encoded_key(left_row, &local_fields)?.and_then(|key| right_by_key.get(&key));
        let had_matches = matches.is_some_and(|rows| !rows.is_empty());
        match &spec.kind {
            JoinType::Inner => {
                if let Some(matches) = matches {
                    for &index in matches {
                        push_combined(&mut output, Some(left_row), Some(&right[index]))?;
                    }
                }
            }
            JoinType::Left => {
                if let Some(matches) = matches {
                    for &index in matches {
                        push_combined(&mut output, Some(left_row), Some(&right[index]))?;
                    }
                }
                if !had_matches {
                    push_combined(&mut output, Some(left_row), None)?;
                }
            }
            JoinType::Semi if had_matches => output.push(left_row.clone()),
            JoinType::Anti if !had_matches => output.push(left_row.clone()),
            JoinType::Semi | JoinType::Anti => {}
            JoinType::Right | JoinType::Full | JoinType::Cross => {
                unreachable!("join type handled above")
            }
        }
        if output.len() > MAX_JOIN_ROWS {
            return Err(JoinError::Invalid(format!(
                "join result exceeds {MAX_JOIN_ROWS} rows"
            )));
        }
    }
    Ok(output)
}

fn execute_full(
    left: Vec<Map<String, Value>>,
    right: &[Map<String, Value>],
    local_fields: &[String],
    foreign_fields: &[String],
) -> Result<Vec<Map<String, Value>>, JoinError> {
    let mut right_by_key = BTreeMap::<String, Vec<usize>>::new();
    for (index, right_row) in right.iter().enumerate() {
        let Some(key) = encoded_key(right_row, foreign_fields)? else {
            continue;
        };
        right_by_key.entry(key).or_default().push(index);
    }

    let mut matched_right = vec![false; right.len()];
    let mut output = Vec::new();
    for left_row in &left {
        let matches = encoded_key(left_row, local_fields)?.and_then(|key| right_by_key.get(&key));
        if let Some(matches) = matches {
            for &index in matches {
                matched_right[index] = true;
                push_combined(&mut output, Some(left_row), Some(&right[index]))?;
            }
        } else {
            push_combined(&mut output, Some(left_row), None)?;
        }
    }

    for (index, right_row) in right.iter().enumerate() {
        if !matched_right[index] {
            push_combined(&mut output, None, Some(right_row))?;
        }
    }
    Ok(output)
}

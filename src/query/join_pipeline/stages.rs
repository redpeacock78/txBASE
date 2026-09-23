mod cross;
mod full;
mod hash;

use super::super::join::JoinSource;
use super::super::join::{JoinError, JoinSpec, JoinType};
use super::super::join_strategy::JoinCostInput;
use super::{stage_fields, unqualified_field};
use serde_json::{Map, Value};

pub(super) fn apply(
    source: &dyn JoinSource,
    left: Vec<Map<String, Value>>,
    right: &[Map<String, Value>],
    right_numbers: &[usize],
    spec: &JoinSpec,
    cost_input: JoinCostInput,
) -> Result<Vec<Map<String, Value>>, JoinError> {
    let (local_fields, foreign_fields) = stage_fields(spec);
    if matches!(&spec.kind, JoinType::Cross) {
        return cross::execute(&left, right);
    }

    if matches!(&spec.kind, JoinType::Full) {
        return full::execute(left, right, &local_fields, &foreign_fields);
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
                    super::super::join_strategy::choose_with_costs(
                        left.len(),
                        right.len(),
                        None,
                        None,
                        cost_input,
                    ),
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
        super::super::join_strategy::choose_with_costs(
            left.len(),
            right.len(),
            right_index.as_ref().and_then(|index| {
                index_fields.as_deref().and_then(|fields| {
                    super::super::join_index::equality_probe_cost(index, right.len(), fields)
                })
            }),
            None,
            cost_input,
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
            super::super::join_strategy::choose_with_costs(
                left.len(),
                right.len(),
                None,
                None,
                cost_input,
            ),
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
        return hash::execute_right(&left, right, &local_fields, &foreign_fields);
    }

    if matches!(
        super::super::join_strategy::choose_with_costs(
            left.len(),
            right.len(),
            None,
            None,
            cost_input,
        ),
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
    hash::execute_left(&left, right, spec, &local_fields, &foreign_fields)
}

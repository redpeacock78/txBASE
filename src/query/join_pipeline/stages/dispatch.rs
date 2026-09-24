use super::super::super::join::JoinSource;
use super::super::super::join::{JoinError, JoinSpec, JoinType};
use super::super::super::join_strategy::JoinCostInput;
use super::super::{stage_fields, unqualified_field};
use super::{full, hash, merge};
use serde_json::{Map, Value};

pub(crate) fn apply(
    source: &dyn JoinSource,
    left: Vec<Map<String, Value>>,
    right: &[Map<String, Value>],
    right_numbers: &[usize],
    spec: &JoinSpec,
    cost_input: JoinCostInput,
) -> Result<Vec<Map<String, Value>>, JoinError> {
    let (local_fields, foreign_fields) = stage_fields(spec);
    if matches!(&spec.kind, JoinType::Cross) {
        return super::cross::execute(&left, right);
    }

    if matches!(&spec.kind, JoinType::Full) {
        return full::execute(left, right, &local_fields, &foreign_fields);
    }

    let index_fields = foreign_fields
        .iter()
        .map(|field| unqualified_field(field, &spec.table).map(str::to_owned))
        .collect::<Option<Vec<_>>>();
    let right_index = if !source.is_historical()
        && left.len().saturating_mul(right.len())
            > super::super::super::join_strategy::NESTED_LOOP_PAIR_LIMIT
    {
        index_fields.as_deref().and_then(|index_fields| {
            source.catalog().and_then(|catalog| {
                super::super::super::join_index::load_fields(catalog, &spec.table, index_fields)
            })
        })
    } else {
        None
    };
    let right_ordered = right_index.as_ref().and_then(|index| {
        index_fields
            .as_deref()
            .and_then(|fields| super::super::super::join_index::ordered_from_index(index, fields))
    });
    let cost_input = JoinCostInput {
        merge_sort_work: right_ordered
            .as_ref()
            .map(|_| {
                super::super::super::join_strategy::ordered_merge_sort_work(
                    left.len(),
                    local_fields.len(),
                )
            })
            .unwrap_or_default(),
        ..cost_input
    };
    let strategy = super::super::super::join_strategy::choose_with_costs(
        left.len(),
        right.len(),
        right_index.as_ref().and_then(|index| {
            index_fields.as_deref().and_then(|fields| {
                super::super::super::join_index::equality_probe_cost(index, right.len(), fields)
            })
        }),
        right_ordered.as_ref().map(|ordered| ordered.page_reads),
        cost_input,
    );
    if matches!(
        strategy,
        super::super::super::join_strategy::JoinStrategy::Merge
    ) {
        if let Some(ordered) = right_ordered.as_ref() {
            return merge::execute(
                &left,
                right,
                right_numbers,
                &ordered.records,
                spec,
                &local_fields,
                &foreign_fields,
            );
        }
    }
    if matches!(
        strategy,
        super::super::super::join_strategy::JoinStrategy::IndexNestedLoop
    ) {
        if let Some(index) = right_index.as_ref() {
            let output = if matches!(&spec.kind, JoinType::Right) {
                if let Some(index_fields) = index_fields.as_deref() {
                    super::super::super::join_index::execute_right_stage(
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
                    Some(index_fields) => super::super::super::join_index::execute_stage(
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
            super::super::super::join_strategy::choose_with_costs(
                left.len(),
                right.len(),
                None,
                None,
                cost_input,
            ),
            super::super::super::join_strategy::JoinStrategy::NestedLoop
        ) {
            return super::super::super::join_nested::execute_right_stage(
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
        super::super::super::join_strategy::choose_with_costs(
            left.len(),
            right.len(),
            None,
            None,
            cost_input,
        ),
        super::super::super::join_strategy::JoinStrategy::NestedLoop
    ) {
        return super::super::super::join_nested::execute_stage(
            &left,
            right,
            spec,
            &local_fields,
            &foreign_fields,
        );
    }
    hash::execute_left(&left, right, spec, &local_fields, &foreign_fields)
}

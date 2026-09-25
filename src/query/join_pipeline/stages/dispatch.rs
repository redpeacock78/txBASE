use super::super::super::join::{JoinError, JoinSpec, JoinType};
use super::super::super::join_strategy::{JoinCostInput, JoinStrategy, NESTED_LOOP_PAIR_LIMIT};
use super::super::super::{join_index, join_strategy};
use super::super::{stage_fields, unqualified_field};
use super::{full, hash, merge};
use crate::index::IndexFile;
use serde_json::{Map, Value};

pub(in crate::query::join_pipeline) struct JoinStagePlan {
    pub(in crate::query::join_pipeline) strategy: JoinStrategy,
    pub(in crate::query::join_pipeline) fallback_strategy: JoinStrategy,
    pub(in crate::query::join_pipeline) kind: JoinType,
    pub(in crate::query::join_pipeline) local_fields: Vec<String>,
    pub(in crate::query::join_pipeline) foreign_fields: Vec<String>,
    pub(in crate::query::join_pipeline) index_fields: Option<Vec<String>>,
    pub(in crate::query::join_pipeline) right_index: Option<IndexFile>,
    pub(in crate::query::join_pipeline) right_order: Option<Vec<usize>>,
}

pub(in crate::query::join_pipeline) fn plan(
    left_count: usize,
    right_count: usize,
    spec: &JoinSpec,
    right_index: Option<IndexFile>,
    mut cost_input: JoinCostInput,
) -> JoinStagePlan {
    let (local_fields, foreign_fields) = stage_fields(spec);
    if matches!(&spec.kind, JoinType::Cross | JoinType::Full) {
        return JoinStagePlan {
            strategy: if matches!(&spec.kind, JoinType::Cross) {
                JoinStrategy::NestedLoop
            } else {
                JoinStrategy::Hash
            },
            fallback_strategy: JoinStrategy::Hash,
            kind: spec.kind.clone(),
            local_fields,
            foreign_fields,
            index_fields: None,
            right_index: None,
            right_order: None,
        };
    }

    let index_fields = foreign_fields
        .iter()
        .map(|field| unqualified_field(field, &spec.table).map(str::to_owned))
        .collect::<Option<Vec<_>>>();
    let right_index = if left_count.saturating_mul(right_count) > NESTED_LOOP_PAIR_LIMIT {
        index_fields.as_deref().and_then(|fields| {
            right_index.filter(|index| {
                index.has_exact_fields(&fields.iter().map(String::as_str).collect::<Vec<_>>())
            })
        })
    } else {
        None
    };
    let right_ordered = right_index.as_ref().and_then(|index| {
        index_fields
            .as_deref()
            .and_then(|fields| join_index::ordered_from_index(index, fields))
    });
    cost_input.merge_sort_work = right_ordered
        .as_ref()
        .map(|_| join_strategy::ordered_merge_sort_work(left_count, local_fields.len()))
        .unwrap_or_default();

    let strategy = join_strategy::choose_with_costs(
        left_count,
        right_count,
        right_index.as_ref().and_then(|index| {
            index_fields
                .as_deref()
                .and_then(|fields| join_index::equality_probe_cost(index, right_count, fields))
        }),
        right_ordered.as_ref().map(|ordered| ordered.page_reads),
        cost_input,
    );
    let fallback_strategy =
        join_strategy::choose_with_costs(left_count, right_count, None, None, cost_input);

    JoinStagePlan {
        strategy,
        fallback_strategy,
        kind: spec.kind.clone(),
        local_fields,
        foreign_fields,
        index_fields,
        right_index,
        right_order: right_ordered.map(|ordered| ordered.records),
    }
}

pub(in crate::query::join_pipeline) fn apply(
    left: Vec<Map<String, Value>>,
    right: &[Map<String, Value>],
    right_numbers: &[usize],
    spec: &JoinSpec,
    plan: JoinStagePlan,
) -> Result<Vec<Map<String, Value>>, JoinError> {
    if matches!(&plan.kind, JoinType::Cross) {
        return super::cross::execute(&left, right);
    }
    if matches!(&plan.kind, JoinType::Full) {
        return full::execute(left, right, &plan.local_fields, &plan.foreign_fields);
    }

    if matches!(plan.strategy, JoinStrategy::Merge) {
        if let Some(right_order) = plan.right_order.as_ref() {
            return merge::execute(
                &left,
                right,
                right_numbers,
                right_order,
                spec,
                &plan.local_fields,
                &plan.foreign_fields,
            );
        }
    }
    if matches!(plan.strategy, JoinStrategy::IndexNestedLoop) {
        if let (Some(index), Some(index_fields)) =
            (plan.right_index.as_ref(), plan.index_fields.as_deref())
        {
            let output = if matches!(&plan.kind, JoinType::Right) {
                super::super::super::join_index::execute_right_stage(
                    &left,
                    right,
                    right_numbers,
                    index,
                    &plan.local_fields,
                    index_fields,
                )?
            } else {
                super::super::super::join_index::execute_stage(
                    &left,
                    right,
                    right_numbers,
                    spec,
                    index,
                    &plan.local_fields,
                    index_fields,
                )?
            };
            if let Some(output) = output {
                return Ok(output);
            }
        }
    }

    if matches!(plan.fallback_strategy, JoinStrategy::NestedLoop) {
        if matches!(&plan.kind, JoinType::Right) {
            return super::super::super::join_nested::execute_right_stage(
                &left,
                right,
                spec,
                &plan.local_fields,
                &plan.foreign_fields,
            );
        }
        return super::super::super::join_nested::execute_stage(
            &left,
            right,
            spec,
            &plan.local_fields,
            &plan.foreign_fields,
        );
    }
    if matches!(&plan.kind, JoinType::Right) {
        hash::execute_right(&left, right, &plan.local_fields, &plan.foreign_fields)
    } else {
        hash::execute_left(&left, right, spec, &plan.local_fields, &plan.foreign_fields)
    }
}

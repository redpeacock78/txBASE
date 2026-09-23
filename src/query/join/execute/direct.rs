mod cross;
mod left;
mod right;

use super::super::super::{join_merge, join_nested, join_strategy};
use super::super::{JoinError, JoinRequest, JoinType};
use super::JoinSource;
use crate::catalog::Catalog;
use crate::dbf::DbfRecord;
use serde_json::Value;

pub(super) struct DirectJoinContext<'a> {
    pub(super) current_catalog: Option<&'a Catalog>,
    pub(super) large_join: bool,
    pub(super) left_records: &'a [&'a DbfRecord],
    pub(super) right_records: &'a [&'a DbfRecord],
    pub(super) request: &'a JoinRequest,
    pub(super) local_fields: &'a [String],
    pub(super) foreign_fields: &'a [String],
    pub(super) cost_input: join_strategy::JoinCostInput,
}

pub(super) fn execute<S: JoinSource>(
    source: &S,
    request: &JoinRequest,
) -> Result<Vec<Value>, JoinError> {
    let (local_fields, foreign_fields) = super::super::join_fields(request)?;
    let (left, right) = source.open_tables(&request.from, &request.join.table)?;
    let left_records = left.active_records().collect::<Vec<_>>();
    let right_records = right.active_records().collect::<Vec<_>>();

    if matches!(&request.join.kind, JoinType::Cross) {
        return cross::execute(request, &left_records, &right_records);
    }

    let current_catalog = source.catalog();
    let left_page_reads = super::logical_page_reads(&left);
    let right_page_reads = super::logical_page_reads(&right);
    let estimated_output_rows = super::estimate_join_rows(
        &left_records,
        &right_records,
        &local_fields,
        &foreign_fields,
        &request.join.kind,
    )?;
    let output_columns = request.projection.len().max(1);
    let left_cost_input = join_strategy::JoinCostInput {
        outer_page_reads: left_page_reads,
        inner_page_reads: right_page_reads,
        output_rows: estimated_output_rows,
        output_columns,
        ..join_strategy::JoinCostInput::default()
    };
    let right_cost_input = join_strategy::JoinCostInput {
        outer_page_reads: right_page_reads,
        inner_page_reads: left_page_reads,
        output_rows: estimated_output_rows,
        output_columns,
        ..join_strategy::JoinCostInput::default()
    };
    let large_join = current_catalog.is_some()
        && !source.is_historical()
        && left_records.len().saturating_mul(right_records.len())
            > join_strategy::NESTED_LOOP_PAIR_LIMIT;
    let left_ordered = if large_join {
        current_catalog.and_then(|catalog| {
            super::super::super::join_index::load_ordered(catalog, &request.from, &local_fields)
        })
    } else {
        None
    };
    let right_ordered = if large_join {
        current_catalog.and_then(|catalog| {
            super::super::super::join_index::load_ordered(
                catalog,
                &request.join.table,
                &foreign_fields,
            )
        })
    } else {
        None
    };
    let merge_page_reads = left_ordered
        .as_ref()
        .zip(right_ordered.as_ref())
        .map(|(left, right)| left.page_reads.saturating_add(right.page_reads));
    let mut direct_context = DirectJoinContext {
        current_catalog,
        large_join,
        left_records: &left_records,
        right_records: &right_records,
        request,
        local_fields: &local_fields,
        foreign_fields: &foreign_fields,
        cost_input: left_cost_input,
    };

    if matches!(&request.join.kind, JoinType::Right) {
        if matches!(
            join_strategy::choose_with_costs(
                right_records.len(),
                left_records.len(),
                None,
                merge_page_reads,
                right_cost_input,
            ),
            join_strategy::JoinStrategy::Merge
        ) {
            if let (Some(left_order), Some(right_order)) =
                (left_ordered.as_ref(), right_ordered.as_ref())
            {
                return join_merge::execute(
                    &left_records,
                    &right_records,
                    &left_order.records,
                    &right_order.records,
                    request,
                    &local_fields,
                    &foreign_fields,
                );
            }
        }
        direct_context.cost_input = right_cost_input;
        return right::execute(&direct_context);
    }

    if matches!(
        join_strategy::choose_with_costs(
            left_records.len(),
            right_records.len(),
            None,
            merge_page_reads,
            left_cost_input,
        ),
        join_strategy::JoinStrategy::Merge
    ) {
        if let (Some(left_order), Some(right_order)) =
            (left_ordered.as_ref(), right_ordered.as_ref())
        {
            return join_merge::execute(
                &left_records,
                &right_records,
                &left_order.records,
                &right_order.records,
                request,
                &local_fields,
                &foreign_fields,
            );
        }
    }

    if matches!(&request.join.kind, JoinType::Full) {
        return join_nested::execute_full_join(
            &left_records,
            &right_records,
            request,
            &local_fields,
            &foreign_fields,
        );
    }

    left::execute(&direct_context)
}

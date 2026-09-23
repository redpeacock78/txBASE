use super::{PlannedAccess, QueryCost, QueryPlan, QueryRequest};
use crate::index::IndexFile;

pub(super) fn estimated_cost(
    access: &PlannedAccess,
    index_file: &IndexFile,
    active_record_count: usize,
    request: &QueryRequest,
) -> QueryCost {
    let record_count = access
        .records
        .as_ref()
        .map_or(active_record_count, Vec::len);
    let record_reads = estimated_record_reads(access, index_file, record_count);
    let remaining_sort = request.sort.len() > access.ordered_prefix;
    let sort_cost = if remaining_sort {
        estimated_sort_cost(record_count, access.ordered_prefix)
    } else {
        0
    };
    let index_traversal = index_traversal_cost(access, index_file);
    let total = record_reads
        .saturating_add(record_count)
        .saturating_add(index_traversal)
        .saturating_add(sort_cost);
    QueryCost {
        candidate_rows: record_count,
        index_traversal,
        record_reads,
        filter_evaluations: record_count,
        sort_work: sort_cost,
        total,
    }
}

pub(super) fn selection_cost(
    access: &PlannedAccess,
    index_file: &IndexFile,
    active_record_count: usize,
    request: &QueryRequest,
) -> usize {
    // Preserve the existing plan ranking while the explanation exposes the additional work terms.
    let cost = estimated_cost(access, index_file, active_record_count, request);
    cost.candidate_rows
        .saturating_add(cost.index_traversal)
        .saturating_add(cost.sort_work)
}

fn estimated_record_reads(
    access: &PlannedAccess,
    index_file: &IndexFile,
    candidate_rows: usize,
) -> usize {
    match &access.plan {
        QueryPlan::IndexIntersection { fields, .. } => fields
            .iter()
            .map(|field| {
                index_file
                    .equality_selectivity_estimate(field)
                    .unwrap_or(candidate_rows)
            })
            .fold(candidate_rows, usize::saturating_add),
        _ => candidate_rows,
    }
}

fn index_traversal_cost(access: &PlannedAccess, index_file: &IndexFile) -> usize {
    match &access.plan {
        QueryPlan::TableScan => 0,
        QueryPlan::EqualityIndex { name, .. }
        | QueryPlan::CompoundEqualityIndex { name, .. }
        | QueryPlan::CompoundEqualityPrefixIndex { name, .. }
        | QueryPlan::RangeIndex { name, .. }
        | QueryPlan::OrderedIndex { name, .. }
        | QueryPlan::OrderedIndexPrefix { name, .. }
        | QueryPlan::CompoundOrderedIndex { name, .. } => {
            index_file.index_traversal_cost(name).unwrap_or_default()
        }
        QueryPlan::IndexIntersection { names, .. } => names
            .iter()
            .filter_map(|name| index_file.index_traversal_cost(name))
            .sum(),
    }
}

fn estimated_sort_cost(record_count: usize, ordered_prefix: usize) -> usize {
    if record_count < 2 {
        return 0;
    }
    let full_sort = record_count.saturating_mul(record_count.ilog2() as usize);
    // ponytail: group cardinalities are not persisted; discount one full pass when an ordered prefix exists.
    if ordered_prefix == 0 {
        full_sort
    } else {
        full_sort.saturating_sub(record_count)
    }
}

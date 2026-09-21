use super::{PlannedAccess, QueryPlan, QueryRequest};
use crate::index::IndexFile;

pub(super) fn estimated_cost(
    access: &PlannedAccess,
    index_file: &IndexFile,
    active_record_count: usize,
    request: &QueryRequest,
) -> usize {
    let record_count = access
        .records
        .as_ref()
        .map_or(active_record_count, Vec::len);
    let remaining_sort = request.sort.len() > access.ordered_prefix;
    let sort_cost = if remaining_sort {
        estimated_sort_cost(record_count, access.ordered_prefix)
    } else {
        0
    };
    record_count
        .saturating_add(index_traversal_cost(access, index_file))
        .saturating_add(sort_cost)
}

fn index_traversal_cost(access: &PlannedAccess, index_file: &IndexFile) -> usize {
    match &access.plan {
        QueryPlan::TableScan => 0,
        QueryPlan::EqualityIndex { name, .. }
        | QueryPlan::CompoundEqualityIndex { name, .. }
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

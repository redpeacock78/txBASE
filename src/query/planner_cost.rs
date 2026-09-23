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
    let index_page_reads = index_page_reads(access, index_file);
    let record_page_reads = estimated_record_page_reads(
        access,
        index_file.source_dbf_page_count(),
        active_record_count,
        record_count,
    );
    let total = record_reads
        .saturating_add(record_count)
        .saturating_add(index_traversal)
        .saturating_add(index_page_reads)
        .saturating_add(record_page_reads)
        .saturating_add(sort_cost);
    QueryCost {
        candidate_rows: record_count,
        index_traversal,
        index_page_reads,
        record_reads,
        record_page_reads,
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
) -> (usize, usize) {
    let cost = estimated_cost(access, index_file, active_record_count, request);
    (
        cost.candidate_rows
            .saturating_add(cost.index_traversal)
            .saturating_add(cost.sort_work),
        cost.index_page_reads.saturating_add(cost.record_page_reads),
    )
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

fn index_page_reads(access: &PlannedAccess, index_file: &IndexFile) -> usize {
    matches!(&access.plan, QueryPlan::TableScan)
        .then_some(0)
        .unwrap_or_else(|| index_file.estimated_page_count())
}

fn estimated_record_page_reads(
    access: &PlannedAccess,
    table_page_count: usize,
    active_record_count: usize,
    candidate_rows: usize,
) -> usize {
    if candidate_rows == 0 {
        return 0;
    }
    if matches!(&access.plan, QueryPlan::TableScan) || candidate_rows >= active_record_count {
        return table_page_count;
    }
    // A candidate can occupy a different page in the worst case. This deliberately bounds
    // random fetches without pretending to know the filesystem cache or record distribution.
    candidate_rows.min(table_page_count)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{IndexDefinition, IndexFile, sidecar_path};
    use std::fs;

    #[test]
    fn bounds_logical_page_estimates_for_scan_and_candidates() {
        let path =
            std::env::temp_dir().join(format!("txbase-planner-cost-{}.dbf", std::process::id()));
        let sidecar = sidecar_path(&path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&sidecar);
        let bytes = include_str!("../../tests/fixtures/users.dbf.hex")
            .split_whitespace()
            .map(|token| u8::from_str_radix(token, 16).unwrap())
            .collect::<Vec<_>>();
        fs::write(&path, bytes).unwrap();
        let index = IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")]).unwrap();
        index.save(&path).unwrap();

        let table_scan = PlannedAccess {
            plan: QueryPlan::TableScan,
            records: None,
            ordered_prefix: 0,
        };
        let indexed = PlannedAccess {
            plan: QueryPlan::EqualityIndex {
                name: "NAME".into(),
                field: "NAME".into(),
            },
            records: Some(vec![1]),
            ordered_prefix: 0,
        };
        assert_eq!(index_page_reads(&table_scan, &index), 0);
        assert!(index_page_reads(&indexed, &index) > 0);
        assert_eq!(estimated_record_page_reads(&indexed, 3, 2, 0), 0);
        assert_eq!(estimated_record_page_reads(&table_scan, 3, 2, 2), 3);
        assert_eq!(estimated_record_page_reads(&indexed, 3, 2, 1), 1);
        assert_eq!(estimated_record_page_reads(&indexed, 3, 2, 2), 3);

        let cost = estimated_cost(&indexed, &index, 2, &QueryRequest::default());
        assert!(cost.index_page_reads > 0);
        assert!(cost.record_page_reads > 0);

        let _ = fs::remove_file(path);
        let _ = fs::remove_file(sidecar);
    }
}

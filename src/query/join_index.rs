use crate::catalog::Catalog;
use crate::index::IndexFile;
use serde_json::Map;

use super::join_strategy::JoinProbeCost;

mod execute;
mod probe;

pub(super) use execute::{join as execute_join, right_join as execute_right_join};
pub(super) use execute::{right_stage as execute_right_stage, stage as execute_stage};

pub(super) struct OrderedIndex {
    pub(super) records: Vec<usize>,
    pub(super) page_reads: usize,
}

pub(super) fn load_fields(
    catalog: &Catalog,
    table_name: &str,
    fields: &[String],
) -> Option<IndexFile> {
    if catalog.is_historical() {
        return None;
    }
    let path = catalog.table_path(table_name)?;
    let index = IndexFile::load(path).ok()?;
    let fields = fields.iter().map(String::as_str).collect::<Vec<_>>();
    index.has_exact_fields(&fields).then_some(index)
}

pub(super) fn equality_probe_cost(
    index: &IndexFile,
    inner_count: usize,
    fields: &[String],
) -> Option<JoinProbeCost> {
    let fields = fields.iter().map(String::as_str).collect::<Vec<_>>();
    let fanout = index.equality_fanout_estimate(&fields)?;
    Some(JoinProbeCost {
        per_probe: super::join_strategy::index_probe_cost(inner_count, fanout),
        index_page_reads: index.estimated_page_count(),
        record_page_reads_per_probe: fanout.min(index.source_dbf_page_count()),
    })
}

pub(super) fn load_ordered(
    catalog: &Catalog,
    table_name: &str,
    fields: &[String],
) -> Option<OrderedIndex> {
    let index = load_fields(catalog, table_name, fields)?;
    ordered_from_index(&index, fields)
}

pub(super) fn ordered_from_index(index: &IndexFile, fields: &[String]) -> Option<OrderedIndex> {
    let records = ordered_records(index, fields)?;
    Some(OrderedIndex {
        records,
        page_reads: index.estimated_page_count(),
    })
}

pub(super) fn ordered_records(index: &IndexFile, fields: &[String]) -> Option<Vec<usize>> {
    if fields.is_empty() {
        return None;
    }
    let fields = fields.iter().map(String::as_str).collect::<Vec<_>>();
    if fields.len() == 1 {
        return index
            .lookup_ordered_for_field(fields[0], false)
            .ok()?
            .map(|(_, records)| records);
    }
    let directions = vec![1; fields.len()];
    index
        .lookup_ordered_for_fields(&fields, &directions, &Map::new())
        .ok()?
        .map(|(_, _, _, records)| records)
}

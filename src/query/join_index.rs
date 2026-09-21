use crate::catalog::Catalog;
use crate::index::IndexFile;
use serde_json::Map;

mod execute;
mod probe;

pub(super) use execute::{join as execute_join, right_join as execute_right_join};
pub(super) use execute::{right_stage as execute_right_stage, stage as execute_stage};

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
) -> Option<usize> {
    let fields = fields.iter().map(String::as_str).collect::<Vec<_>>();
    let fanout = index.equality_fanout_estimate(&fields)?;
    Some(super::join_strategy::index_probe_cost(inner_count, fanout))
}

pub(super) fn load_ordered_fields(
    catalog: &Catalog,
    table_name: &str,
    fields: &[String],
) -> Option<Vec<usize>> {
    let index = load_fields(catalog, table_name, fields)?;
    ordered_records(&index, fields)
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

use super::QueryRequest;
use crate::index::IndexFile;
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryPlan {
    TableScan,
    EqualityIndex { name: String, field: String },
}

pub(super) struct PlannedAccess {
    pub(super) plan: QueryPlan,
    pub(super) records: Option<Vec<usize>>,
}

pub(super) fn choose(dbf_path: &Path, request: &QueryRequest) -> PlannedAccess {
    let Ok(index_file) = IndexFile::load(dbf_path) else {
        return table_scan();
    };
    for (field, condition) in &request.filter {
        let Some(value) = equality_value(condition) else {
            continue;
        };
        let Ok(Some((name, records))) = index_file.lookup_eq_for_field(field, value) else {
            continue;
        };
        return PlannedAccess {
            plan: QueryPlan::EqualityIndex {
                name,
                field: field.clone(),
            },
            records: Some(records),
        };
    }
    table_scan()
}

fn table_scan() -> PlannedAccess {
    PlannedAccess {
        plan: QueryPlan::TableScan,
        records: None,
    }
}

fn equality_value(condition: &Value) -> Option<&Value> {
    let Some(object) = condition.as_object() else {
        return (!condition.is_array() && !condition.is_object()).then_some(condition);
    };
    object.get("$eq")
}

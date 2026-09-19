use super::QueryRequest;
use crate::index::IndexFile;
use serde_json::Value;
use std::path::Path;

type RangeBound<'a> = (&'a Value, bool);
type RangeBounds<'a> = (Option<RangeBound<'a>>, Option<RangeBound<'a>>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryPlan {
    TableScan,
    EqualityIndex {
        name: String,
        field: String,
    },
    RangeIndex {
        name: String,
        field: String,
    },
    OrderedIndex {
        name: String,
        field: String,
        direction: i8,
    },
}

pub(super) struct PlannedAccess {
    pub(super) plan: QueryPlan,
    pub(super) records: Option<Vec<usize>>,
    pub(super) ordered: bool,
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
            ordered: false,
        };
    }
    for (field, condition) in &request.filter {
        let Some((lower, upper)) = range_bounds(condition) else {
            continue;
        };
        let Ok(Some((name, records))) = index_file.lookup_range_for_field(field, lower, upper)
        else {
            continue;
        };
        return PlannedAccess {
            plan: QueryPlan::RangeIndex {
                name,
                field: field.clone(),
            },
            records: Some(records),
            ordered: false,
        };
    }
    if request.sort.len() == 1 {
        let (field, direction) = request.sort.iter().next().expect("sort has one field");
        let Ok(Some((name, records))) =
            index_file.lookup_ordered_for_field(field, *direction == -1)
        else {
            return table_scan();
        };
        return PlannedAccess {
            plan: QueryPlan::OrderedIndex {
                name,
                field: field.clone(),
                direction: *direction,
            },
            records: Some(records),
            ordered: true,
        };
    }
    table_scan()
}

fn table_scan() -> PlannedAccess {
    PlannedAccess {
        plan: QueryPlan::TableScan,
        records: None,
        ordered: false,
    }
}

fn equality_value(condition: &Value) -> Option<&Value> {
    let Some(object) = condition.as_object() else {
        return (!condition.is_array() && !condition.is_object()).then_some(condition);
    };
    object.get("$eq")
}

fn range_bounds(condition: &Value) -> Option<RangeBounds<'_>> {
    let object = condition.as_object()?;
    let mut lower = None;
    let mut upper = None;
    for (operator, value) in object {
        match operator.as_str() {
            "$gt" => set_bound(&mut lower, value, false)?,
            "$gte" => set_bound(&mut lower, value, true)?,
            "$lt" => set_bound(&mut upper, value, false)?,
            "$lte" => set_bound(&mut upper, value, true)?,
            _ => {}
        }
    }
    (lower.is_some() || upper.is_some()).then_some((lower, upper))
}

fn set_bound<'a>(
    target: &mut Option<RangeBound<'a>>,
    value: &'a Value,
    inclusive: bool,
) -> Option<()> {
    target.replace((value, inclusive)).is_none().then_some(())
}

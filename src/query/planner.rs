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
    IndexIntersection {
        names: Vec<String>,
        fields: Vec<String>,
    },
    OrderedIndex {
        name: String,
        field: String,
        direction: i8,
    },
    OrderedIndexPrefix {
        name: String,
        field: String,
        direction: i8,
    },
    CompoundOrderedIndex {
        name: String,
        fields: Vec<String>,
        direction: i8,
    },
}

pub(super) struct PlannedAccess {
    pub(super) plan: QueryPlan,
    pub(super) records: Option<Vec<usize>>,
    pub(super) ordered_prefix: usize,
}

pub(super) fn choose(dbf_path: &Path, request: &QueryRequest) -> PlannedAccess {
    let Ok(index_file) = IndexFile::load(dbf_path) else {
        return table_scan();
    };
    let mut equality_fields = request
        .filter
        .iter()
        .filter_map(|(field, condition)| equality_value(condition).map(|_| field.clone()))
        .collect::<Vec<_>>();
    // ponytail: uniform distribution estimate; add histograms only if skew makes plan choices measurable.
    equality_fields.sort_by_key(|field| {
        index_file
            .equality_selectivity_estimate(field)
            .unwrap_or(usize::MAX)
    });
    let mut equality_indexes = Vec::new();
    for field in equality_fields {
        let Some(condition) = request.filter.get(&field) else {
            continue;
        };
        let Some(value) = equality_value(condition) else {
            continue;
        };
        let Ok(Some((name, records))) = index_file.lookup_eq_for_field(&field, value) else {
            continue;
        };
        equality_indexes.push((name, field.clone(), records));
    }
    if let Some((name, field, first_records)) = equality_indexes.first() {
        let mut records = first_records.clone();
        for (_, _, candidates) in equality_indexes.iter().skip(1) {
            records.retain(|record| candidates.binary_search(record).is_ok());
        }
        let plan = if equality_indexes.len() == 1 {
            QueryPlan::EqualityIndex {
                name: name.clone(),
                field: field.clone(),
            }
        } else {
            QueryPlan::IndexIntersection {
                names: equality_indexes
                    .iter()
                    .map(|(name, _, _)| name.clone())
                    .collect(),
                fields: equality_indexes
                    .iter()
                    .map(|(_, field, _)| field.clone())
                    .collect(),
            }
        };
        return PlannedAccess {
            plan,
            records: Some(records),
            ordered_prefix: 0,
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
            ordered_prefix: 0,
        };
    }
    if !request.sort.is_empty() {
        let (_first_field, first_direction) =
            request.sort.iter().next().expect("sort is non-empty");
        let all_directions_match = request
            .sort
            .values()
            .all(|direction| direction == first_direction);
        if request.sort.len() > 1 && all_directions_match {
            let fields = request.sort.keys().map(String::as_str).collect::<Vec<_>>();
            if let Ok(Some((name, index_fields, records))) =
                index_file.lookup_ordered_for_fields(&fields, *first_direction == -1)
            {
                return PlannedAccess {
                    plan: QueryPlan::CompoundOrderedIndex {
                        name,
                        fields: index_fields,
                        direction: *first_direction,
                    },
                    records: Some(records),
                    ordered_prefix: request.sort.len(),
                };
            }
        }
        let (field, direction) = request.sort.iter().next().expect("sort has one field");
        let Ok(Some((name, records))) =
            index_file.lookup_ordered_for_field(field, *direction == -1)
        else {
            return table_scan();
        };
        let plan = if request.sort.len() == 1 {
            QueryPlan::OrderedIndex {
                name,
                field: field.clone(),
                direction: *direction,
            }
        } else {
            QueryPlan::OrderedIndexPrefix {
                name,
                field: field.clone(),
                direction: *direction,
            }
        };
        return PlannedAccess {
            plan,
            records: Some(records),
            ordered_prefix: 1,
        };
    }
    table_scan()
}

fn table_scan() -> PlannedAccess {
    PlannedAccess {
        plan: QueryPlan::TableScan,
        records: None,
        ordered_prefix: 0,
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

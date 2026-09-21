use super::QueryRequest;
use crate::index::IndexFile;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

#[path = "planner_cost.rs"]
mod cost;

type RangeBound<'a> = (&'a Value, bool);
type RangeBounds<'a> = (Option<RangeBound<'a>>, Option<RangeBound<'a>>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
        directions: Vec<i8>,
    },
}

pub(super) struct PlannedAccess {
    pub(super) plan: QueryPlan,
    pub(super) records: Option<Vec<usize>>,
    pub(super) ordered_prefix: usize,
}

pub(super) fn choose(dbf_path: &Path, request: &QueryRequest) -> PlannedAccess {
    // ponytail: custom collation uses a table scan; add collation-aware index keys only if needed.
    if request.collation.is_some() {
        return table_scan();
    }
    let Ok(index_file) = IndexFile::load(dbf_path) else {
        return table_scan();
    };
    let active_record_count = index_file.active_record_count();
    let mut candidates = vec![table_scan()];
    if let Some(access) = choose_equality(&index_file, request) {
        candidates.push(access);
    }
    if let Some(access) = choose_range(&index_file, request, active_record_count) {
        candidates.push(access);
    }
    if let Some(access) = choose_ordered(&index_file, request) {
        candidates.push(access);
    }

    // ponytail: bounded record, traversal, and sort cost; add I/O/cache terms only with measurements and a contract.
    candidates
        .into_iter()
        .min_by_key(|access| {
            cost::estimated_cost(access, &index_file, active_record_count, request)
        })
        .unwrap_or_else(table_scan)
}

fn choose_equality(index_file: &IndexFile, request: &QueryRequest) -> Option<PlannedAccess> {
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
    let (name, field, first_records) = equality_indexes.first()?;
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
    Some(PlannedAccess {
        plan,
        records: Some(records),
        ordered_prefix: 0,
    })
}

fn choose_range(
    index_file: &IndexFile,
    request: &QueryRequest,
    active_record_count: usize,
) -> Option<PlannedAccess> {
    let mut range_fields = request
        .filter
        .iter()
        .filter_map(|(field, condition)| range_bounds(condition).map(|_| field.clone()))
        .collect::<Vec<_>>();
    // ponytail: histogram estimates order candidate construction; exact candidates choose the path.
    range_fields.sort_by_key(|field| {
        let condition = request.filter.get(field).expect("range field exists");
        let (lower, upper) = range_bounds(condition).expect("range field has bounds");
        index_file
            .range_selectivity_estimate(field, lower, upper)
            .unwrap_or(usize::MAX)
    });
    let mut candidates = Vec::new();
    for field in range_fields {
        let Some(condition) = request.filter.get(&field) else {
            continue;
        };
        let Some((lower, upper)) = range_bounds(condition) else {
            continue;
        };
        if let Ok(Some((name, records))) =
            index_file.lookup_compound_range_for_field(&field, lower, upper, &request.filter)
        {
            candidates.push(PlannedAccess {
                plan: QueryPlan::RangeIndex {
                    name,
                    field: field.clone(),
                },
                records: Some(records),
                ordered_prefix: 0,
            });
        }
        if let Ok(Some((name, records))) = index_file.lookup_range_for_field(&field, lower, upper) {
            candidates.push(PlannedAccess {
                plan: QueryPlan::RangeIndex { name, field },
                records: Some(records),
                ordered_prefix: 0,
            });
        }
    }
    candidates
        .into_iter()
        .min_by_key(|access| cost::estimated_cost(access, index_file, active_record_count, request))
}

fn choose_ordered(index_file: &IndexFile, request: &QueryRequest) -> Option<PlannedAccess> {
    if !request.sort.is_empty() {
        let fields = request.sort.keys().map(String::as_str).collect::<Vec<_>>();
        let directions = request.sort.values().copied().collect::<Vec<_>>();
        if let Ok(Some((name, index_fields, index_directions, records))) =
            index_file.lookup_ordered_for_fields(&fields, &directions, &request.filter)
        {
            let plan = if index_fields.len() == 1 {
                QueryPlan::OrderedIndex {
                    name,
                    field: index_fields[0].clone(),
                    direction: directions[0],
                }
            } else {
                QueryPlan::CompoundOrderedIndex {
                    name,
                    fields: index_fields,
                    directions: index_directions,
                }
            };
            return Some(PlannedAccess {
                plan,
                records: Some(records),
                ordered_prefix: request.sort.len(),
            });
        }
        let (field, direction) = request.sort.iter().next().expect("sort has one field");
        let Ok(Some((name, records))) =
            index_file.lookup_ordered_for_field(field, *direction == -1)
        else {
            return None;
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
        return Some(PlannedAccess {
            plan,
            records: Some(records),
            ordered_prefix: 1,
        });
    }
    None
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

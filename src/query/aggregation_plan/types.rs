use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::super::expression::NumericExpression;

#[derive(Debug, Clone)]
pub struct GroupSpec {
    pub key_field: Option<String>,
    pub accumulators: Vec<AccumulatorSpec>,
}

#[derive(Debug, Clone)]
pub struct AggregationPlan {
    pub input: Vec<InputStage>,
    pub group_matches: Vec<Map<String, Value>>,
    pub group: Option<GroupSpec>,
    pub bucket: Option<BucketSpec>,
    pub sort_by_count: Option<String>,
    pub count: Option<String>,
    pub distinct: Option<String>,
    pub projection: Option<BTreeMap<String, i8>>,
    pub sort: Option<IndexMap<String, i8>>,
    pub skip: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct BucketSpec {
    pub group_by: String,
    pub boundaries: Vec<Value>,
    pub default: Option<Value>,
    pub output: GroupSpec,
}

#[derive(Debug, Clone)]
pub struct UnwindSpec {
    pub field: String,
    pub include_array_index: Option<String>,
    pub preserve_null_and_empty: bool,
}

#[derive(Debug, Clone)]
pub enum SetExpression {
    Field(String),
    Literal(Value),
    Numeric(NumericExpression),
    IfNull(Box<SetExpression>, Box<SetExpression>),
}

#[derive(Debug, Clone)]
pub enum InputStage {
    Match(Map<String, Value>),
    Unwind(UnwindSpec),
    Set(BTreeMap<String, SetExpression>),
    Project(BTreeMap<String, i8>),
    Sort(IndexMap<String, i8>),
    Skip(u64),
    Limit(u64),
}

#[derive(Debug, Clone)]
pub struct AccumulatorSpec {
    pub name: String,
    pub kind: AccumulatorKind,
}

#[derive(Debug, Clone)]
pub enum AccumulatorKind {
    Count,
    Average(NumericExpression),
    StdDevPop(NumericExpression),
    StdDevSamp(NumericExpression),
    Sum(NumericExpression),
    Min(String),
    Max(String),
    First(String),
    Last(String),
    Push(String),
    AddToSet(String),
}

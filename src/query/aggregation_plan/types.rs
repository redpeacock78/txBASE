use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct GroupSpec {
    pub key_field: Option<String>,
    pub accumulators: Vec<AccumulatorSpec>,
}

#[derive(Debug, Clone)]
pub struct AggregationPlan {
    pub matches: Vec<Map<String, Value>>,
    pub unwinds: Vec<String>,
    pub group_matches: Vec<Map<String, Value>>,
    pub group: Option<GroupSpec>,
    pub count: Option<String>,
    pub distinct: Option<String>,
    pub projection: Option<BTreeMap<String, i8>>,
    pub sort: Option<IndexMap<String, i8>>,
    pub skip: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct AccumulatorSpec {
    pub name: String,
    pub kind: AccumulatorKind,
}

#[derive(Debug, Clone)]
pub enum AccumulatorKind {
    Count,
    Average(String),
    Sum(SumOperand),
    Min(String),
    Max(String),
    First(String),
    Last(String),
    Push(String),
    AddToSet(String),
}

#[derive(Debug, Clone)]
pub enum SumOperand {
    Field(String),
    Literal(serde_json::Number),
}

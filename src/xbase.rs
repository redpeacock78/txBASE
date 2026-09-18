use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationMethod {
    Get,
    Query,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperationIr {
    pub method: OperationMethod,
    pub path: String,
    pub body: Option<Value>,
}

impl OperationMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Query => "QUERY",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OperationMethod {
    Get,
    Query,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationIr {
    pub method: OperationMethod,
    pub path: String,
    pub body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationBatch {
    pub operations: Vec<OperationIr>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_methods_use_http_tokens() {
        assert_eq!(OperationMethod::Get.as_str(), "GET");
        assert_eq!(OperationMethod::Query.as_str(), "QUERY");
        assert_eq!(OperationMethod::Post.as_str(), "POST");
        assert_eq!(OperationMethod::Put.as_str(), "PUT");
        assert_eq!(OperationMethod::Patch.as_str(), "PATCH");
        assert_eq!(OperationMethod::Delete.as_str(), "DELETE");
    }

    #[test]
    fn operation_batch_uses_one_transaction_document() {
        let batch: OperationBatch = serde_json::from_str(
            r#"{"operations":[{"method":"PATCH","path":"/records/1","body":{"NAME":"Alice"}}]}"#,
        )
        .unwrap();
        assert_eq!(batch.operations.len(), 1);
        assert_eq!(batch.operations[0].method, OperationMethod::Patch);
    }

    #[test]
    fn operation_batch_rejects_unknown_fields() {
        let error = serde_json::from_str::<OperationBatch>(r#"{"operations":[],"extra":true}"#)
            .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }
}

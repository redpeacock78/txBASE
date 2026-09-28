use serde::{Deserialize, Serialize, de::Error as DeError};
use serde_json::Value;

pub const MAX_OPERATION_BATCH: usize = 1_000;

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
    #[serde(deserialize_with = "deserialize_operations")]
    pub operations: Vec<OperationIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionBatch {
    #[serde(deserialize_with = "deserialize_transaction_steps")]
    pub operations: Vec<TransactionStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TransactionStep {
    Mutation(OperationIr),
    Command(TransactionCommand),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum TransactionCommand {
    SetConstraints {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        table: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        all: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        names: Vec<String>,
        mode: crate::ConstraintMode,
    },
}

fn is_false(value: &bool) -> bool {
    !value
}

fn deserialize_operations<'de, D>(deserializer: D) -> Result<Vec<OperationIr>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let operations = Vec::<OperationIr>::deserialize(deserializer)?;
    if operations.len() > MAX_OPERATION_BATCH {
        return Err(D::Error::custom(format!(
            "operation count exceeds {MAX_OPERATION_BATCH}"
        )));
    }
    Ok(operations)
}

fn deserialize_transaction_steps<'de, D>(deserializer: D) -> Result<Vec<TransactionStep>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let steps = Vec::<TransactionStep>::deserialize(deserializer)?;
    if steps.len() > MAX_OPERATION_BATCH {
        return Err(D::Error::custom(format!(
            "operation count exceeds {MAX_OPERATION_BATCH}"
        )));
    }
    if !steps
        .iter()
        .any(|step| matches!(step, TransactionStep::Mutation(_)))
    {
        return Err(D::Error::custom(
            "transaction must contain at least one record mutation",
        ));
    }
    for step in &steps {
        step.validate().map_err(D::Error::custom)?;
    }
    Ok(steps)
}

impl TransactionStep {
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Mutation(operation)
                if matches!(
                    operation.method,
                    OperationMethod::Get | OperationMethod::Query
                ) =>
            {
                Err(format!("{} is a read operation", operation.method.as_str()))
            }
            Self::Mutation(_) => Ok(()),
            Self::Command(TransactionCommand::SetConstraints {
                table, all, names, ..
            }) => {
                if table.as_ref().is_some_and(|name| name.trim().is_empty()) {
                    return Err("constraint table name must not be empty".into());
                }
                if *all && !names.is_empty() {
                    return Err("all and names cannot be used together".into());
                }
                if !*all && names.is_empty() {
                    return Err("constraint names must not be empty unless all is true".into());
                }
                if names.iter().any(|name| name.trim().is_empty()) {
                    return Err("constraint names must not be empty".into());
                }
                Ok(())
            }
        }
    }

    pub(crate) fn is_constraint_command(&self) -> bool {
        matches!(self, Self::Command(_))
    }

    pub(crate) fn into_mutation(self) -> Option<OperationIr> {
        match self {
            Self::Mutation(operation) => Some(operation),
            Self::Command(_) => None,
        }
    }
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

    #[test]
    fn operation_batch_rejects_more_than_the_shared_limit() {
        let operations = (0..=MAX_OPERATION_BATCH)
            .map(|_| {
                serde_json::json!({
                    "method": "DELETE",
                    "path": "/records/1"
                })
            })
            .collect::<Vec<_>>();
        let error =
            serde_json::from_value::<OperationBatch>(serde_json::json!({"operations": operations}))
                .unwrap_err();
        assert!(error.to_string().contains("operation count"));
    }

    #[test]
    fn transaction_batch_preserves_order_and_decodes_constraint_commands() {
        let batch: TransactionBatch = serde_json::from_str(
            r#"{"operations":[{"type":"setConstraints","names":["users_name_unique"],"mode":"deferred"},{"method":"POST","path":"/records","body":{"ID":3}},{"type":"setConstraints","all":true,"mode":"immediate"}]}"#,
        )
        .unwrap();

        assert!(matches!(
            &batch.operations[0],
            TransactionStep::Command(TransactionCommand::SetConstraints {
                all: false,
                names,
                mode: crate::ConstraintMode::Deferred,
                ..
            }) if names == &["users_name_unique"]
        ));
        assert!(matches!(
            &batch.operations[1],
            TransactionStep::Mutation(OperationIr {
                method: OperationMethod::Post,
                ..
            })
        ));
        assert!(matches!(
            &batch.operations[2],
            TransactionStep::Command(TransactionCommand::SetConstraints {
                all: true,
                mode: crate::ConstraintMode::Immediate,
                ..
            })
        ));
    }

    #[test]
    fn transaction_batch_rejects_control_only_and_invalid_constraint_selections() {
        for body in [
            r#"{"operations":[{"type":"setConstraints","all":true,"mode":"deferred"}]}"#,
            r#"{"operations":[{"method":"PATCH","path":"/records/1"},{"type":"setConstraints","all":true,"names":["users_name_unique"],"mode":"deferred"}]}"#,
            r#"{"operations":[{"method":"PATCH","path":"/records/1"},{"type":"setConstraints","names":[],"mode":"deferred"}]}"#,
        ] {
            assert!(serde_json::from_str::<TransactionBatch>(body).is_err());
        }
    }

    #[test]
    fn transaction_batch_counts_control_steps_toward_the_shared_limit() {
        let operations = (0..=MAX_OPERATION_BATCH)
            .map(|_| {
                serde_json::json!({
                    "method": "DELETE",
                    "path": "/records/1"
                })
            })
            .collect::<Vec<_>>();
        let error = serde_json::from_value::<TransactionBatch>(
            serde_json::json!({"operations": operations}),
        )
        .unwrap_err();
        assert!(error.to_string().contains("operation count"));
    }
}

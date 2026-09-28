use super::ReplicationError;
use crate::xbase::{
    MAX_OPERATION_BATCH, OperationIr, OperationMethod, TransactionCommand, TransactionStep,
};
use serde::{Deserialize, Serialize};

pub const REPLICATION_ENTRY_VERSION: u16 = 1;
const REPLICATION_ENTRY_STEPS_VERSION: u16 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicationEntry {
    pub version: u16,
    pub term: u64,
    pub index: u64,
    pub transaction_id: u64,
    pub schema_tag: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<OperationIr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<TransactionStep>,
}

impl ReplicationEntry {
    pub fn new(
        term: u64,
        index: u64,
        transaction_id: u64,
        schema_tag: String,
        operations: Vec<OperationIr>,
    ) -> Result<Self, ReplicationError> {
        let entry = Self {
            version: REPLICATION_ENTRY_VERSION,
            term,
            index,
            transaction_id,
            schema_tag,
            operations,
            steps: Vec::new(),
        };
        entry.validate()?;
        Ok(entry)
    }

    pub(super) fn new_with_steps(
        term: u64,
        index: u64,
        transaction_id: u64,
        schema_tag: String,
        steps: Vec<TransactionStep>,
    ) -> Result<Self, ReplicationError> {
        if !steps.iter().any(TransactionStep::is_constraint_command) {
            let operations = steps
                .into_iter()
                .map(TransactionStep::into_mutation)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    ReplicationError::Invalid(
                        "constraint command requires a version 2 replication entry".into(),
                    )
                })?;
            return Self::new(term, index, transaction_id, schema_tag, operations);
        }
        let entry = Self {
            version: REPLICATION_ENTRY_STEPS_VERSION,
            term,
            index,
            transaction_id,
            schema_tag,
            operations: Vec::new(),
            steps,
        };
        entry.validate()?;
        Ok(entry)
    }

    pub(super) fn transaction_steps(&self) -> Vec<TransactionStep> {
        if self.version == REPLICATION_ENTRY_VERSION {
            self.operations
                .iter()
                .cloned()
                .map(TransactionStep::Mutation)
                .collect()
        } else {
            self.steps.clone()
        }
    }

    pub fn validate(&self) -> Result<(), ReplicationError> {
        if !matches!(
            self.version,
            REPLICATION_ENTRY_VERSION | REPLICATION_ENTRY_STEPS_VERSION
        ) {
            return Err(ReplicationError::Invalid(format!(
                "unsupported replication entry version: {}",
                self.version
            )));
        }
        if self.term == 0 || self.index == 0 || self.transaction_id == 0 {
            return Err(ReplicationError::Invalid(
                "term, index, and transaction_id must be positive".into(),
            ));
        }
        if self.schema_tag.trim().is_empty() {
            return Err(ReplicationError::Invalid(
                "schema_tag must not be empty".into(),
            ));
        }
        let steps = self.transaction_steps();
        if (self.version == REPLICATION_ENTRY_VERSION
            && (self.operations.is_empty() || !self.steps.is_empty()))
            || (self.version == REPLICATION_ENTRY_STEPS_VERSION
                && (!self.operations.is_empty()
                    || self.steps.is_empty()
                    || !self
                        .steps
                        .iter()
                        .any(TransactionStep::is_constraint_command)))
        {
            return Err(ReplicationError::Invalid(
                "replication entry fields do not match their version".into(),
            ));
        }
        if steps.is_empty() {
            return Err(ReplicationError::Invalid(
                "operations must not be empty".into(),
            ));
        }
        if steps.len() > MAX_OPERATION_BATCH {
            return Err(ReplicationError::Invalid(format!(
                "operation count exceeds {MAX_OPERATION_BATCH}"
            )));
        }
        let mut has_mutation = false;
        for step in &steps {
            step.validate().map_err(ReplicationError::Invalid)?;
            match step {
                TransactionStep::Mutation(operation) => {
                    has_mutation = true;
                    if matches!(
                        operation.method,
                        OperationMethod::Get | OperationMethod::Query
                    ) {
                        return Err(ReplicationError::Invalid(format!(
                            "{} is a read operation",
                            operation.method.as_str()
                        )));
                    }
                }
                TransactionStep::Command(TransactionCommand::SetConstraints {
                    table, all, ..
                }) if !all && table.is_none() => {
                    return Err(ReplicationError::Invalid(
                        "table is required when setting named constraints in a replication entry"
                            .into(),
                    ));
                }
                TransactionStep::Command(_) => {}
            }
        }
        if !has_mutation {
            return Err(ReplicationError::Invalid(
                "replication entry must contain a record mutation".into(),
            ));
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, ReplicationError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        if bytes.len() > super::MAX_REPLICATION_ENTRY_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "replication entry JSON exceeds {} bytes",
                super::MAX_REPLICATION_ENTRY_BYTES
            )));
        }
        Ok(bytes)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ReplicationError> {
        if bytes.len() > super::MAX_REPLICATION_ENTRY_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "replication entry JSON exceeds {} bytes",
                super::MAX_REPLICATION_ENTRY_BYTES
            )));
        }
        let entry: Self = serde_json::from_slice(bytes)
            .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        entry.validate()?;
        Ok(entry)
    }
}

use crate::xbase::{MAX_OPERATION_BATCH, TransactionStep};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RAFT_COMMAND_VERSION: u16 = 2;
pub const MAX_RAFT_CLIENT_ID_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "camelCase", deny_unknown_fields)]
pub enum RaftCommandPrecondition {
    Catalog {
        if_match: Option<String>,
        if_none_match: Option<String>,
    },
    Table {
        name: String,
        if_match: Option<String>,
        if_none_match: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaftCommand {
    pub version: u16,
    pub client_id: String,
    pub sequence: u64,
    pub expected_catalog_tag: String,
    pub precondition: Option<RaftCommandPrecondition>,
    pub request_fingerprint: Vec<u8>,
    pub steps: Vec<TransactionStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RaftRejection {
    CatalogChanged,
    PreconditionFailed,
    ClientSequenceConflict,
    ClientSequenceTooOld,
    ClientSequenceGap,
    InvalidTransaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
pub enum RaftResponseResult {
    Applied { transaction_id: u64 },
    Rejected { reason: RaftRejection },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaftResponse {
    pub sequence: u64,
    pub result: RaftResponseResult,
}

impl RaftCommand {
    pub fn new(
        client_id: String,
        sequence: u64,
        expected_catalog_tag: String,
        precondition: Option<RaftCommandPrecondition>,
        steps: Vec<TransactionStep>,
    ) -> Result<Self, String> {
        let mut command = Self {
            version: RAFT_COMMAND_VERSION,
            client_id,
            sequence,
            expected_catalog_tag,
            precondition,
            request_fingerprint: Vec::new(),
            steps,
        };
        command.request_fingerprint =
            Self::request_fingerprint(&command.precondition, &command.steps)?;
        command.validate()?;
        Ok(command)
    }

    pub fn request_fingerprint(
        precondition: &Option<RaftCommandPrecondition>,
        steps: &[TransactionStep],
    ) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(&("txbase-raft-command-v2", precondition, steps))
            .map_err(|error| error.to_string())?;
        Ok(Sha256::digest(bytes).to_vec())
    }

    pub fn with_request_fingerprint(mut self, fingerprint: Vec<u8>) -> Result<Self, String> {
        if fingerprint.len() != 32 {
            return Err("request_fingerprint must be a SHA-256 digest".into());
        }
        self.request_fingerprint = fingerprint;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != RAFT_COMMAND_VERSION {
            return Err(format!(
                "unsupported Raft command version: {}",
                self.version
            ));
        }
        if self.client_id.is_empty()
            || self.client_id.len() > MAX_RAFT_CLIENT_ID_BYTES
            || !self
                .client_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(format!(
                "client_id must be 1 to {MAX_RAFT_CLIENT_ID_BYTES} ASCII letters, digits, '.', '_', or '-'"
            ));
        }
        if self.sequence == 0 {
            return Err("client sequence must be positive".into());
        }
        if self.expected_catalog_tag.trim().is_empty() {
            return Err("expected_catalog_tag must not be empty".into());
        }
        if self.request_fingerprint.len() != 32 {
            return Err("request_fingerprint must be a SHA-256 digest".into());
        }
        if self.steps.is_empty() || self.steps.len() > MAX_OPERATION_BATCH {
            return Err(format!(
                "transaction must contain 1 to {MAX_OPERATION_BATCH} steps"
            ));
        }
        if !self
            .steps
            .iter()
            .any(|step| matches!(step, TransactionStep::Mutation(_)))
        {
            return Err("transaction must contain at least one record mutation".into());
        }
        for step in &self.steps {
            step.validate()?;
        }
        if let Some(precondition) = &self.precondition {
            let (name, if_match, if_none_match) = match precondition {
                RaftCommandPrecondition::Catalog {
                    if_match,
                    if_none_match,
                } => (None, if_match, if_none_match),
                RaftCommandPrecondition::Table {
                    name,
                    if_match,
                    if_none_match,
                } => (Some(name), if_match, if_none_match),
            };
            if name.is_some_and(|value| value.trim().is_empty()) {
                return Err("precondition table name must not be empty".into());
            }
            if if_match.is_none() && if_none_match.is_none() {
                return Err("precondition must include if_match or if_none_match".into());
            }
            if if_match
                .iter()
                .chain(if_none_match.iter())
                .any(|tag| tag.trim().is_empty())
            {
                return Err("precondition tags must not be empty".into());
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        if bytes.len() > crate::MAX_JSON_INPUT_BYTES {
            return Err(format!(
                "Raft command exceeds {} bytes",
                crate::MAX_JSON_INPUT_BYTES
            ));
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| error.to_string())
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > crate::MAX_JSON_INPUT_BYTES {
            return Err(format!(
                "Raft command exceeds {} bytes",
                crate::MAX_JSON_INPUT_BYTES
            ));
        }
        let command: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        command.validate()?;
        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xbase::{OperationIr, OperationMethod};
    use serde_json::json;

    #[test]
    fn request_fingerprint_ignores_catalog_tag_but_tracks_mutation() {
        let operation = |age: i64| {
            TransactionStep::Mutation(OperationIr {
                method: OperationMethod::Post,
                path: "/users/records".into(),
                body: Some(json!({"AGE": age})),
            })
        };
        let command = |catalog_tag: &str, age: i64| {
            RaftCommand::new(
                "client".into(),
                1,
                catalog_tag.into(),
                None,
                vec![operation(age)],
            )
            .unwrap()
        };

        assert_eq!(
            command("catalog-before", 42).request_fingerprint,
            command("catalog-after", 42).request_fingerprint
        );
        assert_ne!(
            command("catalog-before", 42).request_fingerprint,
            command("catalog-before", 43).request_fingerprint
        );
    }

    fn valid_command() -> RaftCommand {
        RaftCommand::new(
            "cli-client.1".into(),
            1,
            "\"txbase-0123456789abcdef\"".into(),
            None,
            vec![TransactionStep::Mutation(OperationIr {
                method: OperationMethod::Post,
                path: "/users/records".into(),
                body: Some(json!({"ID": 1})),
            })],
        )
        .unwrap()
    }

    #[test]
    fn command_round_trips_with_validation() {
        let command = valid_command();
        assert_eq!(
            RaftCommand::from_json(&command.to_json().unwrap()).unwrap(),
            command
        );
    }

    #[test]
    fn command_rejects_invalid_client_and_sequence() {
        let mut command = valid_command();
        command.client_id = "client id".into();
        assert!(command.validate().unwrap_err().contains("client_id"));

        command = valid_command();
        command.sequence = 0;
        assert!(command.validate().unwrap_err().contains("sequence"));
    }

    #[test]
    fn command_rejects_reads_and_empty_preconditions() {
        let mut command = valid_command();
        command.steps[0] = TransactionStep::Mutation(OperationIr {
            method: OperationMethod::Get,
            path: "/users/records".into(),
            body: None,
        });
        assert!(command.validate().unwrap_err().contains("read operation"));

        command = valid_command();
        command.precondition = Some(RaftCommandPrecondition::Catalog {
            if_match: None,
            if_none_match: None,
        });
        assert!(command.validate().unwrap_err().contains("if_match"));
    }

    #[test]
    fn command_rejects_oversized_json() {
        let mut command = valid_command();
        command.steps[0] = TransactionStep::Mutation(OperationIr {
            method: OperationMethod::Post,
            path: "/users/records".into(),
            body: Some(json!({"payload": "x".repeat(crate::MAX_JSON_INPUT_BYTES)})),
        });
        assert!(command.validate().unwrap_err().contains("exceeds"));
    }
}

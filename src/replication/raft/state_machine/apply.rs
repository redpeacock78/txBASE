use super::super::{
    RaftCommand, RaftCommandPrecondition, RaftRejection, RaftResponse, RaftResponseResult,
    TypeConfig,
};
use super::{
    ClientResult, RAFT_STATE_SIDECAR_NAME, RaftApplicationState, save_state, valid_client_id,
};
use crate::catalog::{Catalog, CatalogError, CatalogTransactionError, CommitPrecondition};
use openraft::{Entry, EntryPayload, StoredMembership};
use sha2::{Digest, Sha256};

#[derive(Debug)]
enum SequenceDecision {
    Accept,
    Return(RaftResponse),
}

fn sequence_decision(
    state: &RaftApplicationState,
    command: &RaftCommand,
    fingerprint: &[u8],
) -> SequenceDecision {
    let rejection = |reason| RaftResponse {
        sequence: command.sequence,
        result: RaftResponseResult::Rejected { reason },
    };
    let Some(previous) = state.clients.get(&command.client_id) else {
        return if command.sequence == 1 {
            SequenceDecision::Accept
        } else {
            SequenceDecision::Return(rejection(RaftRejection::ClientSequenceGap))
        };
    };
    if command.sequence == previous.sequence {
        return if fingerprint == previous.fingerprint {
            SequenceDecision::Return(previous.response.clone())
        } else {
            SequenceDecision::Return(rejection(RaftRejection::ClientSequenceConflict))
        };
    }
    if command.sequence < previous.sequence {
        return SequenceDecision::Return(rejection(RaftRejection::ClientSequenceTooOld));
    }
    if previous.sequence.checked_add(1) != Some(command.sequence) {
        return SequenceDecision::Return(rejection(RaftRejection::ClientSequenceGap));
    }
    SequenceDecision::Accept
}

fn command_precondition<'a>(
    command: &'a RaftCommand,
    transaction_id: u64,
) -> CommitPrecondition<'a> {
    match &command.precondition {
        Some(RaftCommandPrecondition::Catalog {
            if_match,
            if_none_match,
        }) => CommitPrecondition::Catalog {
            expected_transaction_id: Some(transaction_id),
            expected_catalog_tag: Some(&command.expected_catalog_tag),
            if_match: if_match.as_deref(),
            if_none_match: if_none_match.as_deref(),
        },
        Some(RaftCommandPrecondition::Table {
            name,
            if_match,
            if_none_match,
        }) => CommitPrecondition::Table {
            expected_transaction_id: Some(transaction_id),
            expected_catalog_tag: Some(&command.expected_catalog_tag),
            name,
            if_match: if_match.as_deref(),
            if_none_match: if_none_match.as_deref(),
        },
        None => CommitPrecondition::Catalog {
            expected_transaction_id: Some(transaction_id),
            expected_catalog_tag: Some(&command.expected_catalog_tag),
            if_match: None,
            if_none_match: None,
        },
    }
}

fn transaction_rejection(error: CatalogTransactionError) -> Result<RaftRejection, String> {
    match error {
        CatalogTransactionError::Invalid(_) => Ok(RaftRejection::InvalidTransaction),
        CatalogTransactionError::CatalogTagChanged { .. } => Ok(RaftRejection::CatalogChanged),
        CatalogTransactionError::PreconditionFailed { .. } => Ok(RaftRejection::PreconditionFailed),
        CatalogTransactionError::Catalog(CatalogError::Invalid(_)) => {
            Ok(RaftRejection::InvalidTransaction)
        }
        other => Err(format!("Raft catalog apply failed: {other}")),
    }
}

pub(super) fn apply_entries(
    catalog: &mut Catalog,
    entries: Vec<Entry<TypeConfig>>,
) -> Result<Vec<Option<RaftResponse>>, String> {
    let (mut state, mut state_bytes) = super::read_state(catalog)?;
    let mut responses = Vec::with_capacity(entries.len());
    for entry in entries {
        match state.last_applied {
            Some(previous) if entry.log_id.index <= previous.index => {
                return Err(format!(
                    "Raft apply log index {} does not advance past {}",
                    entry.log_id.index, previous.index
                ));
            }
            _ => {}
        }
        let mut next = state.clone();
        next.last_applied = Some(entry.log_id);
        next.genesis = false;
        match entry.payload {
            EntryPayload::Blank => {
                state_bytes = save_state(catalog, &state_bytes, &next)?;
                state = next;
                responses.push(None);
            }
            EntryPayload::Membership(membership) => {
                next.last_membership = StoredMembership::new(Some(entry.log_id), membership);
                state_bytes = save_state(catalog, &state_bytes, &next)?;
                state = next;
                responses.push(None);
            }
            EntryPayload::Normal(command) => {
                let response = apply_command(catalog, &mut state, &mut state_bytes, next, command)?;
                responses.push(Some(response));
            }
        }
    }
    Ok(responses)
}

fn apply_command(
    catalog: &Catalog,
    state: &mut RaftApplicationState,
    state_bytes: &mut Vec<u8>,
    mut next: RaftApplicationState,
    command: RaftCommand,
) -> Result<RaftResponse, String> {
    let sequence = command.sequence;
    let client_id = command.client_id.clone();
    let encoded = serde_json::to_vec(&command).map_err(|error| error.to_string())?;
    let fingerprint = Sha256::digest(encoded).to_vec();
    if !valid_client_id(&command.client_id) || command.sequence == 0 {
        let response = rejected(sequence, RaftRejection::InvalidTransaction);
        *state_bytes = save_state(catalog, state_bytes, &next)?;
        *state = next;
        return Ok(response);
    }
    match sequence_decision(state, &command, &fingerprint) {
        SequenceDecision::Return(response) => {
            *state_bytes = save_state(catalog, state_bytes, &next)?;
            *state = next;
            return Ok(response);
        }
        SequenceDecision::Accept => {}
    }
    if command.validate().is_err() {
        let response = rejected(sequence, RaftRejection::InvalidTransaction);
        next.clients.insert(
            client_id,
            ClientResult {
                sequence,
                fingerprint,
                response: response.clone(),
            },
        );
        *state_bytes = save_state(catalog, state_bytes, &next)?;
        *state = next;
        return Ok(response);
    }

    let precondition = command_precondition(&command, state.catalog_transaction_id);
    let mut prepared = next.clone();
    prepared.catalog_transaction_id = u64::MAX;
    prepared.clients.insert(
        client_id.clone(),
        ClientResult {
            sequence,
            fingerprint: fingerprint.clone(),
            response: RaftResponse {
                sequence,
                result: RaftResponseResult::Applied {
                    transaction_id: u64::MAX,
                },
            },
        },
    );
    prepared.to_bytes()?;

    let client_fingerprint = fingerprint.clone();
    let mut committed_state = next.clone();
    let commit = catalog.commit_steps_with_sidecar_factory(
        &command.steps,
        precondition,
        RAFT_STATE_SIDECAR_NAME,
        Some(state_bytes.clone()),
        move |transaction_id| {
            let response = RaftResponse {
                sequence,
                result: RaftResponseResult::Applied { transaction_id },
            };
            committed_state.catalog_transaction_id = transaction_id;
            committed_state.clients.insert(
                client_id,
                ClientResult {
                    sequence,
                    fingerprint: client_fingerprint,
                    response,
                },
            );
            committed_state
                .to_bytes()
                .expect("prevalidated Raft state serialization remains valid")
        },
    );
    match commit {
        Ok((transaction_id, bytes)) => {
            let committed = RaftApplicationState::from_bytes(&bytes)?;
            if committed.catalog_transaction_id != transaction_id {
                return Err("Raft state sidecar has the wrong catalog transaction".into());
            }
            let response = committed
                .clients
                .get(&command.client_id)
                .ok_or_else(|| "Raft state sidecar lost the client response".to_owned())?
                .response
                .clone();
            *state = committed;
            *state_bytes = bytes;
            Ok(response)
        }
        Err(error) => match transaction_rejection(error) {
            Ok(reason) => {
                let response = rejected(sequence, reason);
                next.clients.insert(
                    command.client_id,
                    ClientResult {
                        sequence,
                        fingerprint,
                        response: response.clone(),
                    },
                );
                *state_bytes = save_state(catalog, state_bytes, &next)?;
                *state = next;
                Ok(response)
            }
            Err(error) => Err(error),
        },
    }
}

fn rejected(sequence: u64, reason: RaftRejection) -> RaftResponse {
    RaftResponse {
        sequence,
        result: RaftResponseResult::Rejected { reason },
    }
}

use super::TypeConfig;
use super::state_machine::{
    RAFT_SNAPSHOT_SIDECAR_NAME, RAFT_STATE_SIDECAR_NAME, RaftApplicationState, SharedCatalog,
    lock_catalog, run_blocking,
};
use crate::catalog::Catalog;
use crate::replication::MAX_REPLICATION_SNAPSHOT_BYTES;
use openraft::{
    BasicNode, ErrorSubject, ErrorVerb, RaftSnapshotBuilder, Snapshot, SnapshotMeta, StorageError,
};
use sha2::{Digest, Sha256};
use std::io::Cursor;

const SNAPSHOT_MAGIC: &[u8; 4] = b"TXRF";
const SNAPSHOT_VERSION: u8 = 1;
const SNAPSHOT_HEADER_BYTES: usize = 13;
const NO_CATALOG_SNAPSHOT: u32 = u32::MAX;

pub struct CatalogSnapshotBuilder {
    pub(super) catalog: SharedCatalog,
}

struct SnapshotContents {
    state: RaftApplicationState,
    state_bytes: Vec<u8>,
    catalog_bytes: Option<Vec<u8>>,
}

impl CatalogSnapshotBuilder {
    fn build(&self) -> Result<Snapshot<TypeConfig>, String> {
        let catalog = lock_catalog(&self.catalog)?;
        let (transaction_id, catalog_bytes, sidecars) = catalog
            .export_snapshot_with_sidecars(&[RAFT_STATE_SIDECAR_NAME, RAFT_SNAPSHOT_SIDECAR_NAME])
            .map_err(|error| error.to_string())?;
        let state_bytes = sidecars[0]
            .as_deref()
            .ok_or_else(|| "Raft application state sidecar is missing".to_owned())?;
        let state = RaftApplicationState::from_bytes(state_bytes)?;
        if state.catalog_transaction_id != transaction_id {
            return Err("Raft application state does not match the catalog position".into());
        }
        let bytes = encode_snapshot(state_bytes, catalog_bytes.as_deref())?;
        catalog
            .update_sidecars_at_transaction(
                Some(transaction_id),
                vec![(
                    RAFT_SNAPSHOT_SIDECAR_NAME.into(),
                    sidecars[1].clone(),
                    Some(bytes.clone()),
                )],
            )
            .map_err(|error| error.to_string())?;
        Ok(snapshot_from_bytes(bytes)?)
    }
}

impl RaftSnapshotBuilder<TypeConfig> for CatalogSnapshotBuilder {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<u64>> {
        let catalog = self.catalog.clone();
        run_blocking(ErrorSubject::StateMachine, ErrorVerb::Read, move || {
            Self { catalog }.build()
        })
        .await
    }
}

pub(super) fn current_snapshot(catalog: &Catalog) -> Result<Option<Snapshot<TypeConfig>>, String> {
    let (transaction_id, _, sidecars) = catalog
        .export_snapshot_with_sidecars(&[RAFT_STATE_SIDECAR_NAME, RAFT_SNAPSHOT_SIDECAR_NAME])
        .map_err(|error| error.to_string())?;
    let current_state_bytes = sidecars[0]
        .as_deref()
        .ok_or_else(|| "Raft application state sidecar is missing".to_owned())?;
    let current_state = RaftApplicationState::from_bytes(current_state_bytes)?;
    if current_state.catalog_transaction_id != transaction_id {
        return Err("Raft application state does not match the catalog position".into());
    }
    let Some(snapshot_bytes) = &sidecars[1] else {
        return Ok(None);
    };
    let snapshot = decode_snapshot(snapshot_bytes)?;
    if snapshot.state.catalog_transaction_id > current_state.catalog_transaction_id
        || snapshot.state.last_applied.map(|id| id.index)
            > current_state.last_applied.map(|id| id.index)
    {
        return Err("stored Raft snapshot is newer than the application state".into());
    }
    if snapshot.state.last_applied == current_state.last_applied
        && snapshot.state_bytes != current_state_bytes
    {
        return Err("stored Raft snapshot disagrees with the application state".into());
    }
    Ok(Some(snapshot_from_bytes(snapshot_bytes.clone())?))
}

pub(super) fn install_snapshot(
    catalog: &mut Catalog,
    meta: &SnapshotMeta<u64, BasicNode>,
    bytes: &[u8],
) -> Result<(), String> {
    let contents = decode_snapshot(bytes)?;
    if contents.state.last_applied != meta.last_log_id
        || contents.state.last_membership != meta.last_membership
        || snapshot_id(bytes) != meta.snapshot_id
    {
        return Err("Raft snapshot metadata does not match its payload".into());
    }
    let (current_transaction_id, current_catalog_bytes, sidecars) = catalog
        .export_snapshot_with_sidecars(&[RAFT_STATE_SIDECAR_NAME, RAFT_SNAPSHOT_SIDECAR_NAME])
        .map_err(|error| error.to_string())?;
    let current_state_bytes = sidecars[0]
        .as_deref()
        .ok_or_else(|| "Raft application state sidecar is missing".to_owned())?;
    let current_state = RaftApplicationState::from_bytes(current_state_bytes)?;
    if current_state.catalog_transaction_id != current_transaction_id {
        return Err("Raft application state does not match the catalog position".into());
    }
    if contents.state.catalog_transaction_id < current_transaction_id {
        return Err("Raft snapshot catalog position is older than the local catalog".into());
    }
    match (current_state.last_applied, contents.state.last_applied) {
        (Some(current), Some(incoming)) if incoming.index < current.index => {
            return Err("Raft snapshot log position is older than the local state machine".into());
        }
        (Some(current), Some(incoming)) if incoming.index == current.index => {
            if incoming != current
                || contents.state_bytes != current_state_bytes
                || contents.catalog_bytes != current_catalog_bytes
            {
                return Err("Raft snapshot conflicts with the local applied position".into());
            }
        }
        (Some(_), None) => {
            return Err("Raft snapshot has no applied position but local state does".into());
        }
        _ => {}
    }

    let state_after = Some(contents.state_bytes);
    let snapshot_after = Some(bytes.to_vec());
    if contents.state.catalog_transaction_id > current_transaction_id {
        let catalog_snapshot = contents
            .catalog_bytes
            .as_deref()
            .ok_or_else(|| "Raft snapshot is missing its catalog image".to_owned())?;
        catalog
            .install_snapshot_with_sidecars(
                catalog_snapshot,
                current_transaction_id,
                vec![
                    (RAFT_STATE_SIDECAR_NAME, sidecars[0].clone(), state_after),
                    (
                        RAFT_SNAPSHOT_SIDECAR_NAME,
                        sidecars[1].clone(),
                        snapshot_after,
                    ),
                ],
            )
            .map_err(|error| error.to_string())?;
    } else {
        if contents.catalog_bytes != current_catalog_bytes {
            return Err("Raft snapshot catalog image conflicts with the local catalog".into());
        }
        catalog
            .update_sidecars_at_transaction(
                Some(current_transaction_id),
                vec![
                    (
                        RAFT_STATE_SIDECAR_NAME.into(),
                        sidecars[0].clone(),
                        state_after,
                    ),
                    (
                        RAFT_SNAPSHOT_SIDECAR_NAME.into(),
                        sidecars[1].clone(),
                        snapshot_after,
                    ),
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn encode_snapshot(state_bytes: &[u8], catalog_bytes: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let state_length = u32::try_from(state_bytes.len())
        .map_err(|_| "Raft application state is too large for a snapshot".to_owned())?;
    let catalog_length = catalog_bytes
        .map(|bytes| {
            u32::try_from(bytes.len())
                .map_err(|_| "Raft catalog image is too large for a snapshot".to_owned())
        })
        .transpose()?
        .unwrap_or(NO_CATALOG_SNAPSHOT);
    let payload_length = SNAPSHOT_HEADER_BYTES
        .checked_add(state_bytes.len())
        .and_then(|length| length.checked_add(catalog_bytes.map_or(0, <[u8]>::len)))
        .ok_or_else(|| "Raft snapshot length overflow".to_owned())?;
    if payload_length > MAX_REPLICATION_SNAPSHOT_BYTES {
        return Err(format!(
            "Raft snapshot exceeds {MAX_REPLICATION_SNAPSHOT_BYTES} bytes"
        ));
    }
    let mut bytes = Vec::with_capacity(payload_length);
    bytes.extend_from_slice(SNAPSHOT_MAGIC);
    bytes.push(SNAPSHOT_VERSION);
    bytes.extend_from_slice(&state_length.to_be_bytes());
    bytes.extend_from_slice(&catalog_length.to_be_bytes());
    bytes.extend_from_slice(state_bytes);
    if let Some(catalog_bytes) = catalog_bytes {
        bytes.extend_from_slice(catalog_bytes);
    }
    Ok(bytes)
}

fn decode_snapshot(bytes: &[u8]) -> Result<SnapshotContents, String> {
    if bytes.len() > MAX_REPLICATION_SNAPSHOT_BYTES || bytes.len() < SNAPSHOT_HEADER_BYTES {
        return Err("Raft snapshot size is invalid".into());
    }
    if &bytes[..4] != SNAPSHOT_MAGIC || bytes[4] != SNAPSHOT_VERSION {
        return Err("Raft snapshot header or version is invalid".into());
    }
    let state_length = u32::from_be_bytes(bytes[5..9].try_into().unwrap()) as usize;
    let catalog_length = u32::from_be_bytes(bytes[9..13].try_into().unwrap());
    let state_end = SNAPSHOT_HEADER_BYTES
        .checked_add(state_length)
        .ok_or_else(|| "Raft snapshot state length overflow".to_owned())?;
    if state_end > bytes.len() {
        return Err("Raft snapshot state is truncated".into());
    }
    let catalog_bytes = if catalog_length == NO_CATALOG_SNAPSHOT {
        if state_end != bytes.len() {
            return Err("Raft snapshot has unexpected trailing bytes".into());
        }
        None
    } else {
        let end = state_end
            .checked_add(catalog_length as usize)
            .ok_or_else(|| "Raft snapshot catalog length overflow".to_owned())?;
        if end != bytes.len() {
            return Err("Raft snapshot catalog length does not match payload".into());
        }
        Some(bytes[state_end..end].to_vec())
    };
    let state_bytes = bytes[SNAPSHOT_HEADER_BYTES..state_end].to_vec();
    let state = RaftApplicationState::from_bytes(&state_bytes)?;
    match (&catalog_bytes, state.catalog_transaction_id) {
        (None, 0) => {}
        (Some(_), 0) => return Err("empty Raft snapshot has an unexpected catalog image".into()),
        (Some(bytes), expected) => {
            let (actual, _) =
                Catalog::snapshot_metadata(bytes).map_err(|error| error.to_string())?;
            if actual != expected {
                return Err("Raft snapshot catalog position does not match its state".into());
            }
        }
        (None, _) => return Err("Raft snapshot is missing its catalog image".into()),
    }
    Ok(SnapshotContents {
        state,
        state_bytes,
        catalog_bytes,
    })
}

fn snapshot_id(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn snapshot_from_bytes(bytes: Vec<u8>) -> Result<Snapshot<TypeConfig>, String> {
    let contents = decode_snapshot(&bytes)?;
    let meta = SnapshotMeta {
        last_log_id: contents.state.last_applied,
        last_membership: contents.state.last_membership,
        snapshot_id: snapshot_id(&bytes),
    };
    Ok(Snapshot {
        meta,
        snapshot: Box::new(Cursor::new(bytes)),
    })
}

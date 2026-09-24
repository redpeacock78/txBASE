use super::{
    CatalogReplicationRole, HttpResponse, error, header, json_bytes_response, json_response,
    read_json_body, read_json_body_with_limit, request_header,
};
use crate::catalog::{Catalog, CatalogTransactionError};
use crate::replication::{
    ApplyOutcome, MAX_REPLICATION_SNAPSHOT_BYTES, REPLICATION_TRANSPORT_VERSION, ReplicationEntry,
    ReplicationError, ReplicationLog, ReplicationSnapshot,
};
use serde_json::json;
use tiny_http::{Method, Request};

const STATUS_PATH: &str = "/replication/status";
const SNAPSHOT_PATH: &str = "/replication/snapshot";
const ENTRY_PATH: &str = "/replication/entry";

#[cfg(test)]
pub(super) fn response(
    request: &mut Request,
    path: &str,
    catalog: &mut Catalog,
    log: &mut ReplicationLog,
    role: CatalogReplicationRole,
) -> Option<HttpResponse> {
    response_with_auth(request, path, catalog, log, role, None)
}

pub(super) fn response_with_auth(
    request: &mut Request,
    path: &str,
    catalog: &mut Catalog,
    log: &mut ReplicationLog,
    role: CatalogReplicationRole,
    replication_token: Option<&str>,
) -> Option<HttpResponse> {
    if !path.starts_with("/replication/") {
        return None;
    }
    if let Some(expected_token) = replication_token {
        if let Err(response) = authorize(request, expected_token) {
            return Some(response);
        }
    }

    Some(match (request.method(), path) {
        (Method::Get | Method::Head, STATUS_PATH) => status(catalog, log, role),
        (Method::Get | Method::Head, SNAPSHOT_PATH) => snapshot(catalog, log),
        (Method::Post, ENTRY_PATH) => receive_entry(request, catalog, log),
        (Method::Post, SNAPSHOT_PATH) => install_snapshot(request, catalog, log),
        _ => json_response(
            405,
            error(
                "method_not_allowed",
                "replication supports GET or HEAD for status and snapshots, and POST for entries or snapshots",
            ),
            false,
        )
        .with_header(header("Allow", "GET, HEAD, POST")),
    })
}

fn authorize(request: &Request, expected_token: &str) -> Result<(), HttpResponse> {
    let Some(value) = request_header(request, "Authorization") else {
        return Err(unauthorized(
            "replication_auth_required",
            "replication routes require Authorization: Bearer <token>",
        ));
    };
    let Some(token) = bearer_token(value) else {
        return Err(unauthorized(
            "replication_invalid_token",
            "replication bearer token is invalid",
        ));
    };
    if !constant_time_eq(token.as_bytes(), expected_token.as_bytes()) {
        return Err(unauthorized(
            "replication_invalid_token",
            "replication bearer token is invalid",
        ));
    }
    Ok(())
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim_start_matches(' ');
    if scheme.eq_ignore_ascii_case("Bearer") && is_valid_bearer_token(token) {
        Some(token)
    } else {
        None
    }
}

pub(super) fn is_valid_bearer_token(token: &str) -> bool {
    let mut has_token_byte = false;
    let mut padding = false;
    for byte in token.bytes() {
        match byte {
            b'=' => padding = true,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'+' | b'/'
                if !padding =>
            {
                has_token_byte = true
            }
            _ => return false,
        }
    }
    has_token_byte
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

fn unauthorized(code: &str, message: &str) -> HttpResponse {
    json_response(401, error(code, message), false)
        .with_header(header("WWW-Authenticate", "Bearer"))
}

fn status(catalog: &Catalog, log: &ReplicationLog, role: CatalogReplicationRole) -> HttpResponse {
    let (_, schema_tag) = match catalog.schema_representation() {
        Ok(value) => value,
        Err(error) => {
            return json_response(
                500,
                super::error("catalog_error", &error.to_string()),
                false,
            );
        }
    };
    json_response(
        200,
        json!({
            "transport_version": REPLICATION_TRANSPORT_VERSION,
            "role": match role {
                CatalogReplicationRole::Authority => "authority",
                CatalogReplicationRole::Follower => "follower",
            },
            "term": log.term(),
            "base_index": log.base_index(),
            "base_transaction_id": log.base_transaction_id(),
            "last_index": log.last_index(),
            "last_transaction_id": log.last_transaction_id(),
            "schema_tag": schema_tag,
        }),
        false,
    )
}

pub(super) fn follower_read_only_response() -> HttpResponse {
    json_response(
        409,
        error(
            "replication_follower_read_only",
            "catalog mutations require the authority role; deliver a replication entry instead",
        ),
        false,
    )
}

fn snapshot(catalog: &Catalog, log: &ReplicationLog) -> HttpResponse {
    match log
        .snapshot(catalog)
        .and_then(|snapshot| snapshot.to_json())
    {
        Ok(body) => json_bytes_response(200, body, false),
        Err(error) => replication_error_response(error),
    }
}

fn receive_entry(
    request: &mut Request,
    catalog: &Catalog,
    log: &mut ReplicationLog,
) -> HttpResponse {
    let body = match read_json_body(request, "POST /replication/entry", false) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let entry = match ReplicationEntry::from_json(&body) {
        Ok(entry) => entry,
        Err(error) => return replication_error_response(error),
    };
    match log.receive(catalog, entry) {
        Ok(outcome) => apply_outcome_response(outcome),
        Err(error) => replication_error_response(error),
    }
}

fn install_snapshot(
    request: &mut Request,
    catalog: &mut Catalog,
    log: &mut ReplicationLog,
) -> HttpResponse {
    let body = match read_json_body_with_limit(
        request,
        "POST /replication/snapshot",
        false,
        MAX_REPLICATION_SNAPSHOT_BYTES,
    ) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let snapshot = match ReplicationSnapshot::from_json(&body) {
        Ok(snapshot) => snapshot,
        Err(error) => return replication_error_response(error),
    };
    match log.install_snapshot(catalog, snapshot) {
        Ok(outcome) => apply_outcome_response(outcome),
        Err(error) => replication_error_response(error),
    }
}

fn apply_outcome_response(outcome: ApplyOutcome) -> HttpResponse {
    let (outcome, index, transaction_id) = match outcome {
        ApplyOutcome::Applied {
            index,
            transaction_id,
        } => ("applied", index, transaction_id),
        ApplyOutcome::Duplicate {
            index,
            transaction_id,
        } => ("duplicate", index, transaction_id),
        ApplyOutcome::SnapshotInstalled {
            index,
            transaction_id,
        } => ("snapshot_installed", index, transaction_id),
        ApplyOutcome::SnapshotDuplicate {
            index,
            transaction_id,
        } => ("snapshot_duplicate", index, transaction_id),
    };
    json_response(
        200,
        json!({
            "transport_version": REPLICATION_TRANSPORT_VERSION,
            "outcome": outcome,
            "index": index,
            "transaction_id": transaction_id,
        }),
        false,
    )
}

pub(super) fn replication_error_response(error: ReplicationError) -> HttpResponse {
    let (status, tag) = match &error {
        ReplicationError::Invalid(_) | ReplicationError::Serialization(_) => (422, None),
        ReplicationError::Catalog(_) => (500, None),
        ReplicationError::Commit(CatalogTransactionError::PreconditionFailed { tag }) => {
            (412, Some(tag.as_str()))
        }
        ReplicationError::Commit(CatalogTransactionError::Invalid(_)) => (422, None),
        ReplicationError::Commit(
            CatalogTransactionError::SidecarPreconditionFailed { .. }
            | CatalogTransactionError::TransactionPreconditionFailed { .. }
            | CatalogTransactionError::TableSetChanged,
        ) => (409, None),
        ReplicationError::Commit(CatalogTransactionError::Catalog(_)) => (500, None),
        _ => (409, None),
    };
    let response = json_response(status, error_json(&error), false);
    if let Some(tag) = tag {
        super::etag::with_tag(response, tag)
    } else {
        response
    }
}

fn error_json(replication_error: &ReplicationError) -> serde_json::Value {
    let code = match replication_error {
        ReplicationError::Invalid(_) | ReplicationError::Serialization(_) => "invalid_replication",
        ReplicationError::Catalog(_) => "replication_storage",
        ReplicationError::Commit(CatalogTransactionError::PreconditionFailed { .. }) => {
            "precondition_failed"
        }
        ReplicationError::Commit(CatalogTransactionError::Invalid(_)) => "invalid_transaction",
        ReplicationError::Commit(
            CatalogTransactionError::SidecarPreconditionFailed { .. }
            | CatalogTransactionError::TransactionPreconditionFailed { .. }
            | CatalogTransactionError::TableSetChanged,
        ) => "catalog_changed",
        ReplicationError::Commit(CatalogTransactionError::Catalog(_)) => "replication_storage",
        ReplicationError::TermMismatch { .. } => "replication_term_mismatch",
        ReplicationError::IndexGap { .. } => "replication_index_gap",
        ReplicationError::TransactionGap { .. } => "replication_transaction_gap",
        ReplicationError::CatalogStateMismatch { .. } => "replication_catalog_state_mismatch",
        ReplicationError::SchemaMismatch { .. } => "replication_schema_mismatch",
        ReplicationError::HistoryUnavailable { .. } => "replication_history_unavailable",
        ReplicationError::ConflictingDuplicate { .. } => "replication_conflicting_duplicate",
        ReplicationError::SidecarStateMismatch => "replication_sidecar_mismatch",
        ReplicationError::ReadUnavailable { .. }
        | ReplicationError::ReadHistoryUnavailable { .. } => "replication_read_unavailable",
        ReplicationError::SnapshotStale { .. } => "replication_snapshot_stale",
        ReplicationError::SnapshotConflict { .. } => "replication_snapshot_conflict",
        ReplicationError::SnapshotInstallRace { .. } => "replication_snapshot_install_race",
    };
    error(code, &replication_error.to_string())
}

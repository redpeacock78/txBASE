use super::super::RaftRuntime;
use crate::replication::raft::{self, MAX_RAFT_RPC_BYTES};
use crate::server::{HttpResponse, error, json_response, read_json_body_with_limit};
use openraft::BasicNode;
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tiny_http::Request;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddLearnerRequest {
    version: u16,
    cluster_id: String,
    node_id: u64,
    peer_address: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeMembershipRequest {
    version: u16,
    cluster_id: String,
    expected_membership_log_index: u64,
    expected_voter_ids: Vec<u64>,
    voter_ids: Vec<u64>,
}

impl RaftRuntime {
    pub(super) fn membership_status_response(&self) -> HttpResponse {
        json_response(200, self.membership_status(), false)
    }

    fn membership_status(&self) -> serde_json::Value {
        let metrics = self.node.metrics();
        let metrics = metrics.borrow();
        let membership = &metrics.membership_config;
        let voter_ids = membership.voter_ids().collect::<BTreeSet<_>>();
        let learner_ids = membership
            .membership()
            .learner_ids()
            .collect::<BTreeSet<_>>();
        let voter_configs = membership
            .membership()
            .get_joint_config()
            .iter()
            .map(|config| config.iter().copied().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let nodes = membership
            .nodes()
            .map(|(node_id, node)| {
                json!({
                    "node_id": node_id,
                    "peer_address": node.addr.clone(),
                    "role": if voter_ids.contains(node_id) { "voter" } else { "learner" }
                })
            })
            .collect::<Vec<_>>();

        json!({
            "version": raft::RAFT_RPC_VERSION,
            "cluster_id": self.cluster_id,
            "node_id": self.node_id,
            "server_state": format!("{:?}", metrics.state).to_ascii_lowercase(),
            "leader_id": metrics.current_leader,
            "effective_membership_log_index": membership.log_id().as_ref().map(|id| id.index),
            "effective_voter_configs": voter_configs,
            "voter_ids": voter_ids,
            "learner_ids": learner_ids,
            "membership_change_in_progress": self.membership_change_lock.try_lock().is_err(),
            "nodes": nodes
        })
    }

    pub(super) fn handle_add_learner(&self, request: &mut Request) -> HttpResponse {
        let bytes = match read_json_body_with_limit(
            request,
            raft::RAFT_ADD_LEARNER_PATH,
            false,
            MAX_RAFT_RPC_BYTES,
        ) {
            Ok(bytes) => bytes,
            Err(response) => return response,
        };
        let rpc: AddLearnerRequest = match serde_json::from_slice(&bytes) {
            Ok(rpc) => rpc,
            Err(parse_error) => {
                return json_response(
                    400,
                    error("invalid_raft_learner_request", &parse_error.to_string()),
                    false,
                );
            }
        };
        if rpc.version != raft::RAFT_RPC_VERSION {
            return json_response(
                400,
                error(
                    "unsupported_raft_rpc_version",
                    "Raft request version is unsupported",
                ),
                false,
            );
        }
        if rpc.cluster_id != self.cluster_id {
            return json_response(
                403,
                error(
                    "raft_cluster_mismatch",
                    "Raft request targets another cluster",
                ),
                false,
            );
        }
        if rpc.node_id == 0 || rpc.node_id == self.node_id {
            return json_response(
                400,
                error(
                    "invalid_raft_learner_id",
                    "learner node ID must be positive and different from this node",
                ),
                false,
            );
        }
        match self.add_learner(rpc.node_id, &rpc.peer_address) {
            Ok(added) => json_response(
                if added { 202 } else { 200 },
                json!({
                    "node_id": rpc.node_id,
                    "status": if added { "learner_sync_started" } else { "already_member" }
                }),
                false,
            ),
            Err((status, message)) => {
                json_response(status, error("raft_learner_add_failed", &message), false)
            }
        }
    }

    pub(super) fn handle_change_membership(&self, request: &mut Request) -> HttpResponse {
        let bytes = match read_json_body_with_limit(
            request,
            raft::RAFT_MEMBERSHIP_PATH,
            false,
            MAX_RAFT_RPC_BYTES,
        ) {
            Ok(bytes) => bytes,
            Err(response) => return response,
        };
        let rpc: ChangeMembershipRequest = match serde_json::from_slice(&bytes) {
            Ok(rpc) => rpc,
            Err(parse_error) => {
                return json_response(
                    400,
                    error("invalid_raft_membership_request", &parse_error.to_string()),
                    false,
                );
            }
        };
        if rpc.version != raft::RAFT_RPC_VERSION {
            return json_response(
                400,
                error(
                    "unsupported_raft_rpc_version",
                    "Raft request version is unsupported",
                ),
                false,
            );
        }
        if rpc.cluster_id != self.cluster_id {
            return json_response(
                403,
                error(
                    "raft_cluster_mismatch",
                    "Raft request targets another cluster",
                ),
                false,
            );
        }
        let expected_voter_ids = match voter_ids(rpc.expected_voter_ids, "expected_voter_ids") {
            Ok(voter_ids) => voter_ids,
            Err(message) => {
                return json_response(
                    400,
                    error("invalid_raft_membership_request", &message),
                    false,
                );
            }
        };
        let requested_voter_ids = match voter_ids(rpc.voter_ids, "voter_ids") {
            Ok(voter_ids) => voter_ids,
            Err(message) => {
                return json_response(
                    400,
                    error("invalid_raft_membership_request", &message),
                    false,
                );
            }
        };
        match self.change_voters(
            rpc.expected_membership_log_index,
            expected_voter_ids,
            requested_voter_ids.clone(),
        ) {
            Ok(status) => {
                let status_code = if status == "already_current" {
                    200
                } else {
                    202
                };
                let current = self.membership_status();
                json_response(
                    status_code,
                    json!({
                        "status": status,
                        "requested_voter_ids": requested_voter_ids,
                        "effective_membership_log_index": current["effective_membership_log_index"],
                        "effective_voter_configs": current["effective_voter_configs"]
                    }),
                    false,
                )
            }
            Err((status, message)) => json_response(
                status,
                error("raft_membership_change_failed", &message),
                false,
            ),
        }
    }

    fn change_voters(
        &self,
        expected_membership_log_index: u64,
        expected_voter_ids: BTreeSet<u64>,
        requested_voter_ids: BTreeSet<u64>,
    ) -> Result<&'static str, (u16, String)> {
        let (
            voter_configs,
            current_voter_ids,
            known_nodes,
            membership_log_index,
            is_leader,
            leader_id,
        ) = {
            let metrics = self.node.metrics();
            let metrics = metrics.borrow();
            let membership = &metrics.membership_config;
            (
                membership.membership().get_joint_config().clone(),
                membership.voter_ids().collect::<BTreeSet<_>>(),
                membership
                    .nodes()
                    .map(|(node_id, node)| (*node_id, node.clone()))
                    .collect::<BTreeMap<_, _>>(),
                membership.log_id().as_ref().map(|id| id.index),
                metrics.state.is_leader(),
                metrics.current_leader,
            )
        };

        if requested_voter_ids.is_empty() {
            return Err((400, "the voter set must not be empty".into()));
        }
        if requested_voter_ids.iter().any(|id| *id == 0) {
            return Err((400, "voter IDs must be positive".into()));
        }
        if membership_log_index.is_none() || voter_configs.is_empty() {
            return Err((409, "Raft membership is not initialized".into()));
        }
        if requested_voter_ids
            .iter()
            .any(|node_id| !known_nodes.contains_key(node_id))
        {
            return Err((
                409,
                "every requested voter must already be a member or learner".into(),
            ));
        }

        if voter_configs.len() > 1 {
            if voter_configs.last() != Some(&requested_voter_ids) {
                return Err((
                    409,
                    "another joint membership change is already effective".into(),
                ));
            }
            if !is_leader {
                return Err((
                    409,
                    format!("membership changes must be sent to leader {leader_id:?}"),
                ));
            }
            return Ok(
                match self.schedule_membership_change(requested_voter_ids, Vec::new()) {
                    Ok(()) => "membership_change_resumed",
                    Err(()) => "membership_change_in_progress",
                },
            );
        }

        if !is_leader {
            return Err((
                409,
                format!("membership changes must be sent to leader {leader_id:?}"),
            ));
        }
        if current_voter_ids == requested_voter_ids {
            return Ok("already_current");
        }
        if membership_log_index != Some(expected_membership_log_index)
            || current_voter_ids != expected_voter_ids
        {
            return Err((
                409,
                format!(
                    "stale membership: current effective index is {membership_log_index:?} with voters {current_voter_ids:?}"
                ),
            ));
        }
        let learners = requested_voter_ids
            .difference(&current_voter_ids)
            .map(|node_id| (*node_id, known_nodes[node_id].clone()))
            .collect();
        if self
            .schedule_membership_change(requested_voter_ids, learners)
            .is_err()
        {
            return Err((409, "another membership change is being submitted".into()));
        }
        Ok("membership_change_started")
    }

    fn schedule_membership_change(
        &self,
        voter_ids: BTreeSet<u64>,
        learners: Vec<(u64, BasicNode)>,
    ) -> Result<(), ()> {
        let guard = self
            .membership_change_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| ())?;
        let node = Arc::clone(&self.node);
        let node_id = self.node_id;
        self.runtime.spawn(async move {
            let _guard = guard;
            for (learner_id, learner) in learners {
                if let Err(error) = node.add_learner(learner_id, learner, true).await {
                    eprintln!(
                        "Raft learner {learner_id} did not catch up on node {node_id}: {error}"
                    );
                    return;
                }
            }
            if let Err(error) = node.change_membership(voter_ids, true).await {
                eprintln!("Raft membership change failed on node {node_id}: {error}");
            }
        });
        Ok(())
    }
}

fn voter_ids(ids: Vec<u64>, field: &str) -> Result<BTreeSet<u64>, String> {
    if ids.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    let count = ids.len();
    let ids = ids.into_iter().collect::<BTreeSet<_>>();
    if ids.contains(&0) {
        return Err(format!("{field} must contain only positive node IDs"));
    }
    if ids.len() != count {
        return Err(format!("{field} must not contain duplicate node IDs"));
    }
    Ok(ids)
}

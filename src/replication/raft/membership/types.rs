use super::super::network::is_valid_cluster_id;
use crate::replication::{ReplicationError, ReplicationHttpError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaftAddLearnerRequest {
    pub version: u16,
    pub cluster_id: String,
    pub node_id: u64,
    pub peer_address: String,
}

impl RaftAddLearnerRequest {
    pub fn new(
        cluster_id: impl Into<String>,
        node_id: u64,
        peer_address: impl Into<String>,
    ) -> Result<Self, ReplicationHttpError> {
        let request = Self {
            version: super::super::RAFT_RPC_VERSION,
            cluster_id: cluster_id.into(),
            node_id,
            peer_address: peer_address.into(),
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), ReplicationHttpError> {
        if self.version != super::super::RAFT_RPC_VERSION {
            return Err(invalid_request("unsupported Raft membership version"));
        }
        if !is_valid_cluster_id(&self.cluster_id) {
            return Err(invalid_request("Raft cluster ID is invalid"));
        }
        if self.node_id == 0 {
            return Err(invalid_request("learner node ID must be positive"));
        }
        if self.peer_address.trim().is_empty() || self.peer_address.contains(['\r', '\n', '\0']) {
            return Err(invalid_request("learner peer address is invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaftLearnerAddStatus {
    LearnerSyncStarted,
    AlreadyMember,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaftLearnerAddResponse {
    pub node_id: u64,
    pub status: RaftLearnerAddStatus,
}

impl RaftLearnerAddResponse {
    pub(super) fn validate(&self, requested_node_id: u64) -> Result<(), ReplicationHttpError> {
        if self.node_id != requested_node_id || self.node_id == 0 {
            return Err(invalid_response(
                "learner response has an unexpected node ID",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaftMembershipChangeRequest {
    pub version: u16,
    pub cluster_id: String,
    pub expected_membership_log_index: u64,
    pub expected_voter_ids: Vec<u64>,
    pub voter_ids: Vec<u64>,
}

impl RaftMembershipChangeRequest {
    pub fn new(
        cluster_id: impl Into<String>,
        expected_membership_log_index: u64,
        expected_voter_ids: Vec<u64>,
        voter_ids: Vec<u64>,
    ) -> Result<Self, ReplicationHttpError> {
        let request = Self {
            version: super::super::RAFT_RPC_VERSION,
            cluster_id: cluster_id.into(),
            expected_membership_log_index,
            expected_voter_ids,
            voter_ids,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), ReplicationHttpError> {
        if self.version != super::super::RAFT_RPC_VERSION {
            return Err(invalid_request("unsupported Raft membership version"));
        }
        if !is_valid_cluster_id(&self.cluster_id) {
            return Err(invalid_request("Raft cluster ID is invalid"));
        }
        validate_ids(&self.expected_voter_ids, "expected voter IDs", true)
            .map_err(invalid_request)?;
        validate_ids(&self.voter_ids, "voter IDs", true).map_err(invalid_request)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaftMembershipChangeStatus {
    AlreadyCurrent,
    MembershipChangeStarted,
    MembershipChangeResumed,
    MembershipChangeInProgress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaftMembershipChangeResponse {
    pub status: RaftMembershipChangeStatus,
    pub requested_voter_ids: Vec<u64>,
    pub effective_membership_log_index: Option<u64>,
    pub effective_voter_configs: Vec<Vec<u64>>,
}

impl RaftMembershipChangeResponse {
    pub(super) fn validate(
        &self,
        request: &RaftMembershipChangeRequest,
    ) -> Result<(), ReplicationHttpError> {
        validate_ids(&self.requested_voter_ids, "response voter IDs", true)
            .map_err(invalid_response)?;
        if id_set(&self.requested_voter_ids) != id_set(&request.voter_ids) {
            return Err(invalid_response(
                "membership response does not match the requested voter set",
            ));
        }
        validate_voter_configs(&self.effective_voter_configs).map_err(invalid_response)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaftNodeRole {
    Voter,
    Learner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaftMembershipNode {
    pub node_id: u64,
    pub peer_address: String,
    pub role: RaftNodeRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaftMembershipStatus {
    pub version: u16,
    pub cluster_id: String,
    pub node_id: u64,
    pub server_state: String,
    pub leader_id: Option<u64>,
    pub effective_membership_log_index: Option<u64>,
    pub effective_voter_configs: Vec<Vec<u64>>,
    pub voter_ids: Vec<u64>,
    pub learner_ids: Vec<u64>,
    pub membership_change_in_progress: bool,
    pub nodes: Vec<RaftMembershipNode>,
}

impl RaftMembershipStatus {
    pub fn validate(&self) -> Result<(), ReplicationHttpError> {
        if self.version != super::super::RAFT_RPC_VERSION {
            return Err(invalid_response(
                "unsupported Raft membership response version",
            ));
        }
        if !is_valid_cluster_id(&self.cluster_id) || self.node_id == 0 {
            return Err(invalid_response("Raft membership identity is invalid"));
        }
        if self.server_state.trim().is_empty() || self.server_state.contains(['\r', '\n']) {
            return Err(invalid_response("Raft server state is invalid"));
        }
        if self.leader_id == Some(0) {
            return Err(invalid_response("Raft leader ID must be positive"));
        }
        validate_ids(&self.voter_ids, "voter IDs", false).map_err(invalid_response)?;
        validate_ids(&self.learner_ids, "learner IDs", false).map_err(invalid_response)?;
        validate_voter_configs(&self.effective_voter_configs).map_err(invalid_response)?;

        let voters = id_set(&self.voter_ids);
        let learners = id_set(&self.learner_ids);
        if !voters.is_disjoint(&learners) {
            return Err(invalid_response("Raft voters and learners overlap"));
        }
        let configured_voters = self
            .effective_voter_configs
            .iter()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        if configured_voters != voters {
            return Err(invalid_response(
                "effective voter configurations do not match voter IDs",
            ));
        }

        let mut node_ids = BTreeSet::new();
        for node in &self.nodes {
            if node.node_id == 0
                || node.peer_address.trim().is_empty()
                || node.peer_address.contains(['\r', '\n', '\0'])
                || !node_ids.insert(node.node_id)
            {
                return Err(invalid_response("Raft node metadata is invalid"));
            }
            let voter = voters.contains(&node.node_id);
            if (node.role == RaftNodeRole::Voter) != voter
                || (!voter && !learners.contains(&node.node_id))
            {
                return Err(invalid_response(
                    "Raft node roles do not match effective membership",
                ));
            }
        }
        if node_ids != voters.union(&learners).copied().collect() {
            return Err(invalid_response(
                "Raft node metadata does not cover effective membership",
            ));
        }
        Ok(())
    }
}

fn validate_voter_configs(configs: &[Vec<u64>]) -> Result<(), String> {
    for config in configs {
        validate_ids(config, "effective voter configuration", true)?;
    }
    Ok(())
}

fn validate_ids(ids: &[u64], label: &str, required: bool) -> Result<(), String> {
    if required && ids.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if ids.contains(&0) {
        return Err(format!("{label} must contain only positive IDs"));
    }
    if id_set(ids).len() != ids.len() {
        return Err(format!("{label} must not contain duplicate IDs"));
    }
    Ok(())
}

fn id_set(ids: &[u64]) -> BTreeSet<u64> {
    ids.iter().copied().collect()
}

pub(super) fn invalid_request(message: impl Into<String>) -> ReplicationHttpError {
    ReplicationError::Invalid(message.into()).into()
}

pub(super) fn invalid_response(message: impl Into<String>) -> ReplicationHttpError {
    ReplicationHttpError::InvalidResponse(message.into())
}

#[cfg(test)]
mod tests {
    use super::{
        RaftMembershipChangeRequest, RaftMembershipChangeResponse, RaftMembershipChangeStatus,
        RaftMembershipNode, RaftMembershipStatus, RaftNodeRole,
    };

    fn status() -> RaftMembershipStatus {
        RaftMembershipStatus {
            version: super::super::super::RAFT_RPC_VERSION,
            cluster_id: "cluster-1".into(),
            node_id: 1,
            server_state: "leader".into(),
            leader_id: Some(1),
            effective_membership_log_index: Some(4),
            effective_voter_configs: vec![vec![1, 2]],
            voter_ids: vec![1, 2],
            learner_ids: vec![3],
            membership_change_in_progress: false,
            nodes: vec![
                RaftMembershipNode {
                    node_id: 1,
                    peer_address: "https://one.example".into(),
                    role: RaftNodeRole::Voter,
                },
                RaftMembershipNode {
                    node_id: 2,
                    peer_address: "https://two.example".into(),
                    role: RaftNodeRole::Voter,
                },
                RaftMembershipNode {
                    node_id: 3,
                    peer_address: "https://three.example".into(),
                    role: RaftNodeRole::Learner,
                },
            ],
        }
    }

    #[test]
    fn membership_status_validates_stable_and_joint_voter_sets() {
        let mut response = status();
        response.validate().unwrap();
        response.effective_voter_configs = vec![vec![1, 2], vec![1, 2, 3]];
        response.voter_ids = vec![1, 2, 3];
        response.learner_ids.clear();
        response.nodes[2].role = RaftNodeRole::Voter;
        response.validate().unwrap();
    }

    #[test]
    fn membership_status_rejects_inconsistent_roles_and_duplicate_ids() {
        let mut response = status();
        response.nodes[2].role = RaftNodeRole::Voter;
        assert!(response.validate().is_err());

        let mut response = status();
        response.voter_ids.push(1);
        assert!(response.validate().is_err());
    }

    #[test]
    fn membership_change_request_rejects_empty_duplicate_and_zero_ids() {
        assert!(RaftMembershipChangeRequest::new("cluster-1", 4, vec![], vec![1]).is_err());
        assert!(RaftMembershipChangeRequest::new("cluster-1", 4, vec![1, 1], vec![1, 2]).is_err());
        assert!(RaftMembershipChangeRequest::new("cluster-1", 4, vec![1], vec![0, 2]).is_err());
    }

    #[test]
    fn membership_change_response_must_match_requested_voters() {
        let request =
            RaftMembershipChangeRequest::new("cluster-1", 4, vec![1, 2], vec![1, 3]).unwrap();
        let mut response = RaftMembershipChangeResponse {
            status: RaftMembershipChangeStatus::MembershipChangeStarted,
            requested_voter_ids: vec![1, 3],
            effective_membership_log_index: Some(4),
            effective_voter_configs: vec![vec![1, 2]],
        };
        response.validate(&request).unwrap();

        response.requested_voter_ids = vec![1, 2];
        assert!(response.validate(&request).is_err());
    }
}

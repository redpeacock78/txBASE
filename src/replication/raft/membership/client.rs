use super::super::{MAX_RAFT_RPC_BYTES, RAFT_ADD_LEARNER_PATH, RAFT_MEMBERSHIP_PATH};
use super::types::{
    RaftAddLearnerRequest, RaftLearnerAddResponse, RaftMembershipChangeRequest,
    RaftMembershipChangeResponse, RaftMembershipStatus, invalid_request, invalid_response,
};
use crate::replication::{ReplicationHttpClient, ReplicationHttpError};
use serde::{Serialize, de::DeserializeOwned};
use std::path::Path;
use std::time::Duration;

#[derive(Clone)]
pub struct RaftMembershipHttpClient {
    client: ReplicationHttpClient,
}

impl RaftMembershipHttpClient {
    pub fn new(
        peer_url: &str,
        bearer_token: impl Into<String>,
    ) -> Result<Self, ReplicationHttpError> {
        Ok(Self {
            client: ReplicationHttpClient::new(peer_url)?.with_bearer_token(bearer_token)?,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, ReplicationHttpError> {
        self.client = self.client.with_timeout(timeout)?;
        Ok(self)
    }

    pub fn with_client_certificate_files(
        mut self,
        certificate: impl AsRef<Path>,
        private_key: impl AsRef<Path>,
    ) -> Result<Self, ReplicationHttpError> {
        self.client = self
            .client
            .with_client_certificate_files(certificate, private_key)?;
        Ok(self)
    }

    pub fn status(&self) -> Result<RaftMembershipStatus, ReplicationHttpError> {
        let body = self
            .client
            .get_json_bytes(RAFT_MEMBERSHIP_PATH, MAX_RAFT_RPC_BYTES)?;
        let status: RaftMembershipStatus = decode_response(&body, "membership status")?;
        status.validate()?;
        Ok(status)
    }

    pub fn add_learner(
        &self,
        request: &RaftAddLearnerRequest,
    ) -> Result<RaftLearnerAddResponse, ReplicationHttpError> {
        request.validate()?;
        let body = encode_request(request)?;
        let response =
            self.client
                .post_json_bytes(RAFT_ADD_LEARNER_PATH, &body, MAX_RAFT_RPC_BYTES)?;
        let response: RaftLearnerAddResponse = decode_response(&response, "learner add")?;
        response.validate(request.node_id)?;
        Ok(response)
    }

    pub fn change_membership(
        &self,
        request: &RaftMembershipChangeRequest,
    ) -> Result<RaftMembershipChangeResponse, ReplicationHttpError> {
        request.validate()?;
        let body = encode_request(request)?;
        let response =
            self.client
                .post_json_bytes(RAFT_MEMBERSHIP_PATH, &body, MAX_RAFT_RPC_BYTES)?;
        let response: RaftMembershipChangeResponse =
            decode_response(&response, "membership change")?;
        response.validate(request)?;
        Ok(response)
    }
}

fn encode_request<T: Serialize>(request: &T) -> Result<Vec<u8>, ReplicationHttpError> {
    let body = serde_json::to_vec(request)
        .map_err(|error| invalid_request(format!("cannot encode Raft request: {error}")))?;
    if body.len() > MAX_RAFT_RPC_BYTES {
        return Err(invalid_request(format!(
            "Raft membership request exceeds {MAX_RAFT_RPC_BYTES} bytes"
        )));
    }
    Ok(body)
}

fn decode_response<T: DeserializeOwned>(
    body: &[u8],
    label: &str,
) -> Result<T, ReplicationHttpError> {
    serde_json::from_slice(body)
        .map_err(|error| invalid_response(format!("invalid Raft {label} response: {error}")))
}

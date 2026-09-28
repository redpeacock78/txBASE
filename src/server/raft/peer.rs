use super::{CatalogRaftConfig, RPC_TIMEOUT, RaftRuntime, config};
use crate::replication::raft::{self, MAX_RAFT_RPC_BYTES, TypeConfig};
use crate::server::{HttpResponse, error, header, json_response, read_json_body_with_limit};
use openraft::raft::{AppendEntriesRequest, InstallSnapshotRequest, VoteRequest};
use serde::Serialize;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use tiny_http::{Method, Request, Server};

pub(in crate::server) struct PeerListener {
    server: Arc<Server>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for PeerListener {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl RaftRuntime {
    pub(in crate::server) fn bind_peer_listener(
        &self,
        config: &CatalogRaftConfig,
    ) -> Result<Server, String> {
        if self.peer_tls {
            let certificate = config::read_tls_file(config.tls_certificate.as_deref())?;
            let private_key = config::read_tls_file(config.tls_private_key.as_deref())?;
            Server::https(
                &config.peer_bind,
                tiny_http::SslConfig {
                    certificate,
                    private_key,
                },
            )
            .map_err(|error| {
                format!(
                    "cannot bind Raft peer listener {}: {error}",
                    config.peer_bind
                )
            })
        } else {
            Server::http(&config.peer_bind).map_err(|error| {
                format!(
                    "cannot bind Raft peer listener {}: {error}",
                    config.peer_bind
                )
            })
        }
    }

    pub(in crate::server) fn spawn_peer_listener(
        &self,
        server: Server,
    ) -> Result<PeerListener, String> {
        let raft = self.clone();
        let server = Arc::new(server);
        let listener_server = server.clone();
        let listener_thread = thread::Builder::new()
            .name(format!("txbase-raft-peer-{}", self.node_id))
            .spawn(move || {
                for mut request in listener_server.incoming_requests() {
                    let path = request.url().split('?').next().unwrap_or("/").to_owned();
                    let response = raft.peer_response(&mut request, &path).unwrap_or_else(|| {
                        json_response(404, error("not_found", "Raft peer route not found"), false)
                    });
                    if let Err(error) = request.respond(response) {
                        eprintln!("failed to send Raft peer response: {error}");
                    }
                }
            })
            .map_err(|error| format!("cannot start Raft peer listener: {error}"))?;
        Ok(PeerListener {
            server,
            thread: Some(listener_thread),
        })
    }

    fn peer_response(&self, request: &mut Request, path: &str) -> Option<HttpResponse> {
        if !path.starts_with("/raft/") {
            return None;
        }
        if request.method() != &Method::Post {
            return Some(
                json_response(
                    405,
                    error("method_not_allowed", "Raft RPC requires POST"),
                    false,
                )
                .with_header(header("Allow", "POST")),
            );
        }
        if let Err(response) = crate::server::replication::authorize(request, &self.token) {
            return Some(response);
        }
        if request.secure() != self.peer_tls {
            return Some(json_response(
                400,
                error(
                    "raft_transport_mismatch",
                    "peer transport security does not match this node",
                ),
                false,
            ));
        }
        Some(match path {
            raft::RAFT_VOTE_PATH => self.vote(request),
            raft::RAFT_APPEND_PATH => self.append_entries(request),
            raft::RAFT_SNAPSHOT_PATH => self.install_snapshot(request),
            _ => json_response(404, error("not_found", "Raft peer route not found"), false),
        })
    }

    fn vote(&self, request: &mut Request) -> HttpResponse {
        let rpc = match read_rpc::<VoteRequest<u64>>(request, raft::RAFT_VOTE_PATH) {
            Ok(rpc)
                if self.validate_sender(
                    rpc.sender_id,
                    &rpc.cluster_id,
                    rpc.version,
                    &rpc.genesis_fingerprint,
                    rpc.payload.vote.leader_id.voted_for,
                ) =>
            {
                rpc
            }
            Ok(_) => return rejected_peer(),
            Err(response) => return response,
        };
        match self.runtime.block_on(async {
            tokio::time::timeout(RPC_TIMEOUT, self.node.vote(rpc.payload)).await
        }) {
            Ok(result) => self.rpc_response(result),
            Err(_) => json_response(504, error("raft_rpc_timeout", "vote RPC timed out"), false),
        }
    }

    fn append_entries(&self, request: &mut Request) -> HttpResponse {
        let rpc =
            match read_rpc::<AppendEntriesRequest<TypeConfig>>(request, raft::RAFT_APPEND_PATH) {
                Ok(rpc)
                    if self.validate_sender(
                        rpc.sender_id,
                        &rpc.cluster_id,
                        rpc.version,
                        &rpc.genesis_fingerprint,
                        rpc.payload.vote.leader_id.voted_for,
                    ) =>
                {
                    rpc
                }
                Ok(_) => return rejected_peer(),
                Err(response) => return response,
            };
        match self.runtime.block_on(async {
            tokio::time::timeout(RPC_TIMEOUT, self.node.append_entries(rpc.payload)).await
        }) {
            Ok(result) => self.rpc_response(result),
            Err(_) => json_response(
                504,
                error("raft_rpc_timeout", "append RPC timed out"),
                false,
            ),
        }
    }

    fn install_snapshot(&self, request: &mut Request) -> HttpResponse {
        let rpc =
            match read_rpc::<InstallSnapshotRequest<TypeConfig>>(request, raft::RAFT_SNAPSHOT_PATH)
            {
                Ok(rpc)
                    if self.validate_sender(
                        rpc.sender_id,
                        &rpc.cluster_id,
                        rpc.version,
                        &rpc.genesis_fingerprint,
                        rpc.payload.vote.leader_id.voted_for,
                    ) =>
                {
                    rpc
                }
                Ok(_) => return rejected_peer(),
                Err(response) => return response,
            };
        match self.runtime.block_on(async {
            tokio::time::timeout(RPC_TIMEOUT, self.node.install_snapshot(rpc.payload)).await
        }) {
            Ok(result) => self.rpc_response(result),
            Err(_) => json_response(
                504,
                error("raft_rpc_timeout", "snapshot RPC timed out"),
                false,
            ),
        }
    }

    fn validate_sender(
        &self,
        sender_id: u64,
        cluster_id: &str,
        version: u16,
        genesis_fingerprint: &[u8],
        vote_sender: Option<u64>,
    ) -> bool {
        if sender_id == self.node_id
            || sender_id == 0
            || vote_sender != Some(sender_id)
            || cluster_id != self.cluster_id
            || version != raft::RAFT_RPC_VERSION
            || genesis_fingerprint != self.genesis_fingerprint
        {
            return false;
        }
        let metrics = self.node.metrics();
        let membership = &metrics.borrow().membership_config;
        if membership.log_id().is_some() {
            membership.nodes().any(|(node_id, _)| *node_id == sender_id)
        } else {
            self.initial_members.contains(&sender_id)
        }
    }

    fn rpc_response<T: Serialize, E: Serialize>(&self, result: Result<T, E>) -> HttpResponse {
        serde_json::to_value(raft::raft_rpc_reply(
            &self.cluster_id,
            self.node_id,
            &self.genesis_fingerprint,
            result,
        ))
        .map(|value| json_response(200, value, false))
        .unwrap_or_else(|serialization_error| {
            json_response(
                500,
                error("raft_rpc_serialization", &serialization_error.to_string()),
                false,
            )
        })
    }
}

fn read_rpc<T: serde::de::DeserializeOwned>(
    request: &mut Request,
    operation: &str,
) -> Result<raft::RpcRequestWire<T>, HttpResponse> {
    let bytes = read_json_body_with_limit(request, operation, false, MAX_RAFT_RPC_BYTES)?;
    serde_json::from_slice(&bytes).map_err(|parse_error| {
        json_response(
            400,
            error("invalid_raft_rpc", &parse_error.to_string()),
            false,
        )
    })
}

fn rejected_peer() -> HttpResponse {
    json_response(
        403,
        error(
            "raft_peer_rejected",
            "Raft RPC sender is not an active member",
        ),
        false,
    )
}

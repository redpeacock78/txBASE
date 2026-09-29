mod failover;
mod idempotency;
mod membership;
mod membership_recovery;

use super::raft::RaftRuntime;
use crate::catalog::Catalog;
use crate::replication::raft::{
    RAFT_MEMBERSHIP_PATH, RaftCommand, RaftCommandPrecondition, RaftResponseResult,
};
use crate::server::CatalogRaftConfig;
use crate::xbase::{OperationIr, OperationMethod, TransactionStep};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_CLUSTER_ID: AtomicUsize = AtomicUsize::new(0);

pub(super) fn temporary_cluster() -> std::path::PathBuf {
    let id = NEXT_CLUSTER_ID.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("txbase-raft-cluster-{}-{id}", std::process::id()));
    fs::create_dir(&root).unwrap();
    root
}

pub(super) fn free_address() -> String {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .to_string()
}

pub(super) fn peer_request(
    peer_url: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    token: &str,
) -> String {
    let authority = peer_url.strip_prefix("http://").unwrap();
    let content_type = if body.is_some() {
        "Content-Type: application/json\r\n"
    } else {
        ""
    };
    let body = body
        .map(|body| serde_json::to_vec(&body).unwrap())
        .unwrap_or_default();
    let mut stream = TcpStream::connect(authority).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {token}\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(&body).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    String::from_utf8(response).unwrap()
}

pub(super) fn response_json(response: &str) -> (u16, Value) {
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    let status = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, serde_json::from_str(body).unwrap())
}

pub(super) fn post_add_learner(peer_url: &str, learner_url: &str) -> String {
    peer_request(
        peer_url,
        "POST",
        "/raft/v1/learner",
        Some(json!({
            "version": crate::replication::raft::RAFT_RPC_VERSION,
            "cluster_id": "ci-raft-cluster",
            "node_id": 3,
            "peer_address": learner_url,
        })),
        "ci-token",
    )
}

pub(super) fn get_membership(peer_url: &str, token: &str) -> (u16, Value) {
    response_json(&peer_request(
        peer_url,
        "GET",
        RAFT_MEMBERSHIP_PATH,
        None,
        token,
    ))
}

pub(super) fn post_membership(peer_url: &str, body: Value) -> (u16, Value) {
    response_json(&peer_request(
        peer_url,
        "POST",
        RAFT_MEMBERSHIP_PATH,
        Some(body),
        "ci-token",
    ))
}

pub(super) fn membership_change_body(
    expected_index: u64,
    expected_voter_ids: &[u64],
    voter_ids: &[u64],
) -> Value {
    json!({
        "version": crate::replication::raft::RAFT_RPC_VERSION,
        "cluster_id": "ci-raft-cluster",
        "expected_membership_log_index": expected_index,
        "expected_voter_ids": expected_voter_ids,
        "voter_ids": voter_ids
    })
}

pub(super) fn record_command(
    catalog_root: &Path,
    sequence: u64,
    record_id: i64,
    name: &str,
    age: i64,
) -> RaftCommand {
    let catalog = Catalog::from_path(catalog_root).unwrap();
    let (_, catalog_tag) = catalog.schema_representation().unwrap();
    RaftCommand::new(
        "ci-client".into(),
        sequence,
        catalog_tag,
        None::<RaftCommandPrecondition>,
        vec![TransactionStep::Mutation(OperationIr {
            method: OperationMethod::Post,
            path: "/users/records".into(),
            body: Some(json!({
                "ID": record_id,
                "NAME": name,
                "AGE": age,
                "ACTIVE": true
            })),
        })],
    )
    .unwrap()
}

pub(super) fn commit(leader: &RaftRuntime, command: RaftCommand) -> RaftResponseResult {
    leader
        .runtime
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(15), leader.node.client_write(command)).await
        })
        .unwrap()
        .unwrap()
        .data
        .unwrap()
        .result
}

pub(super) fn current_leader_index(nodes: &[RaftRuntime], timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    loop {
        for (index, node) in nodes.iter().enumerate() {
            let is_leader = {
                let metrics = node.node.metrics();
                metrics.borrow().current_leader == Some(node.node_id)
            };
            if is_leader && node.linearizable_read().is_ok() {
                return index;
            }
        }
        assert!(
            Instant::now() < deadline,
            "Raft cluster did not elect a leader"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn has_membership(
    node: &RaftRuntime,
    expected_voter_ids: &BTreeSet<u64>,
    expected_learner_ids: &BTreeSet<u64>,
) -> bool {
    let metrics = node.node.metrics();
    let metrics = metrics.borrow();
    let membership = &metrics.membership_config;
    membership.membership().get_joint_config() == &vec![expected_voter_ids.clone()]
        && membership
            .membership()
            .learner_ids()
            .collect::<BTreeSet<_>>()
            == *expected_learner_ids
}

pub(super) fn wait_for_membership(
    nodes: &[RaftRuntime],
    expected_voter_ids: &BTreeSet<u64>,
    expected_learner_ids: &BTreeSet<u64>,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        if nodes
            .iter()
            .all(|node| has_membership(node, expected_voter_ids, expected_learner_ids))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Raft membership did not converge to voters {expected_voter_ids:?} and learners {expected_learner_ids:?}"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn wait_for_transaction(
    nodes: &[RaftRuntime],
    root: &Path,
    expected: u64,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        if nodes.iter().all(|node| {
            Catalog::from_path(root.join(format!("catalog-{}", node.node_id)))
                .ok()
                .and_then(|catalog| catalog.transaction_id().ok().flatten())
                == Some(expected)
        }) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "catalog transaction {expected} did not reach every active node"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn prepare_catalog(root: &Path) {
    fs::create_dir(root).unwrap();
    let dbf = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect::<Vec<_>>();
    fs::write(root.join("users.dbf"), dbf).unwrap();
    let catalog = Catalog::from_path(root).unwrap();
    let mut transaction = catalog.begin_serializable().unwrap();
    transaction
        .apply(&OperationIr {
            method: OperationMethod::Post,
            path: "/users/records".into(),
            body: Some(json!({
                "ID": 3,
                "NAME": "Genesis",
                "AGE": 42,
                "ACTIVE": true
            })),
        })
        .unwrap();
    assert_eq!(transaction.commit().unwrap(), 1);
}

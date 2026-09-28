use super::raft::RaftRuntime;
use crate::catalog::Catalog;
use crate::replication::raft::{RaftCommand, RaftCommandPrecondition, RaftResponseResult};
use crate::server::CatalogRaftConfig;
use crate::xbase::{OperationIr, OperationMethod, TransactionStep};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_CLUSTER_ID: AtomicUsize = AtomicUsize::new(0);

fn temporary_cluster() -> std::path::PathBuf {
    let id = NEXT_CLUSTER_ID.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("txbase-raft-cluster-{}-{id}", std::process::id()));
    fs::create_dir(&root).unwrap();
    root
}

fn free_address() -> String {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .to_string()
}

fn prepare_catalog(root: &Path) {
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

#[test]
fn three_nodes_commit_over_authenticated_peer_rpc_and_deduplicate_retry() {
    let root = temporary_cluster();
    let addresses = (0..3).map(|_| free_address()).collect::<Vec<_>>();
    let members = addresses
        .iter()
        .enumerate()
        .map(|(index, address)| (index as u64 + 1, format!("http://{address}")))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = Vec::new();
    let mut listeners = Vec::new();

    for node_id in 1..=3 {
        let catalog_root = root.join(format!("catalog-{node_id}"));
        prepare_catalog(&catalog_root);
        let config = CatalogRaftConfig {
            node_id,
            cluster_id: "ci-raft-cluster".into(),
            node_directory: root.join(format!("node-{node_id}")),
            peer_bind: addresses[(node_id - 1) as usize].clone(),
            peer_advertise: members[&node_id].clone(),
            initial_members: members.clone(),
            bootstrap: node_id == 1,
            initialize_catalog: node_id != 1,
            tls_certificate: None,
            tls_private_key: None,
        };
        let raft =
            RaftRuntime::start(&catalog_root, config.clone(), Some("ci-token".into())).unwrap();
        let server = raft.bind_peer_listener(&config).unwrap();
        listeners.push(raft.spawn_peer_listener(server).unwrap());
        nodes.push(raft);
    }

    let deadline = Instant::now() + Duration::from_secs(20);
    let leader_index = loop {
        let elected = nodes.iter().position(|node| {
            let metrics = node.node.metrics();
            metrics.borrow().current_leader == Some(node.node_id)
        });
        if let Some(index) = elected {
            if nodes[index].linearizable_read().is_ok() {
                break index;
            }
        }
        assert!(
            Instant::now() < deadline,
            "three-node Raft cluster did not elect a leader"
        );
        thread::sleep(Duration::from_millis(100));
    };

    let leader = &nodes[leader_index];
    let catalog_root = root.join(format!("catalog-{}", leader.node_id));
    let catalog = Catalog::from_path(&catalog_root).unwrap();
    let (_, catalog_tag) = catalog.schema_representation().unwrap();
    let command = RaftCommand::new(
        "ci-client".into(),
        1,
        catalog_tag,
        None::<RaftCommandPrecondition>,
        vec![TransactionStep::Mutation(OperationIr {
            method: OperationMethod::Post,
            path: "/users/records".into(),
            body: Some(json!({
                "ID": 4,
                "NAME": "Quorum",
                "AGE": 43,
                "ACTIVE": true
            })),
        })],
    )
    .unwrap();
    let first = leader
        .runtime
        .block_on(async {
            tokio::time::timeout(
                Duration::from_secs(15),
                leader.node.client_write(command.clone()),
            )
            .await
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        first.data.unwrap().result,
        RaftResponseResult::Applied { transaction_id: 2 }
    );

    let retry = leader
        .runtime
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(15), leader.node.client_write(command)).await
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        retry.data.unwrap().result,
        RaftResponseResult::Applied { transaction_id: 2 }
    );

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let applied_everywhere = (1..=3).all(|node_id| {
            Catalog::from_path(root.join(format!("catalog-{node_id}")))
                .ok()
                .and_then(|catalog| catalog.transaction_id().ok().flatten())
                == Some(2)
        });
        if applied_everywhere {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Raft write did not reach every node"
        );
        thread::sleep(Duration::from_millis(100));
    }
    for node_id in 1..=3 {
        let catalog = Catalog::from_path(root.join(format!("catalog-{node_id}"))).unwrap();
        assert_eq!(catalog.open_table("users").unwrap().records().len(), 4);
    }

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

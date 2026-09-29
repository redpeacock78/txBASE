use super::*;
use crate::server::{CatalogRaftConfig, catalog_transaction, header};
use std::io::Read;
use std::path::Path;
use std::process::Command;
use tiny_http::{Method, StatusCode, TestRequest};

const ROOT_ENV: &str = "TXBASE_RAFT_CRASH_TEST_ROOT";
const ADDRESSES_ENV: &str = "TXBASE_RAFT_CRASH_TEST_ADDRESSES";
const POINT_ENV: &str = "TXBASE_RAFT_CRASH_TEST_POINT";
const TRANSACTION_BODY: &str = r#"{"operations":[{"method":"POST","path":"/users/records","body":{"ID":4,"NAME":"Crash","AGE":43,"ACTIVE":true}}]}"#;

#[test]
fn crash_boundaries_recover_without_duplicate_transactions() {
    for point in [
        "log_entry_persisted",
        "commit_marker_persisted",
        "catalog_and_applied_state_persisted",
        "raft_response_received",
    ] {
        let root = temporary_cluster();
        for node_id in 1..=3 {
            prepare_catalog(&root.join(format!("catalog-{node_id}")));
        }
        let addresses = (0..3).map(|_| free_address()).collect::<Vec<_>>();
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("crash_boundary_child")
            .arg("--nocapture")
            .env(ROOT_ENV, &root)
            .env(ADDRESSES_ENV, addresses.join(","))
            .env(POINT_ENV, point)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(86),
            "crash point {point} did not terminate the child as expected\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        with_cluster(&root, &addresses, point, |nodes| {
            let leader_index = current_leader_index(nodes, Duration::from_secs(20));
            let leader = &nodes[leader_index];
            let catalog =
                Catalog::from_path(root.join(format!("catalog-{}", leader.node_id))).unwrap();
            let response = transaction_response(leader, &catalog);
            let status = response.status_code();
            let mut body = String::new();
            response.into_reader().read_to_string(&mut body).unwrap();
            assert_eq!(
                status,
                StatusCode(200),
                "crash point {point}, response body: {body}"
            );
            let transaction_id = serde_json::from_str::<Value>(&body).unwrap()["transaction_id"]
                .as_u64()
                .unwrap();
            assert_eq!(transaction_id, 2, "crash point {point}");
            wait_for_transaction(nodes, &root, transaction_id, Duration::from_secs(15));

            for node in nodes {
                let catalog =
                    Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
                assert_eq!(catalog.transaction_id().unwrap(), Some(transaction_id));
                let table = catalog.open_table("users").unwrap();
                assert_eq!(table.records().len(), 4, "crash point {point}");
                assert_eq!(table.active_record(4).unwrap().values["NAME"], "Crash");
            }
        });
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn crash_boundary_child() {
    let Ok(root) = std::env::var(ROOT_ENV) else {
        return;
    };
    let addresses = std::env::var(ADDRESSES_ENV)
        .unwrap()
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let point = std::env::var(POINT_ENV).unwrap();
    with_cluster(Path::new(&root), &addresses, &point, |nodes| {
        let leader_index = current_leader_index(nodes, Duration::from_secs(20));
        let leader = &nodes[leader_index];
        let catalog =
            Catalog::from_path(Path::new(&root).join(format!("catalog-{}", leader.node_id)))
                .unwrap();
        crate::test_support::arm_crash_at(&point);
        let response = transaction_response(leader, &catalog);
        panic!(
            "crash hook {point} did not terminate the child; HTTP status was {:?}",
            response.status_code()
        );
    });
}

fn with_cluster<T>(
    root: &Path,
    addresses: &[String],
    crash_point: &str,
    run: impl FnOnce(&[RaftRuntime]) -> T,
) -> T {
    let members = addresses
        .iter()
        .enumerate()
        .map(|(index, address)| (index as u64 + 1, format!("http://{address}")))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = Vec::new();
    let mut listeners = Vec::new();
    for node_id in 1..=3 {
        let catalog_root = root.join(format!("catalog-{node_id}"));
        let config = CatalogRaftConfig {
            node_id,
            cluster_id: "ci-raft-crash-recovery".into(),
            node_directory: root.join(format!("node-{node_id}")),
            peer_bind: addresses[(node_id - 1) as usize].clone(),
            peer_advertise: members[&node_id].clone(),
            initial_members: members.clone(),
            bootstrap: node_id == 1,
            initialize_catalog: node_id != 1,
            tls_certificate: None,
            tls_private_key: None,
        };
        let node = RaftRuntime::start(&catalog_root, config.clone(), Some("ci-token".into()))
            .unwrap_or_else(|error| {
                panic!(
                    "crash point {crash_point}: cannot start node {node_id} with catalog {} and node directory {}: {error}",
                    catalog_root.display(),
                    config.node_directory.display()
                )
            });
        let server = node.bind_peer_listener(&config).unwrap();
        listeners.push(node.spawn_peer_listener(server).unwrap());
        nodes.push(node);
    }
    wait_for_membership(
        &nodes,
        &BTreeSet::from([1, 2, 3]),
        &BTreeSet::new(),
        Duration::from_secs(20),
    );
    let result = run(&nodes);
    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    result
}

fn transaction_response(raft: &RaftRuntime, catalog: &Catalog) -> crate::server::HttpResponse {
    let mut request = TestRequest::new()
        .with_method(Method::Post)
        .with_path("/transaction")
        .with_header(header("Content-Type", "application/json"))
        .with_header(header("X-Txbase-Client-Id", "crash-boundary-client"))
        .with_header(header("X-Txbase-Client-Sequence", "1"))
        .with_body(TRANSACTION_BODY)
        .into();
    catalog_transaction::response_with_raft(&mut request, catalog, raft)
}

use super::*;
use crate::server::{CatalogRaftConfig, catalog_transaction, header};
use std::io::Read;
use tiny_http::{Method, StatusCode, TestRequest};

#[test]
fn transaction_retry_after_lost_response_returns_the_committed_result_once() {
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
            cluster_id: "ci-raft-idempotency".into(),
            node_directory: root.join(format!("node-{node_id}")),
            peer_bind: addresses[(node_id - 1) as usize].clone(),
            peer_advertise: members[&node_id].clone(),
            initial_members: members.clone(),
            bootstrap: node_id == 1,
            initialize_catalog: node_id != 1,
            tls_certificate: None,
            tls_private_key: None,
        };
        let node =
            RaftRuntime::start(&catalog_root, config.clone(), Some("ci-token".into())).unwrap();
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
    let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let leader_id = nodes[leader_index].node_id;
    let catalog = Catalog::from_path(root.join(format!("catalog-{leader_id}"))).unwrap();

    let first_response = transaction_response(
        &nodes[leader_index],
        &catalog,
        r#"{"operations":[{"method":"POST","path":"/users/records","body":{"ID":4,"NAME":"Retry","AGE":43,"ACTIVE":true}}]}"#,
    );
    assert_eq!(first_response.status_code(), StatusCode(200));
    drop(first_response);

    let retry_response = transaction_response(
        &nodes[leader_index],
        &catalog,
        r#"{"operations":[{"method":"POST","path":"/users/records","body":{"ID":4,"NAME":"Retry","AGE":43,"ACTIVE":true}}]}"#,
    );
    assert_eq!(retry_response.status_code(), StatusCode(200));
    let mut body = String::new();
    retry_response
        .into_reader()
        .read_to_string(&mut body)
        .unwrap();
    let transaction_id = serde_json::from_str::<Value>(&body).unwrap()["transaction_id"]
        .as_u64()
        .unwrap();
    assert_eq!(transaction_id, 2);
    wait_for_transaction(&nodes, &root, transaction_id, Duration::from_secs(15));

    for node in &nodes {
        let catalog = Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
        assert_eq!(catalog.transaction_id().unwrap(), Some(transaction_id));
        let table = catalog.open_table("users").unwrap();
        assert_eq!(table.records().len(), 4);
        assert_eq!(table.active_record(4).unwrap().values["NAME"], "Retry");
    }

    let conflicting_response = transaction_response(
        &nodes[leader_index],
        &catalog,
        r#"{"operations":[{"method":"POST","path":"/users/records","body":{"ID":5,"NAME":"Conflict","AGE":44,"ACTIVE":true}}]}"#,
    );
    assert_eq!(conflicting_response.status_code(), StatusCode(409));
    assert_eq!(catalog.transaction_id().unwrap(), Some(transaction_id));
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(5)
            .is_none()
    );

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

fn transaction_response(
    raft: &RaftRuntime,
    catalog: &Catalog,
    body: &'static str,
) -> crate::server::HttpResponse {
    let mut request = TestRequest::new()
        .with_method(Method::Post)
        .with_path("/transaction")
        .with_header(header("Content-Type", "application/json"))
        .with_header(header("X-Txbase-Client-Id", "lost-response-client"))
        .with_header(header("X-Txbase-Client-Sequence", "1"))
        .with_body(body)
        .into();
    catalog_transaction::response_with_raft(&mut request, catalog, raft)
}

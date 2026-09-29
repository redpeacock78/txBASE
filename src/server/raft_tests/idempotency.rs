use super::*;
use crate::server::CatalogRaftConfig;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread;
use tiny_http::{Server, StatusCode};

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
    let catalog_root = root.join(format!("catalog-{leader_id}"));
    let server = Server::http("127.0.0.1:0").unwrap();
    let service_address = server.server_addr().to_ip().unwrap().to_string();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_address = proxy.local_addr().unwrap().to_string();

    let (dropped_response, retry_response, conflicting_response) = thread::scope(|scope| {
        let service = scope.spawn(|| {
            let mut catalog = Catalog::from_path(&catalog_root).unwrap();
            for _ in 0..3 {
                let request = server
                    .recv_timeout(Duration::from_secs(60))
                    .unwrap()
                    .unwrap();
                super::super::catalog::handle_raft_request(
                    request,
                    &mut catalog,
                    &nodes[leader_index],
                );
            }
        });

        let fault_proxy = scope.spawn(move || {
            let mut dropped_response = None;
            for request_index in 0..3 {
                let (mut client, _) = proxy.accept().unwrap();
                let mut request = Vec::new();
                client.read_to_end(&mut request).unwrap();

                let mut upstream = TcpStream::connect(&service_address).unwrap();
                upstream
                    .set_read_timeout(Some(Duration::from_secs(60)))
                    .unwrap();
                upstream.write_all(&request).unwrap();
                upstream.shutdown(Shutdown::Write).unwrap();
                let mut response = Vec::new();
                upstream.read_to_end(&mut response).unwrap();

                if request_index == 0 {
                    dropped_response = Some(response);
                } else {
                    client.write_all(&response).unwrap();
                    client.shutdown(Shutdown::Write).unwrap();
                }
            }
            dropped_response.unwrap()
        });

        let first_response = send_transaction(&proxy_address, RETRY_BODY);
        assert!(first_response.is_empty());
        let retry_response = send_transaction(&proxy_address, RETRY_BODY);
        let conflicting_response = send_transaction(&proxy_address, CONFLICTING_BODY);
        let dropped_response = fault_proxy.join().unwrap();
        service.join().unwrap();
        (dropped_response, retry_response, conflicting_response)
    });

    let (dropped_status, dropped_body) = response_json(&dropped_response);
    let (retry_status, retry_body) = response_json(&retry_response);
    assert_eq!(dropped_status, StatusCode(200));
    assert_eq!(retry_status, StatusCode(200));
    assert_eq!(retry_body, dropped_body);
    let transaction_id = retry_body["transaction_id"].as_u64().unwrap();
    assert_eq!(transaction_id, 2);
    wait_for_transaction(&nodes, &root, transaction_id, Duration::from_secs(15));

    for node in &nodes {
        let catalog = Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
        assert_eq!(catalog.transaction_id().unwrap(), Some(transaction_id));
        let table = catalog.open_table("users").unwrap();
        assert_eq!(table.records().len(), 4);
        assert_eq!(table.active_record(4).unwrap().values["NAME"], "Retry");
    }

    assert_eq!(response_status(&conflicting_response), StatusCode(409));
    let catalog = Catalog::from_path(catalog_root).unwrap();
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

const RETRY_BODY: &str = r#"{"operations":[{"method":"POST","path":"/users/records","body":{"ID":4,"NAME":"Retry","AGE":43,"ACTIVE":true}}]}"#;
const CONFLICTING_BODY: &str = r#"{"operations":[{"method":"POST","path":"/users/records","body":{"ID":5,"NAME":"Conflict","AGE":44,"ACTIVE":true}}]}"#;

fn send_transaction(address: &str, body: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    write!(
        stream,
        "POST /transaction HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nX-Txbase-Client-Id: lost-response-client\r\nX-Txbase-Client-Sequence: 1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    response
}

fn response_status(response: &[u8]) -> StatusCode {
    let response = std::str::from_utf8(response).unwrap();
    let status = response
        .lines()
        .next()
        .unwrap()
        .split_ascii_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    StatusCode(status)
}

fn response_json(response: &[u8]) -> (StatusCode, Value) {
    let response = std::str::from_utf8(response).unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    let status = response_status(headers.as_bytes());
    (status, serde_json::from_str(body).unwrap())
}

use super::*;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;
use tiny_http::Server;

#[test]
fn follower_read_token_waits_for_its_index_and_can_be_chained() {
    let root = temporary_cluster();
    let addresses = free_addresses(3);
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
            cluster_id: "ci-raft-follower-reads".into(),
            node_directory: root.join(format!("node-{node_id}")),
            peer_bind: addresses[(node_id - 1) as usize].clone(),
            peer_advertise: members[&node_id].clone(),
            initial_members: members.clone(),
            bootstrap: node_id == 1,
            initialize_catalog: node_id != 1,
            tls_certificate: None,
            tls_private_key: None,
            tls_client_ca: None,
        };
        let node =
            RaftRuntime::start(&catalog_root, config.clone(), Some("ci-token".into())).unwrap();
        let server = node.bind_peer_listener(&config).unwrap();
        listeners.push(node.spawn_peer_listener(server).unwrap());
        nodes.push(node);
    }

    let voters = BTreeSet::from([1, 2, 3]);
    wait_for_membership(&nodes, &voters, &BTreeSet::new(), Duration::from_secs(20));
    let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let leader_id = nodes[leader_index].node_id;
    let follower_index = (0..nodes.len())
        .find(|index| *index != leader_index)
        .unwrap();
    let follower_id = nodes[follower_index].node_id;
    let quorum = nodes
        .iter()
        .filter(|node| node.node_id != follower_id)
        .cloned()
        .collect::<Vec<_>>();

    let catalog_root = root.join(format!("catalog-{leader_id}"));
    let initial_response =
        catalog_http_request(&nodes[leader_index], &catalog_root, None, None, "/catalog");
    let (initial_status, initial_body) = response_json(&initial_response);
    assert_eq!(initial_status, 200);
    let initial_token = response_header(&initial_response, super::super::raft::READ_TOKEN_HEADER)
        .expect("a linearizable read returns its applied position");
    assert_eq!(
        initial_body["transaction_id"].as_u64(),
        Catalog::from_path(&catalog_root)
            .unwrap()
            .transaction_id()
            .unwrap()
    );

    let wrong_cluster_token = format!("v1.other-cluster.{}", token_index(initial_token));
    let mismatch = catalog_http_request(
        &nodes[follower_index],
        &root.join(format!("catalog-{follower_id}")),
        Some(&wrong_cluster_token),
        None,
        "/catalog",
    );
    let (mismatch_status, mismatch_body) = response_json(&mismatch);
    assert_eq!(mismatch_status, 409);
    assert_eq!(
        mismatch_body["error"]["code"],
        "raft_read_token_cluster_mismatch"
    );

    let historical_read = catalog_http_request(
        &nodes[leader_index],
        &catalog_root,
        Some(initial_token),
        None,
        "/catalog?at=1",
    );
    assert_eq!(response_json(&historical_read).0, 400);

    for node in &nodes {
        if node.node_id == follower_id {
            for peer_id in &voters {
                if *peer_id != follower_id {
                    node.set_peer_blocked(*peer_id, true).unwrap();
                }
            }
        } else {
            node.set_peer_blocked(follower_id, true).unwrap();
        }
    }

    let transaction_id = Catalog::from_path(&catalog_root)
        .unwrap()
        .transaction_id()
        .unwrap()
        .unwrap();
    let command = record_command(&catalog_root, transaction_id, 50, "Follower", 50);
    assert_eq!(
        commit(&nodes[leader_index], command),
        RaftResponseResult::Applied {
            transaction_id: transaction_id + 1
        }
    );
    wait_for_transaction(&quorum, &root, transaction_id + 1, Duration::from_secs(15));

    let current_response =
        catalog_http_request(&nodes[leader_index], &catalog_root, None, None, "/catalog");
    let (current_status, current_body) = response_json(&current_response);
    assert_eq!(current_status, 200);
    assert_eq!(
        current_body["transaction_id"].as_u64(),
        Some(transaction_id + 1)
    );
    let minimum_token = response_header(&current_response, super::super::raft::READ_TOKEN_HEADER)
        .expect("a linearizable read returns its applied position")
        .to_owned();
    assert!(token_index(&minimum_token) > token_index(initial_token));

    let (received_tx, received_rx) = mpsc::channel();
    let (response_tx, response_rx) = mpsc::channel();
    let follower = nodes[follower_index].clone();
    let follower_catalog_root = root.join(format!("catalog-{follower_id}"));
    let request_token = minimum_token.clone();
    let request_thread = thread::spawn(move || {
        let response = catalog_http_request(
            &follower,
            &follower_catalog_root,
            Some(&request_token),
            Some(received_tx),
            "/catalog",
        );
        response_tx.send(response).unwrap();
    });
    received_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the follower request reached its read handler");
    assert!(matches!(
        response_rx.recv_timeout(Duration::from_millis(150)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));

    for node in &nodes {
        for peer_id in &voters {
            if *peer_id != node.node_id {
                node.set_peer_blocked(*peer_id, false).unwrap();
            }
        }
    }
    let follower_response = response_rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the follower responds after applying the requested index");
    request_thread.join().unwrap();
    let (follower_status, follower_body) = response_json(&follower_response);
    assert_eq!(follower_status, 200);
    assert_eq!(
        follower_body["transaction_id"].as_u64(),
        Some(transaction_id + 1)
    );
    let follower_token = response_header(&follower_response, super::super::raft::READ_TOKEN_HEADER)
        .expect("the follower returns its applied position");
    assert!(token_index(follower_token) >= token_index(&minimum_token));

    let chained_response = catalog_http_request(
        &nodes[follower_index],
        &root.join(format!("catalog-{follower_id}")),
        Some(follower_token),
        None,
        "/catalog",
    );
    let (chained_status, chained_body) = response_json(&chained_response);
    assert_eq!(chained_status, 200);
    assert_eq!(
        chained_body["transaction_id"].as_u64(),
        Some(transaction_id + 1)
    );
    let chained_token = response_header(&chained_response, super::super::raft::READ_TOKEN_HEADER)
        .expect("a chained follower read returns a token");
    assert!(token_index(chained_token) >= token_index(follower_token));

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

fn catalog_http_request(
    raft: &RaftRuntime,
    catalog_root: &std::path::Path,
    minimum_token: Option<&str>,
    received: Option<mpsc::Sender<()>>,
    path: &str,
) -> String {
    let server = Server::http("127.0.0.1:0").unwrap();
    let address = server.server_addr().to_ip().unwrap().to_string();
    let token = minimum_token.map(str::to_owned);
    let path = path.to_owned();
    let client = thread::spawn(move || {
        let mut stream = TcpStream::connect(&address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {address}\r\n{}Connection: close\r\n\r\n",
            token.map_or_else(String::new, |token| format!(
                "{}: {token}\r\n",
                super::super::raft::MIN_READ_TOKEN_HEADER
            ))
        )
        .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        String::from_utf8(response).unwrap()
    });
    let request = server
        .recv_timeout(Duration::from_secs(15))
        .unwrap()
        .expect("catalog request reached the test server");
    if let Some(received) = received {
        received.send(()).unwrap();
    }
    let mut catalog = Catalog::from_path(catalog_root).unwrap();
    super::super::catalog::handle_raft_request(request, &mut catalog, raft);
    client.join().unwrap()
}

fn response_header<'a>(response: &'a str, name: &str) -> Option<&'a str> {
    let (headers, _) = response.split_once("\r\n\r\n")?;
    headers.lines().skip(1).find_map(|line| {
        let (field, value) = line.split_once(':')?;
        field.eq_ignore_ascii_case(name).then_some(value.trim())
    })
}

fn token_index(token: &str) -> u64 {
    token.rsplit('.').next().unwrap().parse().unwrap()
}

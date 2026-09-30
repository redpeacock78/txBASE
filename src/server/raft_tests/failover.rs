mod append_delivery;

use super::*;

#[test]
fn partitioned_leader_cannot_commit_and_rejoins_after_majority_failover_and_restart() {
    const RELEASE_ORDERS: [[usize; 4]; 24] = [
        [0, 1, 2, 3],
        [0, 1, 3, 2],
        [0, 2, 1, 3],
        [0, 2, 3, 1],
        [0, 3, 1, 2],
        [0, 3, 2, 1],
        [1, 0, 2, 3],
        [1, 0, 3, 2],
        [1, 2, 0, 3],
        [1, 2, 3, 0],
        [1, 3, 0, 2],
        [1, 3, 2, 0],
        [2, 0, 1, 3],
        [2, 0, 3, 1],
        [2, 1, 0, 3],
        [2, 1, 3, 0],
        [2, 3, 0, 1],
        [2, 3, 1, 0],
        [3, 0, 1, 2],
        [3, 0, 2, 1],
        [3, 1, 0, 2],
        [3, 1, 2, 0],
        [3, 2, 0, 1],
        [3, 2, 1, 0],
    ];
    let unique_orders = RELEASE_ORDERS.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(unique_orders.len(), 24, "release orders must be unique");
    let expected_peers = BTreeSet::from([0, 1, 2, 3]);
    assert!(
        unique_orders
            .iter()
            .all(|order| BTreeSet::from(*order) == expected_peers),
        "every release order must be a permutation of the four peers"
    );
    for (scenario_index, release_order) in RELEASE_ORDERS.into_iter().enumerate() {
        run_partitioned_leader_scenario(release_order, scenario_index == 0);
    }
}

fn run_partitioned_leader_scenario(release_order: [usize; 4], verify_successive_delays: bool) {
    const NODE_COUNT: u64 = 5;

    let root = temporary_cluster();
    let mut addresses = BTreeSet::new();
    while addresses.len() < NODE_COUNT as usize {
        addresses.insert(free_address());
    }
    let addresses = addresses.into_iter().collect::<Vec<_>>();
    let members = addresses
        .iter()
        .enumerate()
        .map(|(index, address)| (index as u64 + 1, format!("http://{address}")))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = Vec::new();
    let mut listeners = Vec::new();
    let mut configs = Vec::new();

    for node_id in 1..=NODE_COUNT {
        let catalog_root = root.join(format!("catalog-{node_id}"));
        prepare_catalog(&catalog_root);
        let config = CatalogRaftConfig {
            node_id,
            cluster_id: "ci-raft-failover".into(),
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
        configs.push(config);
        nodes.push(node);
    }

    let expected_voters = (1..=NODE_COUNT).collect::<BTreeSet<_>>();
    wait_for_membership(
        &nodes,
        &expected_voters,
        &BTreeSet::new(),
        Duration::from_secs(20),
    );
    let bootstrap_leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let bootstrap_leader_id = nodes[bootstrap_leader_index].node_id;
    let initial_command = record_command(
        &root.join(format!("catalog-{bootstrap_leader_id}")),
        1,
        4,
        "Before",
        43,
    );
    assert_eq!(
        commit(&nodes[bootstrap_leader_index], initial_command),
        RaftResponseResult::Applied { transaction_id: 2 }
    );
    wait_for_transaction(&nodes, &root, 2, Duration::from_secs(15));

    let initial_leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let initial_leader_id = nodes[initial_leader_index].node_id;
    let majority = nodes
        .iter()
        .filter(|node| node.node_id != initial_leader_id)
        .cloned()
        .collect::<Vec<_>>();
    let mut delayed_appends = majority
        .iter()
        .map(|peer| {
            (
                peer.node_id,
                nodes[initial_leader_index]
                    .delay_next_append_entries(peer.node_id)
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    delayed_appends.sort_by_key(|(peer_id, _)| *peer_id);
    assert_eq!(delayed_appends.len(), (NODE_COUNT - 1) as usize);

    for node in &nodes {
        if node.node_id == initial_leader_id {
            for peer_id in 1..=NODE_COUNT {
                if peer_id != initial_leader_id {
                    node.set_peer_blocked(peer_id, true).unwrap();
                }
            }
        } else {
            node.set_peer_blocked(initial_leader_id, true).unwrap();
        }
    }

    let uncommitted = record_command(
        &root.join(format!("catalog-{initial_leader_id}")),
        2,
        5,
        "NoQuorum",
        44,
    );
    let isolated_leader = nodes[initial_leader_index].clone();
    let isolated_node = isolated_leader.node.clone();
    let mut no_quorum_write = isolated_leader
        .runtime
        .spawn(async move { isolated_node.client_write(uncommitted).await });
    for (_, delayed_append) in &delayed_appends {
        delayed_append
            .wait_until_paused(Duration::from_secs(10))
            .unwrap();
    }
    let no_quorum_result = nodes[initial_leader_index].runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), &mut no_quorum_write).await
    });
    assert!(
        !matches!(no_quorum_result, Ok(Ok(Ok(_)))),
        "isolated leader reported a write committed without a quorum"
    );
    no_quorum_write.abort();
    let _ = nodes[initial_leader_index]
        .runtime
        .block_on(no_quorum_write);
    drop(isolated_leader);
    let isolated_catalog_root = root.join(format!("catalog-{initial_leader_id}"));
    let isolated_catalog = Catalog::from_path(&isolated_catalog_root).unwrap();
    assert_eq!(isolated_catalog.transaction_id().unwrap(), Some(2));
    assert_eq!(
        isolated_catalog
            .open_table("users")
            .unwrap()
            .records()
            .len(),
        4
    );
    let (status, body) = catalog_http_request(&nodes[initial_leader_index], &isolated_catalog_root);
    assert_eq!(
        status, 503,
        "an isolated former leader served a catalog read without quorum: {body}"
    );
    assert_eq!(body["error"]["code"], "raft_unavailable");

    let election_deadline = Instant::now() + Duration::from_secs(20);
    let (transition_leader_index, transition_leader_id) = loop {
        if let Some((index, leader)) = majority
            .iter()
            .enumerate()
            .find(|(_, node)| node.node.metrics().borrow().current_leader == Some(node.node_id))
        {
            break (index, leader.node_id);
        }
        assert!(
            Instant::now() < election_deadline,
            "the remaining quorum did not elect a replacement leader"
        );
        thread::sleep(Duration::from_millis(100));
    };
    for source in &majority {
        for target in &majority {
            if source.node_id != target.node_id {
                source.set_peer_blocked(target.node_id, true).unwrap();
            }
        }
    }

    let transition_catalog_root = root.join(format!("catalog-{transition_leader_id}"));
    let (status, body) =
        catalog_http_request(&majority[transition_leader_index], &transition_catalog_root);
    assert_eq!(
        status, 503,
        "leader served a catalog read after losing quorum during the transition: {body}"
    );
    assert_eq!(body["error"]["code"], "raft_unavailable");

    for source in &majority {
        for target in &majority {
            if source.node_id != target.node_id {
                source.set_peer_blocked(target.node_id, false).unwrap();
            }
        }
    }
    let replacement_index = current_leader_index(&majority, Duration::from_secs(20));
    let replacement_id = majority[replacement_index].node_id;
    let replacement_catalog_root = root.join(format!("catalog-{replacement_id}"));
    let (status, body) =
        catalog_http_request(&majority[replacement_index], &replacement_catalog_root);
    assert_eq!(
        status, 200,
        "replacement leader did not serve a catalog read: {body}"
    );

    assert_ne!(replacement_id, initial_leader_id);
    let replacement_command = record_command(
        &root.join(format!("catalog-{replacement_id}")),
        2,
        6,
        "Failover",
        45,
    );
    assert_eq!(
        commit(&majority[replacement_index], replacement_command),
        RaftResponseResult::Applied { transaction_id: 3 }
    );
    wait_for_transaction(&majority, &root, 3, Duration::from_secs(15));
    // Deliver the held stale requests in a deterministic order after the new quorum commits.
    let mut released_peers = BTreeSet::new();
    for delay_index in release_order {
        let (peer_id, delayed_append) = &mut delayed_appends[delay_index];
        assert!(
            released_peers.insert(*peer_id),
            "peer released more than once"
        );
        delayed_append.release();
        delayed_append
            .wait_for_completion(Duration::from_secs(10))
            .unwrap();
        let catalog = Catalog::from_path(root.join(format!("catalog-{peer_id}"))).unwrap();
        assert_eq!(
            catalog.transaction_id().unwrap(),
            Some(3),
            "delayed AppendEntries rolled back peer {peer_id}'s committed catalog"
        );
        assert_eq!(
            catalog
                .open_table("users")
                .unwrap()
                .active_record(5)
                .unwrap()
                .values["NAME"],
            "Failover",
            "delayed AppendEntries replaced peer {peer_id}'s newer committed record"
        );
    }
    assert_eq!(released_peers.len(), (NODE_COUNT - 1) as usize);

    for source in &nodes {
        for target_id in 1..=NODE_COUNT {
            if source.node_id != target_id {
                source.set_peer_blocked(target_id, false).unwrap();
            }
        }
    }
    wait_for_transaction(&nodes, &root, 3, Duration::from_secs(20));
    current_leader_index(&nodes, Duration::from_secs(20));

    for node in &nodes {
        let catalog = Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
        assert_eq!(catalog.transaction_id().unwrap(), Some(3));
        let table = catalog.open_table("users").unwrap();
        assert_eq!(table.records().len(), 5);
        assert_eq!(table.active_record(5).unwrap().values["NAME"], "Failover");
        assert!(
            !table
                .active_records()
                .any(|record| record.values["NAME"] == "NoQuorum")
        );
    }

    drop(listeners.remove(initial_leader_index));
    let stopped = nodes.remove(initial_leader_index);
    stopped.shutdown().unwrap();
    drop(stopped);
    let restarted = RaftRuntime::start(
        &root.join(format!("catalog-{initial_leader_id}")),
        configs[initial_leader_index].clone(),
        Some("ci-token".into()),
    )
    .unwrap();
    let server = restarted
        .bind_peer_listener(&configs[initial_leader_index])
        .unwrap();
    listeners.insert(
        initial_leader_index,
        restarted.spawn_peer_listener(server).unwrap(),
    );
    nodes.insert(initial_leader_index, restarted);

    wait_for_transaction(&nodes, &root, 3, Duration::from_secs(20));
    wait_for_membership(
        &nodes,
        &expected_voters,
        &BTreeSet::new(),
        Duration::from_secs(20),
    );
    if verify_successive_delays {
        append_delivery::delay_successive_appends_to_single_peer(&nodes, &root);
        append_delivery::delay_batched_catchup_to_single_peer(&nodes, &root);
    }

    let expected_transaction_id = if verify_successive_delays { 7 } else { 3 };
    let expected_record_count = if verify_successive_delays { 9 } else { 5 };
    wait_for_transaction(
        &nodes,
        &root,
        expected_transaction_id,
        Duration::from_secs(20),
    );
    wait_for_membership(
        &nodes,
        &expected_voters,
        &BTreeSet::new(),
        Duration::from_secs(20),
    );
    current_leader_index(&nodes, Duration::from_secs(20));
    for node in &nodes {
        let catalog = Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
        assert_eq!(
            catalog.transaction_id().unwrap(),
            Some(expected_transaction_id)
        );
        let table = catalog.open_table("users").unwrap();
        assert_eq!(table.records().len(), expected_record_count);
        assert_eq!(table.active_record(5).unwrap().values["NAME"], "Failover");
        if verify_successive_delays {
            for name in ["DelayedOne", "DelayedTwo", "BatchedOne", "BatchedTwo"] {
                assert!(
                    table
                        .active_records()
                        .any(|record| record.values["NAME"] == name),
                    "node {} did not recover record {name}",
                    node.node_id
                );
            }
        }
        assert!(
            !table
                .active_records()
                .any(|record| record.values["NAME"] == "NoQuorum")
        );
    }

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

fn catalog_http_request(raft: &RaftRuntime, catalog_root: &Path) -> (u16, Value) {
    let http_server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let authority = http_server.server_addr().to_ip().unwrap().to_string();
    let client = thread::spawn(move || {
        peer_request(
            &format!("http://{authority}"),
            "GET",
            "/catalog",
            None,
            "ci-token",
        )
    });
    let request = http_server.recv().unwrap();
    let mut catalog = Catalog::from_path(catalog_root).unwrap();
    super::super::catalog::handle_raft_request(request, &mut catalog, raft);
    response_json(&client.join().unwrap())
}

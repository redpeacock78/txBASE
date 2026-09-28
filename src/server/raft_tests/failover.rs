use super::*;

#[test]
fn partitioned_leader_cannot_commit_and_rejoins_after_majority_failover_and_restart() {
    let root = temporary_cluster();
    let addresses = (0..3).map(|_| free_address()).collect::<Vec<_>>();
    let members = addresses
        .iter()
        .enumerate()
        .map(|(index, address)| (index as u64 + 1, format!("http://{address}")))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = Vec::new();
    let mut listeners = Vec::new();
    let mut configs = Vec::new();

    for node_id in 1..=3 {
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
        };
        let node =
            RaftRuntime::start(&catalog_root, config.clone(), Some("ci-token".into())).unwrap();
        let server = node.bind_peer_listener(&config).unwrap();
        listeners.push(node.spawn_peer_listener(server).unwrap());
        configs.push(config);
        nodes.push(node);
    }

    let initial_leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    wait_for_membership(
        &nodes,
        &BTreeSet::from([1, 2, 3]),
        &BTreeSet::new(),
        Duration::from_secs(20),
    );
    let initial_leader_id = nodes[initial_leader_index].node_id;
    let initial_command = record_command(
        &root.join(format!("catalog-{initial_leader_id}")),
        1,
        4,
        "BeforePartition",
        43,
    );
    assert_eq!(
        commit(&nodes[initial_leader_index], initial_command),
        RaftResponseResult::Applied { transaction_id: 2 }
    );
    wait_for_transaction(&nodes, &root, 2, Duration::from_secs(15));

    for node in &nodes {
        if node.node_id == initial_leader_id {
            for peer_id in 1..=3 {
                if peer_id != initial_leader_id {
                    node.set_peer_blocked(peer_id, true).unwrap();
                }
            }
        } else {
            node.set_peer_blocked(initial_leader_id, true).unwrap();
        }
    }

    let majority = nodes
        .iter()
        .filter(|node| node.node_id != initial_leader_id)
        .cloned()
        .collect::<Vec<_>>();
    let replacement_index = current_leader_index(&majority, Duration::from_secs(20));
    let replacement_id = majority[replacement_index].node_id;
    assert_ne!(replacement_id, initial_leader_id);

    let uncommitted = record_command(
        &root.join(format!("catalog-{initial_leader_id}")),
        2,
        5,
        "NoQuorum",
        44,
    );
    let no_quorum_result = nodes[initial_leader_index].runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(3),
            nodes[initial_leader_index].node.client_write(uncommitted),
        )
        .await
    });
    assert!(
        !matches!(no_quorum_result, Ok(Ok(_))),
        "isolated leader reported a write committed without a quorum"
    );
    let isolated_catalog =
        Catalog::from_path(root.join(format!("catalog-{initial_leader_id}"))).unwrap();
    assert_eq!(isolated_catalog.transaction_id().unwrap(), Some(2));
    assert_eq!(
        isolated_catalog
            .open_table("users")
            .unwrap()
            .records()
            .len(),
        2
    );

    let replacement_command = record_command(
        &root.join(format!("catalog-{replacement_id}")),
        2,
        6,
        "AfterFailover",
        45,
    );
    assert_eq!(
        commit(&majority[replacement_index], replacement_command),
        RaftResponseResult::Applied { transaction_id: 3 }
    );
    wait_for_transaction(&majority, &root, 3, Duration::from_secs(15));

    for source in &nodes {
        for target_id in 1..=3 {
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
        assert_eq!(catalog.open_table("users").unwrap().records().len(), 3);
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
        &BTreeSet::from([1, 2, 3]),
        &BTreeSet::new(),
        Duration::from_secs(20),
    );
    current_leader_index(&nodes, Duration::from_secs(20));
    for node in &nodes {
        let catalog = Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
        assert_eq!(catalog.transaction_id().unwrap(), Some(3));
        assert_eq!(catalog.open_table("users").unwrap().records().len(), 3);
    }

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

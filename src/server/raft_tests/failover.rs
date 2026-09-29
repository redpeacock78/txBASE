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

    wait_for_membership(
        &nodes,
        &BTreeSet::from([1, 2, 3]),
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
    let delayed_peer_id = nodes
        .iter()
        .find(|node| node.node_id != initial_leader_id)
        .unwrap()
        .node_id;
    let mut delayed_append = nodes[initial_leader_index]
        .delay_next_append_entries(delayed_peer_id)
        .unwrap();

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
    delayed_append
        .wait_until_paused(Duration::from_secs(10))
        .unwrap();
    let no_quorum_result = nodes[initial_leader_index].runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), &mut no_quorum_write).await
    });
    assert!(
        !matches!(no_quorum_result, Ok(Ok(Ok(_)))),
        "isolated leader reported a write committed without a quorum"
    );
    no_quorum_write.abort();
    let isolated_catalog =
        Catalog::from_path(root.join(format!("catalog-{initial_leader_id}"))).unwrap();
    assert_eq!(isolated_catalog.transaction_id().unwrap(), Some(2));
    assert_eq!(
        isolated_catalog
            .open_table("users")
            .unwrap()
            .records()
            .len(),
        4
    );

    let replacement_index = current_leader_index(&majority, Duration::from_secs(20));
    let replacement_id = majority[replacement_index].node_id;
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
    delayed_append.release();
    delayed_append
        .wait_for_completion(Duration::from_secs(10))
        .unwrap();

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
        let table = catalog.open_table("users").unwrap();
        assert_eq!(table.records().len(), 5);
        assert_eq!(table.active_record(6).unwrap().values["NAME"], "Failover");
        assert!(table.active_record(5).is_none());
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
        let table = catalog.open_table("users").unwrap();
        assert_eq!(table.records().len(), 5);
        assert_eq!(table.active_record(6).unwrap().values["NAME"], "Failover");
        assert!(table.active_record(5).is_none());
    }

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

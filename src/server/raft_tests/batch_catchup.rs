use super::*;

#[test]
fn isolated_voter_receives_two_log_entries_in_one_append_request() {
    const NODE_COUNT: u64 = 3;
    const MAX_PAYLOAD_ENTRIES: u64 = 2;

    let root = temporary_cluster();
    let addresses = free_addresses(NODE_COUNT as usize);
    let members = addresses
        .iter()
        .enumerate()
        .map(|(index, address)| (index as u64 + 1, format!("http://{address}")))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = Vec::new();
    let mut listeners = Vec::new();

    for node_id in 1..=NODE_COUNT {
        let catalog_root = root.join(format!("catalog-{node_id}"));
        prepare_catalog(&catalog_root);
        let config = CatalogRaftConfig {
            node_id,
            cluster_id: "ci-raft-batch-catchup".into(),
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
        let node = RaftRuntime::start_with_max_payload_entries(
            &catalog_root,
            config.clone(),
            Some("ci-token".into()),
            MAX_PAYLOAD_ENTRIES,
        )
        .unwrap();
        let server = node.bind_peer_listener(&config).unwrap();
        listeners.push(node.spawn_peer_listener(server).unwrap());
        nodes.push(node);
    }

    let expected_voters = (1..=NODE_COUNT).collect::<BTreeSet<_>>();
    wait_for_membership(
        &nodes,
        &expected_voters,
        &BTreeSet::new(),
        Duration::from_secs(20),
    );
    let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let leader_id = nodes[leader_index].node_id;
    let leader_root = root.join(format!("catalog-{leader_id}"));
    assert_eq!(
        commit(
            &nodes[leader_index],
            record_command(&leader_root, 1, 4, "BatchBaseline", 43)
        ),
        RaftResponseResult::Applied { transaction_id: 2 }
    );
    wait_for_transaction(&nodes, &root, 2, Duration::from_secs(15));

    let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let leader = &nodes[leader_index];
    let target = nodes
        .iter()
        .find(|node| node.node_id != leader.node_id)
        .unwrap();
    let target_id = target.node_id;
    let target_root = root.join(format!("catalog-{target_id}"));
    let leader_log_index = leader
        .node
        .metrics()
        .borrow()
        .last_log_index
        .expect("the leader must have a committed log before isolation");
    assert_eq!(
        target.node.metrics().borrow().last_log_index,
        Some(leader_log_index),
        "the target must be caught up before isolation"
    );
    let baseline_transaction_id = Catalog::from_path(&target_root)
        .unwrap()
        .transaction_id()
        .unwrap()
        .unwrap();
    let baseline_record_count = Catalog::from_path(&target_root)
        .unwrap()
        .open_table("users")
        .unwrap()
        .records()
        .len();

    for source in &nodes {
        if source.node_id != target_id {
            source.set_peer_blocked(target_id, true).unwrap();
        }
    }
    for peer_id in nodes
        .iter()
        .filter(|node| node.node_id != target_id)
        .map(|node| node.node_id)
    {
        target.set_peer_blocked(peer_id, true).unwrap();
    }

    let first_transaction_id = baseline_transaction_id + 1;
    let second_transaction_id = first_transaction_id + 1;
    assert_eq!(
        commit(
            leader,
            record_command(&leader_root, baseline_transaction_id, 50, "BatchOne", 50)
        ),
        RaftResponseResult::Applied {
            transaction_id: first_transaction_id
        }
    );
    assert_eq!(
        commit(
            leader,
            record_command(&leader_root, first_transaction_id, 51, "BatchTwo", 51,)
        ),
        RaftResponseResult::Applied {
            transaction_id: second_transaction_id
        }
    );

    let healthy_nodes = nodes
        .iter()
        .filter(|node| node.node_id != target_id)
        .cloned()
        .collect::<Vec<_>>();
    wait_for_transaction(
        &healthy_nodes,
        &root,
        second_transaction_id,
        Duration::from_secs(15),
    );
    let mut delayed_append = leader
        .delay_append_entries_at(target_id, leader_log_index + 1)
        .unwrap();
    leader.set_peer_blocked(target_id, false).unwrap();
    target.set_peer_blocked(leader.node_id, false).unwrap();

    assert_eq!(
        delayed_append
            .wait_until_paused(Duration::from_secs(10))
            .unwrap(),
        2,
        "the catch-up AppendEntries request must contain both log entries"
    );
    assert_eq!(
        Catalog::from_path(&target_root)
            .unwrap()
            .transaction_id()
            .unwrap(),
        Some(baseline_transaction_id),
        "the paused catch-up request must not update the isolated voter"
    );
    assert_eq!(
        Catalog::from_path(&target_root)
            .unwrap()
            .open_table("users")
            .unwrap()
            .records()
            .len(),
        baseline_record_count,
        "the paused catch-up request must leave the target table unchanged"
    );

    delayed_append.release();
    delayed_append
        .wait_for_completion(Duration::from_secs(10))
        .unwrap();
    wait_for_transaction(
        &nodes,
        &root,
        second_transaction_id,
        Duration::from_secs(20),
    );
    for source in &nodes {
        if source.node_id != target_id {
            source.set_peer_blocked(target_id, false).unwrap();
        }
    }
    for peer_id in nodes
        .iter()
        .filter(|node| node.node_id != target_id)
        .map(|node| node.node_id)
    {
        target.set_peer_blocked(peer_id, false).unwrap();
    }

    for node in &nodes {
        let catalog = Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
        assert_eq!(
            catalog.transaction_id().unwrap(),
            Some(second_transaction_id)
        );
        let table = catalog.open_table("users").unwrap();
        assert_eq!(table.records().len(), baseline_record_count + 2);
        for name in ["BatchOne", "BatchTwo"] {
            assert_eq!(
                table
                    .active_records()
                    .filter(|record| record.values["NAME"] == name)
                    .count(),
                1,
                "node {} must apply {name} exactly once",
                node.node_id
            );
        }
    }

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

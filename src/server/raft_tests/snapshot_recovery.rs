use super::*;

#[test]
fn lagging_voter_catches_up_from_snapshot_after_leader_purges_log() {
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
            cluster_id: "ci-raft-snapshot-recovery".into(),
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
    let lagging_index = (0..nodes.len())
        .find(|index| *index != leader_index)
        .unwrap();
    let lagging_id = nodes[lagging_index].node_id;
    let quorum = nodes
        .iter()
        .filter(|node| node.node_id != lagging_id)
        .cloned()
        .collect::<Vec<_>>();

    for node in &nodes {
        if node.node_id == lagging_id {
            for peer_id in 1..=3 {
                if peer_id != lagging_id {
                    node.set_peer_blocked(peer_id, true).unwrap();
                }
            }
        } else {
            node.set_peer_blocked(lagging_id, true).unwrap();
        }
    }

    for sequence in 1..=4 {
        let command = record_command(
            &root.join(format!("catalog-{leader_id}")),
            sequence,
            sequence as i64 + 3,
            &format!("Snapshot-{sequence}"),
            40 + sequence as i64,
        );
        assert_eq!(
            commit(&nodes[leader_index], command),
            RaftResponseResult::Applied {
                transaction_id: sequence + 1
            }
        );
        wait_for_transaction(&quorum, &root, sequence + 1, Duration::from_secs(15));
    }
    assert_eq!(
        Catalog::from_path(root.join(format!("catalog-{lagging_id}")))
            .unwrap()
            .transaction_id()
            .unwrap(),
        Some(1)
    );

    let lagging_metrics = nodes[lagging_index].node.metrics();
    let lagging_applied_index = lagging_metrics
        .borrow()
        .last_applied
        .as_ref()
        .map(|log_id| log_id.index)
        .unwrap_or(0);
    let leader = &nodes[leader_index];
    leader
        .runtime
        .block_on(leader.node.trigger().snapshot())
        .unwrap();
    let snapshot = leader
        .runtime
        .block_on(leader.node.get_snapshot())
        .unwrap()
        .unwrap();
    let snapshot_log_index = snapshot
        .meta
        .last_log_id
        .as_ref()
        .map(|log_id| log_id.index)
        .unwrap();
    assert!(snapshot_log_index > lagging_applied_index);
    drop(snapshot);

    leader
        .runtime
        .block_on(leader.node.trigger().purge_log(snapshot_log_index))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let metrics = leader.node.metrics();
        if metrics
            .borrow()
            .purged
            .as_ref()
            .is_some_and(|log_id| log_id.index >= snapshot_log_index)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "leader did not purge log index {snapshot_log_index}"
        );
        thread::sleep(Duration::from_millis(50));
    }

    for node in &nodes {
        for peer_id in 1..=3 {
            if peer_id != node.node_id {
                node.set_peer_blocked(peer_id, false).unwrap();
            }
        }
    }
    wait_for_transaction(&nodes, &root, 5, Duration::from_secs(30));

    let lagging_metrics = nodes[lagging_index].node.metrics();
    assert!(
        lagging_metrics
            .borrow()
            .snapshot
            .as_ref()
            .is_some_and(|log_id| log_id.index >= snapshot_log_index),
        "lagging voter did not install the purged log's snapshot"
    );
    wait_for_membership(
        &nodes,
        &BTreeSet::from([1, 2, 3]),
        &BTreeSet::new(),
        Duration::from_secs(15),
    );

    let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let command = record_command(
        &root.join(format!("catalog-{}", nodes[leader_index].node_id)),
        5,
        8,
        "AfterSnapshot",
        45,
    );
    assert_eq!(
        commit(&nodes[leader_index], command),
        RaftResponseResult::Applied { transaction_id: 6 }
    );
    wait_for_transaction(&nodes, &root, 6, Duration::from_secs(15));

    for node in &nodes {
        let catalog = Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
        assert_eq!(catalog.transaction_id().unwrap(), Some(6));
        let table = catalog.open_table("users").unwrap();
        for record_number in 4..=7 {
            assert_eq!(
                table.active_record(record_number).unwrap().values["NAME"],
                format!("Snapshot-{}", record_number - 3)
            );
        }
        assert_eq!(
            table.active_record(8).unwrap().values["NAME"],
            "AfterSnapshot"
        );
    }

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

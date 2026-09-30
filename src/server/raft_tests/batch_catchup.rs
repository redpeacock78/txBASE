use super::*;

#[test]
fn isolated_voter_applies_batched_catchup_payloads_once() {
    const NODE_COUNT: u64 = 3;
    const MAX_PAYLOAD_ENTRIES: u64 = 16;
    const PAYLOAD_SIZES: [u64; 6] = [2, 4, 8, 16, 17, 32];

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
        commit(&nodes, record_command(&leader_root, 1, 4, "Baseline", 43)),
        RaftResponseResult::Applied { transaction_id: 2 }
    );
    wait_for_transaction(&nodes, &root, 2, Duration::from_secs(15));

    let mut next_record_id = 50_i64;
    for payload_size in PAYLOAD_SIZES {
        let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
        let leader = &nodes[leader_index];
        let leader_root = root.join(format!("catalog-{}", leader.node_id));
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

        let healthy_nodes = nodes
            .iter()
            .filter(|node| node.node_id != target_id)
            .cloned()
            .collect::<Vec<_>>();
        let mut names = Vec::with_capacity(payload_size as usize);
        for offset in 0..payload_size {
            let sequence = baseline_transaction_id + offset;
            let record_id = next_record_id + offset as i64;
            let name = format!("Batch{record_id}");
            assert_eq!(
                commit(
                    &healthy_nodes,
                    record_command(&leader_root, sequence, record_id, &name, 50 + record_id,)
                ),
                RaftResponseResult::Applied {
                    transaction_id: sequence + 1
                }
            );
            names.push(name);
        }
        next_record_id += payload_size as i64;

        let final_transaction_id = baseline_transaction_id + payload_size;
        wait_for_transaction(
            &healthy_nodes,
            &root,
            final_transaction_id,
            Duration::from_secs(15),
        );
        let mut delayed_append = leader
            .delay_append_entries_at(target_id, leader_log_index + 1)
            .unwrap();
        let delayed_followup = if payload_size > MAX_PAYLOAD_ENTRIES {
            Some(
                leader
                    .delay_append_entries_at(target_id, leader_log_index + MAX_PAYLOAD_ENTRIES + 1)
                    .unwrap(),
            )
        } else {
            None
        };
        leader.set_peer_blocked(target_id, false).unwrap();
        target.set_peer_blocked(leader.node_id, false).unwrap();

        assert_eq!(
            delayed_append
                .wait_until_paused(Duration::from_secs(10))
                .unwrap(),
            payload_size.min(MAX_PAYLOAD_ENTRIES) as usize,
            "the catch-up AppendEntries request must respect the payload limit"
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
        assert_eq!(
            target.node.metrics().borrow().last_log_index,
            Some(leader_log_index),
            "the paused request must not append entries to the target log"
        );

        delayed_append.release();
        delayed_append
            .wait_for_completion(Duration::from_secs(10))
            .unwrap();
        if let Some(mut delayed_followup) = delayed_followup {
            let followup_entry_count = delayed_followup
                .wait_until_paused(Duration::from_secs(10))
                .unwrap();
            assert!(
                (1..=MAX_PAYLOAD_ENTRIES as usize).contains(&followup_entry_count),
                "each follow-up AppendEntries request must respect the payload limit"
            );
            assert!(
                target
                    .node
                    .metrics()
                    .borrow()
                    .last_log_index
                    .is_some_and(|last_log_index| {
                        last_log_index <= leader_log_index + MAX_PAYLOAD_ENTRIES
                    }),
                "the target log must not advance past the first payload while the follow-up is held"
            );
            let target_table = Catalog::from_path(&target_root)
                .unwrap()
                .open_table("users")
                .unwrap();
            for name in names.iter().skip(MAX_PAYLOAD_ENTRIES as usize) {
                assert_eq!(
                    target_table
                        .active_records()
                        .filter(|record| record.values["NAME"].as_str() == Some(name.as_str()))
                        .count(),
                    0,
                    "the target must not apply a tail record while its AppendEntries request is held"
                );
            }
            delayed_followup.release();
            delayed_followup
                .wait_for_completion(Duration::from_secs(10))
                .unwrap();
        }
        wait_for_transaction(&nodes, &root, final_transaction_id, Duration::from_secs(20));
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
            let catalog =
                Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
            assert_eq!(
                catalog.transaction_id().unwrap(),
                Some(final_transaction_id)
            );
            let table = catalog.open_table("users").unwrap();
            assert_eq!(
                table.records().len(),
                baseline_record_count + payload_size as usize
            );
            for name in &names {
                assert_eq!(
                    table
                        .active_records()
                        .filter(|record| record.values["NAME"].as_str() == Some(name.as_str()))
                        .count(),
                    1,
                    "node {} must apply {name} exactly once",
                    node.node_id
                );
            }
        }
    }

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

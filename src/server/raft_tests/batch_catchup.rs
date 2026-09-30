use super::*;

mod round;

#[test]
fn isolated_voter_applies_batched_catchup_payloads_once() {
    const NODE_COUNT: u64 = 3;
    const MAX_PAYLOAD_ENTRIES: u64 = 16;
    const PAYLOAD_SIZES: [u64; 7] = [2, 4, 8, 16, 17, 32, 65];

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
        round::run_isolated_voter_catchup_round(
            &nodes,
            &root,
            payload_size,
            &mut next_record_id,
            MAX_PAYLOAD_ENTRIES,
        );
    }

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

use super::*;

#[test]
fn three_nodes_commit_and_change_authenticated_membership_over_peer_rpc() {
    let root = temporary_cluster();
    let addresses = (0..3).map(|_| free_address()).collect::<Vec<_>>();
    let members = addresses
        .iter()
        .enumerate()
        .map(|(index, address)| (index as u64 + 1, format!("http://{address}")))
        .collect::<BTreeMap<_, _>>();
    let voters = members
        .iter()
        .filter(|(node_id, _)| **node_id <= 2)
        .map(|(node_id, address)| (*node_id, address.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = Vec::new();
    let mut listeners = Vec::new();

    for node_id in 1..=2 {
        let catalog_root = root.join(format!("catalog-{node_id}"));
        prepare_catalog(&catalog_root);
        let config = CatalogRaftConfig {
            node_id,
            cluster_id: "ci-raft-cluster".into(),
            node_directory: root.join(format!("node-{node_id}")),
            peer_bind: addresses[(node_id - 1) as usize].clone(),
            peer_advertise: members[&node_id].clone(),
            initial_members: voters.clone(),
            bootstrap: node_id == 1,
            initialize_catalog: node_id != 1,
            tls_certificate: None,
            tls_private_key: None,
        };
        let raft =
            RaftRuntime::start(&catalog_root, config.clone(), Some("ci-token".into())).unwrap();
        let server = raft.bind_peer_listener(&config).unwrap();
        listeners.push(raft.spawn_peer_listener(server).unwrap());
        nodes.push(raft);
    }

    let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let leader_id = nodes[leader_index].node_id;
    let catalog_root = root.join(format!("catalog-{leader_id}"));
    let (status, initial_membership) = get_membership(&members[&leader_id], "ci-token");
    assert_eq!(status, 200);
    assert_eq!(
        initial_membership["effective_voter_configs"],
        json!([[1, 2]])
    );
    assert_eq!(initial_membership["voter_ids"], json!([1, 2]));
    assert_eq!(initial_membership["learner_ids"], json!([]));
    assert_eq!(get_membership(&members[&leader_id], "").0, 401);

    let command = record_command(&catalog_root, 1, 4, "Quorum", 43);
    assert_eq!(
        commit(&nodes[leader_index], command.clone()),
        RaftResponseResult::Applied { transaction_id: 2 }
    );
    assert_eq!(
        commit(&nodes[leader_index], command),
        RaftResponseResult::Applied { transaction_id: 2 }
    );

    let learner_id = 3;
    let learner_url = members[&learner_id].clone();
    let learner_config = CatalogRaftConfig {
        node_id: learner_id,
        cluster_id: "ci-raft-cluster".into(),
        node_directory: root.join(format!("node-{learner_id}")),
        peer_bind: addresses[(learner_id - 1) as usize].clone(),
        peer_advertise: learner_url.clone(),
        initial_members: BTreeMap::from([
            (leader_id, members[&leader_id].clone()),
            (learner_id, learner_url.clone()),
        ]),
        bootstrap: false,
        initialize_catalog: true,
        tls_certificate: None,
        tls_private_key: None,
    };
    let learner_catalog_root = root.join(format!("catalog-{learner_id}"));
    prepare_catalog(&learner_catalog_root);
    let learner = RaftRuntime::start(
        &learner_catalog_root,
        learner_config.clone(),
        Some("ci-token".into()),
    )
    .unwrap();
    let learner_server = learner.bind_peer_listener(&learner_config).unwrap();
    listeners.push(learner.spawn_peer_listener(learner_server).unwrap());
    let response = post_add_learner(&members[&leader_id], &learner_url);
    assert!(response.starts_with("HTTP/1.1 202"), "{response}");
    let response_body = response.split_once("\r\n\r\n").unwrap().1;
    let response: serde_json::Value = serde_json::from_str(response_body).unwrap();
    assert_eq!(response["node_id"], learner_id);
    assert_eq!(response["status"], "learner_sync_started");
    nodes.push(learner);

    wait_for_membership(
        &nodes,
        &BTreeSet::from([1, 2]),
        &BTreeSet::from([3]),
        Duration::from_secs(15),
    );

    let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let leader_id = nodes[leader_index].node_id;
    let (status, learner_membership) = get_membership(&members[&leader_id], "ci-token");
    assert_eq!(status, 200);
    assert_eq!(learner_membership["voter_ids"], json!([1, 2]));
    assert_eq!(learner_membership["learner_ids"], json!([3]));
    let membership_index = learner_membership["effective_membership_log_index"]
        .as_u64()
        .unwrap();

    let duplicate_ids = membership_change_body(membership_index, &[1, 1], &[1, 2, 3]);
    assert_eq!(post_membership(&members[&leader_id], duplicate_ids).0, 400);
    let empty_voters = membership_change_body(membership_index, &[1, 2], &[]);
    assert_eq!(post_membership(&members[&leader_id], empty_voters).0, 400);
    let stale = membership_change_body(membership_index + 1, &[1, 2], &[1, 2, 3]);
    assert_eq!(post_membership(&members[&leader_id], stale).0, 409);

    let follower_id = nodes
        .iter()
        .find(|node| node.node_id != leader_id && node.node_id <= 2)
        .unwrap()
        .node_id;
    let follower_request = membership_change_body(membership_index, &[1, 2], &[1, 2, 3]);
    assert_eq!(
        post_membership(&members[&follower_id], follower_request).0,
        409
    );

    let promote = membership_change_body(membership_index, &[1, 2], &[1, 2, 3]);
    let (status, response) = post_membership(&members[&leader_id], promote.clone());
    assert_eq!(status, 202, "{response}");
    assert_eq!(response["status"], "membership_change_started");
    wait_for_membership(
        &nodes,
        &BTreeSet::from([1, 2, 3]),
        &BTreeSet::new(),
        Duration::from_secs(20),
    );
    wait_for_transaction(&nodes, &root, 2, Duration::from_secs(15));
    let repeated = post_add_learner(&members[&leader_id], &learner_url);
    let (status, response) = response_json(&repeated);
    assert_eq!(status, 200, "{response}");
    assert_eq!(response["status"], "already_member");
    for node in &nodes {
        let catalog = Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
        assert_eq!(catalog.open_table("users").unwrap().records().len(), 4);
    }
    let promoted_leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let promoted_leader_id = nodes[promoted_leader_index].node_id;
    let (status, promoted_membership) = get_membership(&members[&promoted_leader_id], "ci-token");
    assert_eq!(status, 200);
    assert_eq!(
        promoted_membership["effective_voter_configs"],
        json!([[1, 2, 3]])
    );
    assert_eq!(promoted_membership["learner_ids"], json!([]));
    let (status, response) = post_membership(&members[&promoted_leader_id], promote);
    assert_eq!(status, 200, "{response}");
    assert_eq!(response["status"], "already_current");

    let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let command = record_command(
        &root.join(format!("catalog-{}", nodes[leader_index].node_id)),
        2,
        5,
        "Promoted",
        44,
    );
    assert_eq!(
        commit(&nodes[leader_index], command),
        RaftResponseResult::Applied { transaction_id: 3 }
    );
    wait_for_transaction(&nodes, &root, 3, Duration::from_secs(15));

    let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let leader_id = nodes[leader_index].node_id;
    let (status, promoted_membership) = get_membership(&members[&leader_id], "ci-token");
    assert_eq!(status, 200);
    let membership_index = promoted_membership["effective_membership_log_index"]
        .as_u64()
        .unwrap();
    let demote = membership_change_body(membership_index, &[1, 2, 3], &[1, 2]);
    let (status, response) = post_membership(&members[&leader_id], demote);
    assert_eq!(status, 202, "{response}");
    assert_eq!(response["status"], "membership_change_started");
    wait_for_membership(
        &nodes,
        &BTreeSet::from([1, 2]),
        &BTreeSet::from([3]),
        Duration::from_secs(20),
    );

    let demoted_index = nodes.iter().position(|node| node.node_id == 3).unwrap();
    drop(listeners.remove(demoted_index));
    nodes.remove(demoted_index).shutdown().unwrap();
    let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let command = record_command(
        &root.join(format!("catalog-{}", nodes[leader_index].node_id)),
        3,
        6,
        "RetainedLearnerOffline",
        45,
    );
    assert_eq!(
        commit(&nodes[leader_index], command),
        RaftResponseResult::Applied { transaction_id: 4 }
    );
    wait_for_transaction(&nodes, &root, 4, Duration::from_secs(15));
    for node in &nodes {
        let catalog = Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
        assert_eq!(catalog.open_table("users").unwrap().records().len(), 6);
    }

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

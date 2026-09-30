use super::*;
use crate::replication::raft::{
    RaftAddLearnerRequest, RaftMembershipChangeRequest, RaftMembershipChangeStatus,
    RaftMembershipHttpClient,
};

#[test]
fn empty_catalog_joins_an_empty_genesis_cluster() {
    let root = temporary_cluster();
    let addresses = free_addresses(2);
    let leader_url = format!("http://{}", addresses[0]);
    let learner_url = format!("http://{}", addresses[1]);
    let mut nodes = Vec::new();
    let mut listeners = Vec::new();

    for node_id in 1..=2 {
        let catalog_root = root.join(format!("catalog-{node_id}"));
        fs::create_dir(&catalog_root).unwrap();
        let initial_members = if node_id == 1 {
            BTreeMap::from([(1, leader_url.clone())])
        } else {
            BTreeMap::from([(1, leader_url.clone()), (2, learner_url.clone())])
        };
        let peer_address = addresses[(node_id - 1) as usize].clone();
        let config = CatalogRaftConfig {
            node_id,
            cluster_id: "ci-raft-cluster".into(),
            node_directory: root.join(format!("node-{node_id}")),
            peer_bind: peer_address.clone(),
            peer_advertise: format!("http://{peer_address}"),
            initial_members,
            bootstrap: node_id == 1,
            initialize_catalog: false,
            tls_certificate: None,
            tls_private_key: None,
            tls_client_ca: None,
        };
        let raft =
            RaftRuntime::start(&catalog_root, config.clone(), Some("ci-token".into())).unwrap();
        let server = raft.bind_peer_listener(&config).unwrap();
        listeners.push(raft.spawn_peer_listener(server).unwrap());
        nodes.push(raft);
    }

    let leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    assert_eq!(nodes[leader_index].node_id, 1);
    let client = RaftMembershipHttpClient::new(&leader_url, "ci-token").unwrap();
    let request = RaftAddLearnerRequest::new("ci-raft-cluster", 2, learner_url).unwrap();
    let response = client.add_learner(&request).unwrap();
    assert_eq!(response.node_id, 2);
    assert_eq!(
        response.status,
        crate::replication::raft::RaftLearnerAddStatus::LearnerSyncStarted
    );
    wait_for_membership(
        &nodes,
        &BTreeSet::from([1]),
        &BTreeSet::from([2]),
        Duration::from_secs(15),
    );

    for node_id in 1..=2 {
        let catalog = Catalog::from_path(root.join(format!("catalog-{node_id}"))).unwrap();
        assert!(catalog.tables().next().is_none());
        assert_eq!(catalog.transaction_id().unwrap(), None);
    }

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn three_nodes_commit_and_change_authenticated_membership_over_peer_rpc() {
    let root = temporary_cluster();
    let addresses = free_addresses(3);
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
            tls_client_ca: None,
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
    let membership_client =
        RaftMembershipHttpClient::new(&members[&leader_id], "ci-token").unwrap();
    let typed_status = membership_client.status().unwrap();
    assert_eq!(typed_status.cluster_id, "ci-raft-cluster");
    assert_eq!(typed_status.voter_ids, [1, 2]);

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
        tls_client_ca: None,
    };
    let learner_catalog_root = root.join(format!("catalog-{learner_id}"));
    fs::create_dir(&learner_catalog_root).unwrap();
    let learner = RaftRuntime::start(
        &learner_catalog_root,
        learner_config.clone(),
        Some("ci-token".into()),
    )
    .unwrap();
    let learner_server = learner.bind_peer_listener(&learner_config).unwrap();
    listeners.push(learner.spawn_peer_listener(learner_server).unwrap());
    let add_learner =
        RaftAddLearnerRequest::new("ci-raft-cluster", learner_id, learner_url.clone()).unwrap();
    let response = membership_client.add_learner(&add_learner).unwrap();
    assert_eq!(response.node_id, learner_id);
    assert_eq!(
        response.status,
        crate::replication::raft::RaftLearnerAddStatus::LearnerSyncStarted
    );
    nodes.push(learner);

    wait_for_membership(
        &nodes,
        &BTreeSet::from([1, 2]),
        &BTreeSet::from([3]),
        Duration::from_secs(15),
    );
    let leader_catalog = Catalog::from_path(&catalog_root).unwrap();
    let learner_catalog = Catalog::from_path(&learner_catalog_root).unwrap();
    assert_eq!(learner_catalog.transaction_id().unwrap(), Some(2));
    assert_eq!(
        learner_catalog
            .export_snapshot_with_sidecars(&[])
            .unwrap()
            .catalog_snapshot,
        leader_catalog
            .export_snapshot_with_sidecars(&[])
            .unwrap()
            .catalog_snapshot
    );
    assert_eq!(
        crate::replication::raft::raft_genesis_fingerprint(&learner_catalog).unwrap(),
        crate::replication::raft::raft_genesis_fingerprint(&leader_catalog).unwrap()
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

    let promotion_client = RaftMembershipHttpClient::new(&members[&leader_id], "ci-token").unwrap();
    let promote = RaftMembershipChangeRequest::new(
        "ci-raft-cluster",
        membership_index,
        vec![1, 2],
        vec![1, 2, 3],
    )
    .unwrap();
    let response = promotion_client.change_membership(&promote).unwrap();
    assert_eq!(
        response.status,
        RaftMembershipChangeStatus::MembershipChangeStarted
    );
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
    let promoted_client =
        RaftMembershipHttpClient::new(&members[&promoted_leader_id], "ci-token").unwrap();
    let response = promoted_client.change_membership(&promote).unwrap();
    assert_eq!(response.status, RaftMembershipChangeStatus::AlreadyCurrent);

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
    let demote = RaftMembershipChangeRequest::new(
        "ci-raft-cluster",
        membership_index,
        vec![1, 2, 3],
        vec![1, 2],
    )
    .unwrap();
    let demotion_client = RaftMembershipHttpClient::new(&members[&leader_id], "ci-token").unwrap();
    let response = demotion_client.change_membership(&demote).unwrap();
    assert_eq!(
        response.status,
        RaftMembershipChangeStatus::MembershipChangeStarted
    );
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
        "Offline",
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

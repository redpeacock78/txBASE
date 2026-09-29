use super::*;
use crate::replication::raft::{
    RaftMembershipChangeRequest, RaftMembershipChangeStatus, RaftMembershipHttpClient,
};

fn membership_configs(node: &RaftRuntime) -> Vec<BTreeSet<u64>> {
    let metrics = node.node.metrics();
    let metrics = metrics.borrow();
    metrics
        .membership_config
        .membership()
        .get_joint_config()
        .clone()
}

fn wait_for_joint_membership(
    nodes: &[RaftRuntime],
    old_voters: &BTreeSet<u64>,
    new_voters: &BTreeSet<u64>,
    timeout: Duration,
) {
    let expected = BTreeSet::from([old_voters.clone(), new_voters.clone()]);
    let deadline = Instant::now() + timeout;
    loop {
        if nodes.iter().all(|node| {
            membership_configs(node)
                .into_iter()
                .collect::<BTreeSet<_>>()
                == expected
        }) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Raft membership did not enter joint config from {old_voters:?} to {new_voters:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn interrupted_joint_membership_resumes_after_leader_restart() {
    let root = temporary_cluster();
    let addresses = (0..3).map(|_| free_address()).collect::<Vec<_>>();
    let members = addresses
        .iter()
        .enumerate()
        .map(|(index, address)| (index as u64 + 1, format!("http://{address}")))
        .collect::<BTreeMap<_, _>>();
    let old_voters = members.keys().copied().collect::<BTreeSet<_>>();
    let mut configs = Vec::new();
    let mut nodes = Vec::new();
    let mut listeners = Vec::new();

    for node_id in 1..=3 {
        let catalog_root = root.join(format!("catalog-{node_id}"));
        fs::create_dir(&catalog_root).unwrap();
        let config = CatalogRaftConfig {
            node_id,
            cluster_id: "ci-raft-recovery".into(),
            node_directory: root.join(format!("node-{node_id}")),
            peer_bind: addresses[(node_id - 1) as usize].clone(),
            peer_advertise: members[&node_id].clone(),
            initial_members: members.clone(),
            bootstrap: node_id == 1,
            initialize_catalog: false,
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
        &old_voters,
        &BTreeSet::new(),
        Duration::from_secs(20),
    );
    let old_leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let old_leader_id = nodes[old_leader_index].node_id;
    let new_voters = old_voters
        .iter()
        .copied()
        .filter(|node_id| *node_id != old_leader_id)
        .collect::<BTreeSet<_>>();
    let new_voter_ids = new_voters.iter().copied().collect::<Vec<_>>();
    let old_voter_ids = old_voters.iter().copied().collect::<Vec<_>>();
    let (status, membership) = get_membership(&members[&old_leader_id], "ci-token");
    assert_eq!(status, 200);
    let membership_index = membership["effective_membership_log_index"]
        .as_u64()
        .unwrap();
    let request = RaftMembershipChangeRequest::new(
        "ci-raft-recovery",
        membership_index,
        old_voter_ids,
        new_voter_ids.clone(),
    )
    .unwrap();
    let membership_client =
        RaftMembershipHttpClient::new(&members[&old_leader_id], "ci-token").unwrap();

    let mut delayed_appends = new_voters
        .iter()
        .map(|peer_id| {
            nodes[old_leader_index]
                .delay_next_uniform_membership_append(*peer_id)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let response = membership_client.change_membership(&request).unwrap();
    assert_eq!(
        response.status,
        RaftMembershipChangeStatus::MembershipChangeStarted
    );
    for delay in &delayed_appends {
        delay.wait_until_paused(Duration::from_secs(20)).unwrap();
    }

    let new_voter_nodes = nodes
        .iter()
        .filter(|node| new_voters.contains(&node.node_id))
        .cloned()
        .collect::<Vec<_>>();
    wait_for_joint_membership(
        &new_voter_nodes,
        &old_voters,
        &new_voters,
        Duration::from_secs(20),
    );
    drop(new_voter_nodes);
    for peer_id in &new_voters {
        nodes[old_leader_index]
            .set_peer_blocked(*peer_id, true)
            .unwrap();
    }
    for delay in &mut delayed_appends {
        delay.cancel();
    }
    for delay in &delayed_appends {
        assert!(delay.wait_for_completion(Duration::from_secs(5)).is_err());
    }

    let stopped_config = configs[old_leader_index].clone();
    drop(listeners.remove(old_leader_index));
    let stopped = nodes.remove(old_leader_index);
    stopped.shutdown().unwrap();
    drop(stopped);

    let recovered_leader_index = current_leader_index(&nodes, Duration::from_secs(20));
    let recovered_leader_id = nodes[recovered_leader_index].node_id;
    assert!(new_voters.contains(&recovered_leader_id));
    assert_eq!(
        membership_configs(&nodes[recovered_leader_index])
            .into_iter()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([old_voters.clone(), new_voters.clone()])
    );
    let recovery_client =
        RaftMembershipHttpClient::new(&members[&recovered_leader_id], "ci-token").unwrap();
    let response = recovery_client.change_membership(&request).unwrap();
    assert_eq!(
        response.status,
        RaftMembershipChangeStatus::MembershipChangeResumed
    );

    let removed_learners = BTreeSet::from([old_leader_id]);
    wait_for_membership(
        &nodes,
        &new_voters,
        &removed_learners,
        Duration::from_secs(20),
    );
    let restarted = RaftRuntime::start(
        &root.join(format!("catalog-{old_leader_id}")),
        stopped_config.clone(),
        Some("ci-token".into()),
    )
    .unwrap();
    let server = restarted.bind_peer_listener(&stopped_config).unwrap();
    listeners.insert(
        old_leader_index,
        restarted.spawn_peer_listener(server).unwrap(),
    );
    nodes.insert(old_leader_index, restarted);
    wait_for_membership(
        &nodes,
        &new_voters,
        &removed_learners,
        Duration::from_secs(20),
    );

    for node_id in old_voters {
        let (status, membership) = get_membership(&members[&node_id], "ci-token");
        assert_eq!(status, 200);
        assert_eq!(
            membership["effective_voter_configs"],
            json!([new_voter_ids])
        );
        assert_eq!(membership["learner_ids"], json!([old_leader_id]));
    }

    drop(listeners);
    for node in &nodes {
        node.shutdown().unwrap();
    }
    drop(nodes);
    fs::remove_dir_all(root).unwrap();
}

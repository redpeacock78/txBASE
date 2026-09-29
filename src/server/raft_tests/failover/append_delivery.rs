use super::*;

pub(super) fn delay_successive_appends_to_single_peer(nodes: &[RaftRuntime], root: &Path) {
    let leader_index = current_leader_index(nodes, Duration::from_secs(20));
    let leader = &nodes[leader_index];
    let target_id = nodes
        .iter()
        .find(|node| node.node_id != leader.node_id)
        .unwrap()
        .node_id;
    let all_voters = nodes
        .iter()
        .map(|node| node.node_id)
        .collect::<BTreeSet<_>>();
    let test_voters = BTreeSet::from([leader.node_id]);
    let learners = all_voters
        .iter()
        .copied()
        .filter(|node_id| !test_voters.contains(node_id))
        .collect::<BTreeSet<_>>();
    leader
        .runtime
        .block_on(leader.node.change_membership(test_voters.clone(), true))
        .unwrap();
    wait_for_membership(nodes, &test_voters, &learners, Duration::from_secs(20));

    let leader_index = current_leader_index(nodes, Duration::from_secs(20));
    let leader = &nodes[leader_index];
    let first_log_index = leader
        .node
        .metrics()
        .borrow()
        .last_log_index
        .expect("the leader must have a log before delayed appends");
    for source in nodes
        .iter()
        .filter(|node| node.node_id != leader.node_id && node.node_id != target_id)
    {
        source.set_peer_blocked(target_id, true).unwrap();
    }
    let mut first_delay = leader
        .delay_append_entries_at(target_id, first_log_index + 1)
        .unwrap();

    let first_command = record_command(
        &root.join(format!("catalog-{}", leader.node_id)),
        3,
        7,
        "DelayedOne",
        46,
    );
    assert_eq!(
        commit(leader, first_command),
        RaftResponseResult::Applied { transaction_id: 4 }
    );
    first_delay
        .wait_until_paused(Duration::from_secs(10))
        .unwrap();

    let healthy_nodes = nodes
        .iter()
        .filter(|node| node.node_id != target_id)
        .cloned()
        .collect::<Vec<_>>();
    wait_for_transaction(&healthy_nodes, root, 4, Duration::from_secs(15));
    assert_eq!(
        Catalog::from_path(root.join(format!("catalog-{target_id}")))
            .unwrap()
            .transaction_id()
            .unwrap(),
        Some(3),
        "the first AppendEntries request escaped before release"
    );

    let mut second_delay = leader
        .delay_append_entries_at(target_id, first_log_index + 2)
        .unwrap();
    let second_command = record_command(
        &root.join(format!("catalog-{}", leader.node_id)),
        4,
        8,
        "DelayedTwo",
        47,
    );
    assert_eq!(
        commit(leader, second_command),
        RaftResponseResult::Applied { transaction_id: 5 }
    );
    wait_for_transaction(&healthy_nodes, root, 5, Duration::from_secs(15));

    first_delay.release();
    first_delay
        .wait_for_completion(Duration::from_secs(10))
        .unwrap();
    second_delay
        .wait_until_paused(Duration::from_secs(10))
        .unwrap();
    assert_ne!(
        Catalog::from_path(root.join(format!("catalog-{target_id}")))
            .unwrap()
            .transaction_id()
            .unwrap(),
        Some(5),
        "the second AppendEntries request escaped before release"
    );

    second_delay.release();
    second_delay
        .wait_for_completion(Duration::from_secs(10))
        .unwrap();
    for source in nodes.iter().filter(|node| node.node_id != target_id) {
        source.set_peer_blocked(target_id, false).unwrap();
    }
    wait_for_transaction(nodes, root, 5, Duration::from_secs(20));

    let leader_index = current_leader_index(nodes, Duration::from_secs(20));
    let leader = &nodes[leader_index];
    leader
        .runtime
        .block_on(leader.node.change_membership(all_voters.clone(), true))
        .unwrap();
    wait_for_membership(
        nodes,
        &all_voters,
        &BTreeSet::new(),
        Duration::from_secs(20),
    );
}

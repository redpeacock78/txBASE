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

pub(super) fn delay_batched_catchup_to_single_peer(nodes: &[RaftRuntime], root: &Path) {
    let leader_index = current_leader_index(nodes, Duration::from_secs(20));
    let leader = &nodes[leader_index];
    let target_id = nodes
        .iter()
        .find(|node| node.node_id != leader.node_id)
        .unwrap()
        .node_id;
    let leader_log_index = leader
        .node
        .metrics()
        .borrow()
        .last_log_index
        .expect("the leader must have a log before batched catch-up");
    let target = nodes.iter().find(|node| node.node_id == target_id).unwrap();
    assert_eq!(
        target.node.metrics().borrow().last_log_index,
        Some(leader_log_index),
        "the target must be caught up before isolation"
    );
    let target_root = root.join(format!("catalog-{target_id}"));
    let target_catalog = Catalog::from_path(&target_root).unwrap();
    let baseline_transaction_id = target_catalog
        .transaction_id()
        .unwrap()
        .expect("the target must have an applied transaction before isolation");
    let baseline_record_count = target_catalog.open_table("users").unwrap().records().len();

    for source in nodes.iter().filter(|node| node.node_id != target_id) {
        source.set_peer_blocked(target_id, true).unwrap();
    }
    for peer_id in nodes
        .iter()
        .filter(|node| node.node_id != target_id)
        .map(|node| node.node_id)
    {
        target.set_peer_blocked(peer_id, true).unwrap();
    }

    let first_sequence = baseline_transaction_id;
    let first_transaction_id = first_sequence + 1;
    let first_command = record_command(
        &root.join(format!("catalog-{}", leader.node_id)),
        first_sequence,
        48,
        "BatchedOne",
        48,
    );
    assert_eq!(
        commit(leader, first_command),
        RaftResponseResult::Applied {
            transaction_id: first_transaction_id
        }
    );
    let second_transaction_id = first_transaction_id + 1;
    let second_command = record_command(
        &root.join(format!("catalog-{}", leader.node_id)),
        first_sequence + 1,
        49,
        "BatchedTwo",
        49,
    );
    assert_eq!(
        commit(leader, second_command),
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
        root,
        second_transaction_id,
        Duration::from_secs(15),
    );
    assert_eq!(
        Catalog::from_path(&target_root)
            .unwrap()
            .transaction_id()
            .unwrap(),
        Some(baseline_transaction_id),
        "the isolated peer applied a command before catch-up"
    );

    let mut delayed_append = leader
        .delay_append_entries_at(target_id, leader_log_index + 2)
        .unwrap();
    for source in nodes.iter().filter(|node| node.node_id != target_id) {
        source.set_peer_blocked(target_id, false).unwrap();
    }
    for peer_id in nodes
        .iter()
        .filter(|node| node.node_id != target_id)
        .map(|node| node.node_id)
    {
        target.set_peer_blocked(peer_id, false).unwrap();
    }

    assert_eq!(
        delayed_append
            .wait_until_paused(Duration::from_secs(10))
            .unwrap(),
        2,
        "the catch-up AppendEntries request must contain both accumulated log entries"
    );
    assert_eq!(
        Catalog::from_path(&target_root)
            .unwrap()
            .transaction_id()
            .unwrap(),
        Some(baseline_transaction_id),
        "the delayed catch-up request escaped before release"
    );

    delayed_append.release();
    delayed_append
        .wait_for_completion(Duration::from_secs(10))
        .unwrap();
    wait_for_transaction(nodes, root, second_transaction_id, Duration::from_secs(20));
    for node in nodes {
        let catalog = Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
        assert_eq!(
            catalog.transaction_id().unwrap(),
            Some(second_transaction_id)
        );
        let table = catalog.open_table("users").unwrap();
        assert_eq!(table.records().len(), baseline_record_count + 2);
        for name in ["BatchedOne", "BatchedTwo"] {
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
}

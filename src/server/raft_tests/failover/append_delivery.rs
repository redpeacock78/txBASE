use super::*;

pub(super) fn delay_successive_appends_to_single_peer(nodes: &[RaftRuntime], root: &Path) {
    let leader_index = current_leader_index(nodes, Duration::from_secs(20));
    let leader = &nodes[leader_index];
    let target_id = nodes
        .iter()
        .find(|node| node.node_id != leader.node_id)
        .unwrap()
        .node_id;
    let mut first_delay = leader.delay_next_append_entries(target_id).unwrap();

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

    let mut second_delay = leader.delay_next_append_entries(target_id).unwrap();
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
    wait_for_transaction(nodes, root, 5, Duration::from_secs(20));
}

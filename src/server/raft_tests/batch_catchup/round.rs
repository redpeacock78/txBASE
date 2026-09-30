use super::*;
use std::path::Path;

pub(super) fn run_isolated_voter_catchup_round(
    nodes: &[RaftRuntime],
    root: &Path,
    payload_size: u64,
    next_record_id: &mut i64,
    max_payload_entries: u64,
) {
    let leader_index = current_leader_index(nodes, Duration::from_secs(20));
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

    for source in nodes {
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
        let record_id = *next_record_id + offset as i64;
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
    *next_record_id += payload_size as i64;

    let final_transaction_id = baseline_transaction_id + payload_size;
    wait_for_transaction(
        &healthy_nodes,
        root,
        final_transaction_id,
        Duration::from_secs(15),
    );
    let mut delayed_appends = (0..payload_size)
        .step_by(max_payload_entries as usize)
        .map(|entry_offset| {
            leader
                .delay_append_entries_at(target_id, leader_log_index + entry_offset + 1)
                .unwrap()
        })
        .collect::<Vec<_>>();
    leader.set_peer_blocked(target_id, false).unwrap();
    target.set_peer_blocked(leader.node_id, false).unwrap();

    for (batch_index, delayed_append) in delayed_appends.iter_mut().enumerate() {
        let applied_entries_before_request = batch_index as u64 * max_payload_entries;
        let request_entry_count = delayed_append
            .wait_until_paused(Duration::from_secs(10))
            .unwrap();
        assert!(
            (1..=max_payload_entries as usize).contains(&request_entry_count),
            "each catch-up AppendEntries request must respect the payload limit"
        );
        assert!(
            target
                .node
                .metrics()
                .borrow()
                .last_log_index
                .is_some_and(|last_log_index| {
                    last_log_index <= leader_log_index + applied_entries_before_request
                }),
            "the target log must not advance beyond the preceding catch-up boundary"
        );
        let target_catalog = Catalog::from_path(&target_root).unwrap();
        let target_table = target_catalog.open_table("users").unwrap();
        for name in names.iter().skip(applied_entries_before_request as usize) {
            assert_eq!(
                target_table
                    .active_records()
                    .filter(|record| record.values["NAME"].as_str() == Some(name.as_str()))
                    .count(),
                0,
                "the target must not apply a tail record while its AppendEntries request is held"
            );
        }
        if batch_index == 0 {
            assert_eq!(
                target_catalog.transaction_id().unwrap(),
                Some(baseline_transaction_id),
                "the first paused catch-up request must not update the isolated voter"
            );
            assert_eq!(
                target_table.records().len(),
                baseline_record_count,
                "the first paused catch-up request must leave the target table unchanged"
            );
        }
        delayed_append.release();
        delayed_append
            .wait_for_completion(Duration::from_secs(10))
            .unwrap();
    }
    wait_for_transaction(nodes, root, final_transaction_id, Duration::from_secs(20));
    for source in nodes {
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

    for node in nodes {
        let catalog = Catalog::from_path(root.join(format!("catalog-{}", node.node_id))).unwrap();
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

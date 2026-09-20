#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JoinStrategy {
    NestedLoop,
    IndexNestedLoop,
    Merge,
    Hash,
}

pub(super) const NESTED_LOOP_PAIR_LIMIT: usize = 64;

pub(super) fn choose(left_count: usize, right_count: usize, index_available: bool) -> JoinStrategy {
    choose_with_probe_cost(
        left_count,
        right_count,
        index_available.then_some(index_probe_cost(right_count, 0)),
    )
}

pub(super) fn choose_with_merge(
    left_count: usize,
    right_count: usize,
    index_available: bool,
    merge_available: bool,
) -> JoinStrategy {
    choose_with_costs(
        left_count,
        right_count,
        index_available.then_some(index_probe_cost(right_count, 0)),
        merge_available,
    )
}

pub(super) fn choose_with_probe_cost(
    outer_count: usize,
    inner_count: usize,
    probe_cost: Option<usize>,
) -> JoinStrategy {
    choose_with_costs(outer_count, inner_count, probe_cost, false)
}

fn choose_with_costs(
    left_count: usize,
    right_count: usize,
    probe_cost: Option<usize>,
    merge_available: bool,
) -> JoinStrategy {
    let pair_count = left_count.saturating_mul(right_count);
    if pair_count <= NESTED_LOOP_PAIR_LIMIT {
        return JoinStrategy::NestedLoop;
    }

    let mut best = (JoinStrategy::Hash, hash_cost(left_count, right_count), 2);
    if let Some(probe_cost) = probe_cost {
        best = choose_better(
            best,
            (
                JoinStrategy::IndexNestedLoop,
                left_count.saturating_mul(probe_cost),
                1,
            ),
        );
    }
    if merge_available {
        best = choose_better(
            best,
            (JoinStrategy::Merge, merge_cost(left_count, right_count), 0),
        );
    }
    best.0
}

fn choose_better(
    current: (JoinStrategy, usize, u8),
    candidate: (JoinStrategy, usize, u8),
) -> (JoinStrategy, usize, u8) {
    if (candidate.1, candidate.2) < (current.1, current.2) {
        candidate
    } else {
        current
    }
}

fn hash_cost(left_count: usize, right_count: usize) -> usize {
    left_count.saturating_add(right_count)
}

fn merge_cost(left_count: usize, right_count: usize) -> usize {
    hash_cost(left_count, right_count)
}

pub(super) fn index_probe_cost(inner_count: usize, average_fanout: usize) -> usize {
    if inner_count <= 1 {
        1usize.saturating_add(average_fanout)
    } else {
        (inner_count.ilog2() as usize)
            .saturating_add(1)
            .saturating_add(average_fanout)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        JoinStrategy, NESTED_LOOP_PAIR_LIMIT, choose, choose_with_merge, choose_with_probe_cost,
    };

    #[test]
    fn chooses_nested_loop_for_small_join_inputs() {
        assert_eq!(choose(8, 8, false), JoinStrategy::NestedLoop);
        assert_eq!(
            choose(NESTED_LOOP_PAIR_LIMIT, 1, true),
            JoinStrategy::NestedLoop
        );
    }

    #[test]
    fn chooses_hash_for_large_unindexed_inputs() {
        assert_eq!(choose(9, 8, false), JoinStrategy::Hash);
        assert_eq!(choose(usize::MAX, 2, false), JoinStrategy::Hash);
    }

    #[test]
    fn chooses_index_nested_loop_when_the_outer_side_is_small() {
        assert_eq!(choose(8, 1_000, true), JoinStrategy::IndexNestedLoop);
        assert_eq!(choose(1_000, 8, true), JoinStrategy::Hash);
    }

    #[test]
    fn chooses_hash_when_index_fanout_is_expensive() {
        assert_eq!(
            choose_with_probe_cost(100, 1_000, Some(super::index_probe_cost(1_000, 100))),
            JoinStrategy::Hash
        );
    }

    #[test]
    fn chooses_merge_for_large_dual_indexed_inputs() {
        assert_eq!(choose_with_merge(9, 8, true, true), JoinStrategy::Merge);
        assert_eq!(
            choose_with_merge(NESTED_LOOP_PAIR_LIMIT, 1, true, true),
            JoinStrategy::NestedLoop
        );
    }
}

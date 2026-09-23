#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JoinStrategy {
    NestedLoop,
    IndexNestedLoop,
    Merge,
    Hash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct JoinProbeCost {
    pub(super) per_probe: usize,
    pub(super) index_page_reads: usize,
    pub(super) record_page_reads_per_probe: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JoinCost {
    strategy_work: usize,
    index_page_reads: usize,
    record_page_reads: usize,
}

impl JoinCost {
    fn total(self) -> usize {
        self.strategy_work
            .saturating_add(self.index_page_reads)
            .saturating_add(self.record_page_reads)
    }
}

pub(super) const NESTED_LOOP_PAIR_LIMIT: usize = 64;

pub(super) fn choose(left_count: usize, right_count: usize, index_available: bool) -> JoinStrategy {
    choose_with_probe_cost(
        left_count,
        right_count,
        index_available.then_some(JoinProbeCost::legacy(right_count)),
    )
}

pub(super) fn choose_with_probe_cost(
    outer_count: usize,
    inner_count: usize,
    probe_cost: Option<JoinProbeCost>,
) -> JoinStrategy {
    choose_with_merge_page_cost(outer_count, inner_count, probe_cost, None)
}

pub(super) fn choose_with_merge_page_cost(
    left_count: usize,
    right_count: usize,
    probe_cost: Option<JoinProbeCost>,
    merge_page_reads: Option<usize>,
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
                index_nested_loop_cost(left_count, probe_cost),
                1,
            ),
        );
    }
    if let Some(page_reads) = merge_page_reads {
        best = choose_better(
            best,
            (
                JoinStrategy::Merge,
                merge_cost(left_count, right_count, page_reads),
                0,
            ),
        );
    }
    best.0
}

fn choose_better(
    current: (JoinStrategy, JoinCost, u8),
    candidate: (JoinStrategy, JoinCost, u8),
) -> (JoinStrategy, JoinCost, u8) {
    if (candidate.1.total(), candidate.2) < (current.1.total(), current.2) {
        candidate
    } else {
        current
    }
}

fn hash_cost(left_count: usize, right_count: usize) -> JoinCost {
    JoinCost {
        strategy_work: left_count.saturating_add(right_count.saturating_mul(2)),
        index_page_reads: 0,
        record_page_reads: 0,
    }
}

fn merge_cost(left_count: usize, right_count: usize, index_page_reads: usize) -> JoinCost {
    JoinCost {
        strategy_work: left_count.saturating_add(right_count),
        index_page_reads,
        record_page_reads: 0,
    }
}

fn index_nested_loop_cost(outer_count: usize, probe: JoinProbeCost) -> JoinCost {
    JoinCost {
        strategy_work: outer_count.saturating_mul(probe.per_probe),
        index_page_reads: probe.index_page_reads,
        record_page_reads: outer_count.saturating_mul(probe.record_page_reads_per_probe),
    }
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

impl JoinProbeCost {
    fn legacy(inner_count: usize) -> Self {
        Self {
            per_probe: index_probe_cost(inner_count, 0),
            index_page_reads: 0,
            record_page_reads_per_probe: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        JoinProbeCost, JoinStrategy, NESTED_LOOP_PAIR_LIMIT, choose, choose_with_merge_page_cost,
        choose_with_probe_cost,
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
            choose_with_probe_cost(
                100,
                1_000,
                Some(JoinProbeCost {
                    per_probe: super::index_probe_cost(1_000, 100),
                    index_page_reads: 0,
                    record_page_reads_per_probe: 0,
                }),
            ),
            JoinStrategy::Hash
        );
    }

    #[test]
    fn chooses_merge_for_large_dual_indexed_inputs() {
        assert_eq!(
            choose_with_merge_page_cost(9, 8, Some(JoinProbeCost::legacy(8)), Some(0)),
            JoinStrategy::Merge
        );
        assert_eq!(
            choose_with_merge_page_cost(
                NESTED_LOOP_PAIR_LIMIT,
                1,
                Some(JoinProbeCost::legacy(1)),
                Some(0),
            ),
            JoinStrategy::NestedLoop
        );
    }

    #[test]
    fn accounts_for_probe_page_reads() {
        assert_eq!(
            choose_with_probe_cost(
                8,
                1_000,
                Some(JoinProbeCost {
                    per_probe: 4,
                    index_page_reads: 2,
                    record_page_reads_per_probe: 1,
                }),
            ),
            JoinStrategy::IndexNestedLoop
        );
        assert_eq!(
            choose_with_probe_cost(
                100,
                1_000,
                Some(JoinProbeCost {
                    per_probe: 4,
                    index_page_reads: 1_000,
                    record_page_reads_per_probe: 20,
                }),
            ),
            JoinStrategy::Hash
        );
    }

    #[test]
    fn accounts_for_merge_index_pages() {
        assert_eq!(
            choose_with_merge_page_cost(80, 80, None, Some(0)),
            JoinStrategy::Merge
        );
        assert_eq!(
            choose_with_merge_page_cost(80, 80, None, Some(100)),
            JoinStrategy::Hash
        );
    }
}

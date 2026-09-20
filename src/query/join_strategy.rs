#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JoinStrategy {
    NestedLoop,
    IndexNestedLoop,
    Hash,
}

pub(super) const NESTED_LOOP_PAIR_LIMIT: usize = 64;

pub(super) fn choose(left_count: usize, right_count: usize, index_available: bool) -> JoinStrategy {
    // ponytail: fixed pair threshold; add measured index/cache terms when join statistics exist.
    let pair_count = left_count.saturating_mul(right_count);
    if pair_count <= NESTED_LOOP_PAIR_LIMIT {
        JoinStrategy::NestedLoop
    } else if index_available {
        JoinStrategy::IndexNestedLoop
    } else {
        JoinStrategy::Hash
    }
}

#[cfg(test)]
mod tests {
    use super::{JoinStrategy, NESTED_LOOP_PAIR_LIMIT, choose};

    #[test]
    fn chooses_nested_loop_for_small_join_inputs() {
        assert_eq!(choose(8, 8, false), JoinStrategy::NestedLoop);
        assert_eq!(
            choose(NESTED_LOOP_PAIR_LIMIT, 1, true),
            JoinStrategy::NestedLoop
        );
    }

    #[test]
    fn chooses_hash_for_large_join_inputs() {
        assert_eq!(choose(9, 8, false), JoinStrategy::Hash);
        assert_eq!(choose(usize::MAX, 2, false), JoinStrategy::Hash);
    }

    #[test]
    fn chooses_index_nested_loop_for_large_indexed_inputs() {
        assert_eq!(choose(9, 8, true), JoinStrategy::IndexNestedLoop);
    }
}

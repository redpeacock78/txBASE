use super::{
    COST_PAGE_SIZE, JoinCost, JoinCostInput, JoinProbeCost, JoinStrategy, NESTED_LOOP_PAIR_LIMIT,
    PageAccessEstimate, RecordPageLayout, choose, choose_with_costs, choose_with_probe_cost,
    estimated_cached_record_page_reads, hash_cost, index_nested_loop_cost, merge_cost,
    ordered_merge_sort_work, ordered_record_page_access, page_io_work,
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
        choose_with_costs(
            9,
            8,
            Some(JoinProbeCost::legacy(8)),
            Some(0),
            JoinCostInput::default()
        ),
        JoinStrategy::Merge
    );
    assert_eq!(
        choose_with_costs(
            NESTED_LOOP_PAIR_LIMIT,
            1,
            Some(JoinProbeCost::legacy(1)),
            Some(0),
            JoinCostInput::default(),
        ),
        JoinStrategy::NestedLoop
    );
}

#[test]
fn accounts_for_probe_page_reads() {
    assert_eq!(
        choose_with_costs(
            8,
            1_000,
            Some(JoinProbeCost {
                per_probe: 4,
                index_page_reads: 2,
                record_page_reads_per_probe: 1,
            }),
            None,
            JoinCostInput {
                inner_page_reads: 1_000,
                ..JoinCostInput::default()
            },
        ),
        JoinStrategy::IndexNestedLoop
    );
    assert_eq!(
        choose_with_costs(
            100,
            1_000,
            Some(JoinProbeCost {
                per_probe: 4,
                index_page_reads: 1_000,
                record_page_reads_per_probe: 20,
            }),
            None,
            JoinCostInput {
                inner_page_reads: 1_000,
                ..JoinCostInput::default()
            },
        ),
        JoinStrategy::Hash
    );
}

#[test]
fn caps_probe_page_reads_at_the_materialized_inner_capacity() {
    assert_eq!(estimated_cached_record_page_reads(10, 3, 5), 5);
    assert_eq!(estimated_cached_record_page_reads(2, 3, 5), 5);
    assert_eq!(estimated_cached_record_page_reads(2, 0, 5), 0);
}

#[test]
fn weights_random_pages_four_times_more_than_sequential_pages() {
    assert_eq!(page_io_work(1, 0), 1);
    assert_eq!(page_io_work(0, 1), 4);
}

#[test]
fn join_costs_expose_strategy_io_and_materialization_work() {
    let input = JoinCostInput {
        outer_page_reads: 2,
        inner_page_reads: 3,
        output_rows: 5,
        output_columns: 4,
        merge_sort_work: 13,
        ..JoinCostInput::default()
    };
    assert_eq!(
        hash_cost(5, 7, input),
        JoinCost {
            strategy_work: 19,
            page_io_work: 5,
            materialization_work: 20,
        }
    );
    assert_eq!(
        merge_cost(5, 7, 11, input),
        JoinCost {
            strategy_work: 25,
            page_io_work: 16,
            materialization_work: 20,
        }
    );
    assert_eq!(
        index_nested_loop_cost(
            5,
            JoinProbeCost {
                per_probe: 2,
                index_page_reads: 3,
                record_page_reads_per_probe: 2,
            },
            input,
        ),
        JoinCost {
            strategy_work: 10,
            page_io_work: 17,
            materialization_work: 20,
        }
    );
    assert_eq!(
        hash_cost(
            0,
            0,
            JoinCostInput {
                output_rows: 3,
                output_columns: 0,
                ..JoinCostInput::default()
            },
        ),
        JoinCost {
            strategy_work: 0,
            page_io_work: 0,
            materialization_work: 3,
        }
    );
    assert_eq!(
        JoinCost {
            strategy_work: 10,
            page_io_work: 20,
            materialization_work: 30,
        }
        .total(),
        60
    );
}

#[test]
fn join_cost_components_saturate_at_usize_max() {
    let input = JoinCostInput {
        outer_page_reads: usize::MAX,
        inner_page_reads: usize::MAX,
        output_rows: usize::MAX,
        output_columns: usize::MAX,
        merge_sort_work: usize::MAX,
        ..JoinCostInput::default()
    };
    let expected = JoinCost {
        strategy_work: usize::MAX,
        page_io_work: usize::MAX,
        materialization_work: usize::MAX,
    };
    assert_eq!(hash_cost(usize::MAX, usize::MAX, input), expected);
    assert_eq!(
        merge_cost(usize::MAX, usize::MAX, usize::MAX, input),
        expected
    );
    assert_eq!(
        index_nested_loop_cost(
            usize::MAX,
            JoinProbeCost {
                per_probe: usize::MAX,
                index_page_reads: usize::MAX,
                record_page_reads_per_probe: usize::MAX,
            },
            input,
        ),
        expected
    );
    assert_eq!(expected.total(), usize::MAX);
}

#[test]
fn accounts_for_merge_index_pages() {
    assert_eq!(
        choose_with_costs(80, 80, None, Some(0), JoinCostInput::default()),
        JoinStrategy::Merge
    );
    assert_eq!(
        choose_with_costs(80, 80, None, Some(100), JoinCostInput::default()),
        JoinStrategy::Hash
    );
}

#[test]
fn merge_page_cost_accounts_for_order_and_reuse() {
    let layout = RecordPageLayout {
        file_len: 3 * COST_PAGE_SIZE,
        header_length: 0,
        record_length: COST_PAGE_SIZE,
    };
    assert_eq!(
        ordered_record_page_access(layout, &[1, 2, 1, 3]),
        Some(PageAccessEstimate {
            sequential_page_reads: 2,
            random_page_reads: 1,
        })
    );
    assert_eq!(
        ordered_record_page_access(layout, &[1, 3, 2]),
        Some(PageAccessEstimate {
            sequential_page_reads: 0,
            random_page_reads: 3,
        })
    );
    assert_eq!(ordered_record_page_access(layout, &[4]), None);
    assert_eq!(
        ordered_record_page_access(
            RecordPageLayout {
                file_len: 2 * COST_PAGE_SIZE,
                header_length: COST_PAGE_SIZE - 6,
                record_length: 10,
            },
            &[1],
        ),
        Some(PageAccessEstimate {
            sequential_page_reads: 1,
            random_page_reads: 1,
        })
    );
    assert_eq!(
        ordered_record_page_access(layout, &[]),
        Some(PageAccessEstimate::default())
    );
}

#[test]
fn merge_prefers_clustered_pages_over_random_page_order() {
    let input = JoinCostInput {
        outer_page_reads: 250,
        inner_page_reads: 250,
        ..JoinCostInput::default()
    };
    assert_eq!(
        choose_with_costs(
            100,
            100,
            None,
            Some(0),
            JoinCostInput {
                merge_input_page_access: Some(PageAccessEstimate {
                    sequential_page_reads: 500,
                    random_page_reads: 0,
                }),
                ..input
            },
        ),
        JoinStrategy::Merge
    );
    assert_eq!(
        choose_with_costs(
            100,
            100,
            None,
            Some(0),
            JoinCostInput {
                merge_input_page_access: Some(PageAccessEstimate {
                    sequential_page_reads: 0,
                    random_page_reads: 500,
                }),
                ..input
            },
        ),
        JoinStrategy::Hash
    );
}

#[test]
fn accounts_for_record_pages_and_materialization() {
    let input = JoinCostInput {
        outer_page_reads: 2,
        inner_page_reads: 2,
        output_rows: 100,
        output_columns: 2,
        ..JoinCostInput::default()
    };
    assert_eq!(
        choose_with_costs(80, 80, None, Some(0), input),
        JoinStrategy::Merge
    );
    assert_eq!(
        choose_with_costs(80, 80, None, Some(1_000), input),
        JoinStrategy::Hash
    );
}

#[test]
fn indexes_include_the_outer_scan_pages() {
    assert_eq!(
        choose_with_costs(
            80,
            1_000,
            Some(JoinProbeCost {
                per_probe: 40,
                index_page_reads: 2,
                record_page_reads_per_probe: 1,
            }),
            None,
            JoinCostInput {
                outer_page_reads: 1_000,
                inner_page_reads: 1,
                output_rows: 80,
                output_columns: 1,
                ..JoinCostInput::default()
            },
        ),
        JoinStrategy::Hash
    );
}

#[test]
fn accounts_for_sorting_the_intermediate_merge_input() {
    assert_eq!(ordered_merge_sort_work(1, 2), 0);
    assert_eq!(ordered_merge_sort_work(8, 1), 32);
    assert_eq!(
        choose_with_costs(
            8,
            1_000,
            None,
            Some(0),
            JoinCostInput {
                merge_sort_work: ordered_merge_sort_work(8, 1),
                ..JoinCostInput::default()
            },
        ),
        JoinStrategy::Merge
    );
    assert_eq!(
        choose_with_costs(
            80,
            80,
            None,
            Some(0),
            JoinCostInput {
                merge_sort_work: ordered_merge_sort_work(80, 1),
                ..JoinCostInput::default()
            },
        ),
        JoinStrategy::Hash
    );
}

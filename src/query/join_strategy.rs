use crate::{dbf::DbfTable, index::COST_PAGE_SIZE};

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PageAccessEstimate {
    pub(super) sequential_page_reads: usize,
    pub(super) random_page_reads: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RecordPageLayout {
    file_len: usize,
    header_length: usize,
    record_length: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct JoinCostInput {
    pub(super) outer_page_reads: usize,
    pub(super) inner_page_reads: usize,
    pub(super) output_rows: usize,
    pub(super) output_columns: usize,
    pub(super) merge_sort_work: usize,
    pub(super) merge_input_page_access: Option<PageAccessEstimate>,
    pub(super) merge_inner_record_layout: Option<RecordPageLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JoinCost {
    strategy_work: usize,
    page_io_work: usize,
    materialization_work: usize,
}

impl JoinCost {
    fn total(self) -> usize {
        self.strategy_work
            .saturating_add(self.page_io_work)
            .saturating_add(self.materialization_work)
    }
}

const SEQUENTIAL_PAGE_COST: usize = 1;
const RANDOM_PAGE_COST: usize = 4;
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
    choose_with_costs(
        outer_count,
        inner_count,
        probe_cost,
        None,
        JoinCostInput::default(),
    )
}

pub(super) fn choose_with_costs(
    left_count: usize,
    right_count: usize,
    probe_cost: Option<JoinProbeCost>,
    merge_page_reads: Option<usize>,
    input: JoinCostInput,
) -> JoinStrategy {
    let pair_count = left_count.saturating_mul(right_count);
    if pair_count <= NESTED_LOOP_PAIR_LIMIT {
        return JoinStrategy::NestedLoop;
    }

    let mut best = (
        JoinStrategy::Hash,
        hash_cost(left_count, right_count, input),
        2,
    );
    if let Some(probe_cost) = probe_cost {
        best = choose_better(
            best,
            (
                JoinStrategy::IndexNestedLoop,
                index_nested_loop_cost(left_count, probe_cost, input),
                1,
            ),
        );
    }
    if let Some(page_reads) = merge_page_reads {
        best = choose_better(
            best,
            (
                JoinStrategy::Merge,
                merge_cost(left_count, right_count, page_reads, input),
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

fn hash_cost(left_count: usize, right_count: usize, input: JoinCostInput) -> JoinCost {
    JoinCost {
        strategy_work: left_count.saturating_add(right_count.saturating_mul(2)),
        page_io_work: page_io_work(
            input
                .outer_page_reads
                .saturating_add(input.inner_page_reads),
            0,
        ),
        materialization_work: materialization_work(input),
    }
}

fn merge_cost(
    left_count: usize,
    right_count: usize,
    index_page_reads: usize,
    input: JoinCostInput,
) -> JoinCost {
    let input_page_access = input.merge_input_page_access.unwrap_or(PageAccessEstimate {
        sequential_page_reads: input
            .outer_page_reads
            .saturating_add(input.inner_page_reads),
        random_page_reads: 0,
    });
    JoinCost {
        strategy_work: left_count
            .saturating_add(right_count)
            .saturating_add(input.merge_sort_work),
        page_io_work: page_io_work(
            input_page_access
                .sequential_page_reads
                .saturating_add(index_page_reads),
            input_page_access.random_page_reads,
        ),
        materialization_work: materialization_work(input),
    }
}

fn index_nested_loop_cost(
    outer_count: usize,
    probe: JoinProbeCost,
    input: JoinCostInput,
) -> JoinCost {
    JoinCost {
        strategy_work: outer_count.saturating_mul(probe.per_probe),
        page_io_work: page_io_work(
            input
                .outer_page_reads
                .saturating_add(probe.index_page_reads),
            estimated_cached_record_page_reads(
                outer_count,
                probe.record_page_reads_per_probe,
                input.inner_page_reads,
            ),
        ),
        materialization_work: materialization_work(input),
    }
}

fn page_io_work(sequential_pages: usize, random_pages: usize) -> usize {
    sequential_pages
        .saturating_mul(SEQUENTIAL_PAGE_COST)
        .saturating_add(random_pages.saturating_mul(RANDOM_PAGE_COST))
}

pub(super) fn record_page_layout(table: &DbfTable) -> RecordPageLayout {
    RecordPageLayout {
        file_len: table.byte_len(),
        header_length: usize::from(table.header.header_length),
        record_length: usize::from(table.header.record_length),
    }
}

pub(super) fn ordered_record_page_access(
    layout: RecordPageLayout,
    record_numbers: &[usize],
) -> Option<PageAccessEstimate> {
    if record_numbers.is_empty() {
        return Some(PageAccessEstimate::default());
    }
    if layout.record_length == 0 {
        return None;
    }

    let page_count = layout.file_len.div_ceil(COST_PAGE_SIZE);
    // ponytail: assume query-local reuse for every distinct page; a bounded cache model needs a cache budget.
    let mut seen_pages = vec![0_u64; page_count.div_ceil(u64::BITS as usize)];
    let mut previous_page = None;
    let mut access = PageAccessEstimate::default();

    for &record_number in record_numbers {
        let start = record_number
            .checked_sub(1)?
            .checked_mul(layout.record_length)?
            .checked_add(layout.header_length)?;
        let end = start.checked_add(layout.record_length)?;
        if end > layout.file_len {
            return None;
        }

        for page in start / COST_PAGE_SIZE..=(end - 1) / COST_PAGE_SIZE {
            let slot = seen_pages.get_mut(page / u64::BITS as usize)?;
            let mask = 1_u64 << (page % u64::BITS as usize);
            if *slot & mask != 0 {
                continue;
            }
            *slot |= mask;
            if previous_page.and_then(|previous: usize| previous.checked_add(1)) == Some(page) {
                access.sequential_page_reads = access.sequential_page_reads.saturating_add(1);
            } else {
                access.random_page_reads = access.random_page_reads.saturating_add(1);
            }
            previous_page = Some(page);
        }
    }

    Some(access)
}

fn estimated_cached_record_page_reads(
    outer_count: usize,
    record_page_reads_per_probe: usize,
    inner_page_capacity: usize,
) -> usize {
    // ponytail: cap reuse at the fully materialized inner table; page-level eviction needs a streaming executor.
    outer_count
        .saturating_mul(record_page_reads_per_probe)
        .min(inner_page_capacity)
}

fn materialization_work(input: JoinCostInput) -> usize {
    input
        .output_rows
        .saturating_mul(input.output_columns.max(1))
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

pub(super) fn ordered_merge_sort_work(row_count: usize, key_width: usize) -> usize {
    if row_count <= 1 || key_width == 0 {
        return 0;
    }
    row_count
        .saturating_mul((row_count.ilog2() as usize).saturating_add(1))
        .saturating_mul(key_width)
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
mod tests;

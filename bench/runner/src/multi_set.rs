//! The `multi-set` mixed workload — here "how many values sit under one key"
//! is a multiplicity (a count), not a bucket of distinct values, but it is
//! the identical load-bearing parameter `multi-map.rs`'s own docs describe:
//! a workload where every item's multiplicity stays at 1 would benchmark a
//! map with an extra counter, not a multiset.
//!
//! Op mix: 50% `add(item, 1.0)` (mutating, no checksum contribution), 25%
//! `multiplicity(item)` (pure read, contributing the count), 25%
//! `remove(item, 1.0)` (mutating, no checksum — `MultiSet::remove` has no
//! return value, matching `set`'s no-contribution convention elsewhere in
//! this batch).
//!
//! `delete`/`set` are deliberately excluded from this mix: both carry
//! reproduced-bug-for-bug corruption on paths this workload would hit
//! constantly (BUG-MULTI-SET-2's `size` going `NaN` on a `delete` of an absent item;
//! BUG-MULTI-SET-1's `set` silently adding instead of replacing on a present one —
//! `docs/modules/multi-set.md`). Exercising them here would benchmark how
//! fast the port recomputes `NaN`, not how fast a multiset counts — `add`/
//! `remove`/`multiplicity` are the well-behaved trio that stays representative
//! under a uniform random stream.
//!
//! # `size` is the item domain, held far below the op count
//!
//! `workload.size` is the number of distinct items (20,000 — the
//! `mixed-1e6` workload below), not the op count. `add`'s weight (50%) minus
//! `remove`'s (25%) is a net +25% growth rate, so across 1,000,000 ops that
//! is roughly 250,000 net increments spread over 20,000 items: **~12.5 net
//! multiplicity per item** by the run's end, with `remove`'s floor-at-zero
//! clamp (`MultiSet::remove`) keeping any single item from drifting far
//! negative. This is the same "small domain, many ops" shape `multi-map.rs`
//! uses, applied to a multiplicity instead of a bucket length.

use mnemonist_core::structures::multi_set::MultiSet;

use crate::workload::Workload;

/// One measured pass: fresh set, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut set: MultiSet<u32> = MultiSet::new();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let item = workload.a[i];

            match workload.kind[i] {
                0 | 1 => set.add(item, 1.0),
                2 => checksum += set.multiplicity(&item) as u64,
                _ => set.remove(item, 1.0),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&set);

    (batches, checksum)
}

/// `--structure`: prefill `size` distinct items at multiplicity 1 and touch
/// the set. Same reasoning as `multi-map.rs::build_structure`: the extra
/// multiplicity the mixed workload reaches is emergent from running the op
/// stream, not a fixed per-item cost this isolation needs to reproduce.
pub fn build_structure(size: u32) {
    let mut set: MultiSet<u32> = MultiSet::new();

    for i in 0..size {
        set.add(i, 1.0);
    }

    std::hint::black_box(&set);
    std::hint::black_box(set.has(&(size - 1)));
}

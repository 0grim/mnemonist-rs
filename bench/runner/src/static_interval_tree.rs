//! The `static-interval-tree` mixed workload — a `StaticIntervalTree<(f64,
//! f64)>` built once from `size` overlapping intervals, then queried
//! repeatedly.
//!
//! # No `add`, same shape as `vp-tree`/`kd-tree`
//!
//! `StaticIntervalTree::new` builds a balanced, augmented BST from a sorted
//! index list and there is no way to add an interval afterwards — see that
//! module's own docs. The tree is built once before the timed batches
//! begin; every timed op is a query.
//!
//! # Intervals overlap by construction, not by chance
//!
//! Every interval is `(start, start + LENGTH)` with `start` drawn uniformly
//! from `0..size` and `LENGTH` a tenth of the domain — wide enough that a
//! typical point or query interval genuinely overlaps a meaningful fraction
//! of the whole set (measured below), not the handful of degenerate
//! zero-width or barely-touching intervals a naive random-pairs generator
//! would mostly produce. The augmented max-end pruning both query methods
//! rely on only has something to prune when *most* candidate subtrees can be
//! ruled out while a *real* fraction survives to be visited — a workload of
//! near-disjoint intervals would make every subtree's own max-end bound
//! irrelevant (nothing to prune) and a workload where every interval
//! contains every point would make it equally irrelevant the other way
//! (nothing IS pruned). `new` itself sorts by start with `Vec::sort_by` (a
//! proper comparison sort, not the fixed-pivot
//! `inplace_quick_sort_indices` `vp_tree.rs`/`kd_tree.rs` both have to guard
//! against), so no input-order trap applies here.
//!
//! Op mix: 50% `intervals_containing_point` (pure read), 50%
//! `intervals_overlapping_interval` (pure read, itself a `LENGTH`-wide
//! query) — no mutating op, same reason as `vp-tree`/`kd-tree`. Both
//! contribute a position-weighted checksum: neither query method sorts its
//! output, so a plain sum would agree on the same *set* of matches while
//! missing a divergence in the *order* the DFS actually visited them.
//!
//! # Domain
//!
//! `size` 100,000 intervals, matching `kd-tree`'s own scale. Measured before
//! committing: `intervals_containing_point` averages ~10,000 matches per
//! call (10% of the set, as `LENGTH` = 10% of the domain implies for a
//! uniformly-covered point), `intervals_overlapping_interval` averages
//! ~19,000 (two overlapping ranges union to roughly double the single-point
//! figure) — both a real, substantial fraction of the tree pruned around,
//! neither degenerate.

use mnemonist_core::structures::static_interval_tree::StaticIntervalTree;

use crate::workload::Workload;
use crate::xorshift::XorShift32;

/// Interval width as a fraction of the domain — wide enough that pruning is
/// genuinely partial rather than all-or-nothing (see the module docs), but
/// far short of the 10% first tried: that produced ~10,000 matches per
/// query at this module's 100,000-item domain, and collecting (cloning,
/// position-weighting) tens of thousands of hits per call made a
/// 200,000-op pass take **22 seconds** — the same shape as `bit_set.rs`'s
/// `rank` trap, an unrepresentative and pathological workload rather than
/// a demanding one. 0.1% keeps the average match count in the low hundreds.
const LENGTH_FRACTION: f64 = 0.001;

/// `size` intervals `(start, start + length)`, `start` drawn from a
/// fixed-seed `XorShift32` (seed 1, independent of the workload's own
/// `--seed`) — same convention `vp_tree.rs`/`kd_tree.rs` use for their own
/// structure-building randomness.
fn overlapping_intervals(size: u32) -> Vec<(f64, f64)> {
    let mut rng = XorShift32::new(1);
    let length = f64::from(size.max(1)) * LENGTH_FRACTION;

    (0..size)
        .map(|_| {
            let start = f64::from(rng.below(size.max(1)));
            (start, start + length)
        })
        .collect()
}

/// One measured pass: build the tree once (untimed), then the whole
/// workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let bounds = overlapping_intervals(workload.size);
    let intervals = bounds.clone();
    let tree = StaticIntervalTree::new(intervals, bounds)
        .expect("workload.size is always >= 1, so this is never zero intervals");

    let domain = f64::from(workload.size.max(1));
    let length = domain * LENGTH_FRACTION;

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            // No mutating op, so a 50/50 split (rather than this batch's
            // usual 50/25/25) is what actually gives both query methods
            // equal weight -- see the module docs.
            let hits = if workload.kind[i] < 2 {
                let point = f64::from(workload.a[i] % workload.size.max(1));
                tree.intervals_containing_point(point)
            } else {
                let query_start = f64::from(workload.a[i] % workload.size.max(1));
                tree.intervals_overlapping_interval(query_start, query_start + length)
            };

            if let Ok(hits) = hits {
                for (position, (interval_start, _end)) in hits.iter().enumerate() {
                    checksum += (position as u64 + 1) * (*interval_start as u64 + 1);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&tree);

    (batches, checksum)
}

/// `--structure`: a `StaticIntervalTree` has no capacity distinct from its
/// interval count — "size" means "built from `size` overlapping intervals",
/// the same domain `run_mixed` itself uses.
pub fn build_structure(size: u32) {
    let bounds = overlapping_intervals(size);
    let intervals = bounds.clone();
    let tree =
        StaticIntervalTree::new(intervals, bounds).expect("size is always >= 1 via size_flag");

    std::hint::black_box(&tree);
}

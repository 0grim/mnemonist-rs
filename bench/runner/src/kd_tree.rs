//! The `kd-tree` mixed workload — a 2-D `KdTree<u32>` built once from
//! `size` scattered points, then queried repeatedly.
//!
//! # No `add`, same as `vp-tree`
//!
//! `KDTree` is built once (`from_rows`) and never mutated afterwards — see
//! `kd_tree.rs`'s own module docs, which call this out as the same shape
//! `vp_tree.rs` already has. The tree is built before the timed batches
//! begin; every timed op is a query.
//!
//! # Points are scattered, not `0..size` on a line
//!
//! `build_tree` sorts each level's window by ONE axis's raw value
//! (`inplace_quick_sort_indices`, the same fixed-pivot algorithm
//! `vp_tree.rs`'s construction uses) — sorted or correlated input is exactly
//! that algorithm's weak spot (see `vp_tree.rs`'s own module docs for the
//! measurement that caught it there). [`scattered_points`] draws two
//! independent coordinates per point from a fixed-seed `XorShift32` (seed 1,
//! independent of the workload's own `--seed`, same convention `vp_tree.rs`
//! uses), so neither axis is ever sorted, or correlated with insertion
//! order, before construction sorts it.
//!
//! # The load-bearing parameter: the nearest neighbour crossing the plane
//!
//! `nearest_neighbor`'s recursion descends the side of the splitting plane
//! the query falls on, then checks `dx * dx < best_distance` to decide
//! whether the OTHER side could still hold something closer — the classic
//! kd-tree backtrack. For real 2-D data (not points collapsed onto one
//! axis), a meaningful fraction of queries land close enough to *some*
//! split, at *some* depth, that this check succeeds and the algorithm
//! genuinely crosses — this is a property of querying the same distribution
//! the tree was built from in 2 dimensions, not something that needs a
//! second parameter to force (contrast `bk-tree`/`vp-tree`, where a single
//! fixed radius does not naturally exercise both outcomes and two radii were
//! needed instead). `k_nearest_neighbors` reuses
//! `crate::structures::fixed_reverse_heap::FixedReverseHeap` and its own
//! tie-break order, exercising a second, genuinely different query path.
//!
//! Op mix: 75% `nearest_neighbor` (pure read, one label), 25%
//! `k_nearest_neighbors(K)` (pure read, position-weighted — the heap pins an
//! exact tie-break order, `vp_tree.rs`'s own reasoning for the same
//! convention). Not a 50/25/25 or 40/40/20 split: `workload.kind` only ever
//! takes four values, so 3-of-4 / 1-of-4 is the natural division with no
//! mutating op to balance against. No mutating op, same reason as
//! `vp-tree`.
//!
//! # Domain
//!
//! `size` 100,000 points over a `0..100,000` coordinate range per axis —
//! large enough for genuine tree depth (~17 levels), and construction (two
//! independent per-level sorts instead of one) was measured at this size
//! before committing: well under a second, because the coordinates are
//! scattered rather than sorted going in.
//!
//! # This module loses on p50/p99 — measured, and the asymmetry pinned down
//!
//! `mixed-1e5` is the one workload in this batch where the port is
//! genuinely slower: ~2x on p50/p99/min. Isolated with a standalone probe
//! (200,000 calls of each method alone, both sides, same tree): `nearest_neighbor`
//! is 331 ns/call in this port against upstream's 620 ns (the port wins, as
//! elsewhere) — but `k_nearest_neighbors` is 6.6 µs/call here against
//! upstream's 2.1 µs, a genuine, measured reversal, and the SHAPE of the
//! reversal is the interesting part: this port's own `k_nearest_neighbors`
//! costs 20x its own `nearest_neighbor`, where upstream's costs only 3.4x
//! its own — not merely "the k-NN path is slower," but "the k-NN path is
//! disproportionately slower than this same port's own baseline shows it
//! could be." **Cause: unconfirmed.** `recurse_knn` heap-allocates a fresh
//! 3-element `Vec<f64>` for every node visited (`crate::structures::
//! fixed_reverse_heap::FixedReverseHeap`'s backing store, ported verbatim
//! from upstream's own tuple-array shape) where V8's generational GC can
//! bump-allocate the equivalent short-lived array far more cheaply than a
//! general-purpose Rust allocator handles many small, short-lived `malloc`s
//! — a plausible mechanism given where the two sides' costs diverge, but
//! this was not confirmed with a profiler (no allocation count or `perf`
//! trace was taken), so it is labelled a hypothesis, not a finding, per
//! CLAUDE.md's rule against overclaiming performance causation.

use mnemonist_core::structures::kd_tree::KdTree;

use crate::workload::Workload;
use crate::xorshift::XorShift32;

/// `nearest_neighbors`'s own bounded `k`.
const K: usize = 10;

/// `size` scattered 2-D points, coordinates drawn from a fixed-seed
/// `XorShift32` (seed 1, independent of the workload's own `--seed`) — see
/// the module docs for why an unsorted, uncorrelated domain matters here.
fn scattered_points(size: u32) -> (Vec<Vec<f64>>, Vec<u32>) {
    let mut rng = XorShift32::new(1);
    let mut axis0 = Vec::with_capacity(size as usize);
    let mut axis1 = Vec::with_capacity(size as usize);
    let mut labels = Vec::with_capacity(size as usize);

    for i in 0..size {
        axis0.push(f64::from(rng.below(size.max(1))));
        axis1.push(f64::from(rng.below(size.max(1))));
        labels.push(i);
    }

    (vec![axis0, axis1], labels)
}

/// One measured pass: build the tree once (untimed), then the whole
/// workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let (axes, labels) = scattered_points(workload.size);
    let tree = KdTree::from_axes(axes, labels).expect("a benchmark tree is well-formed");

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let query = [
                f64::from(workload.a[i] % workload.size.max(1)),
                f64::from(workload.b[i] % workload.size.max(1)),
            ];

            // 75/25 (3-of-4 / 1-of-4), not this batch's usual 50/25/25 --
            // see the module docs for why: there is no mutating op, and a
            // single query already exercises both outcomes of the
            // cross-plane check, so this split just diversifies which of
            // the two query methods runs.
            if workload.kind[i] < 3 {
                if let Some(label) = tree.nearest_neighbor(&query) {
                    checksum += u64::from(*label);
                }
            } else if let Ok(hits) = tree.k_nearest_neighbors(K, &query) {
                for (position, label) in hits.iter().enumerate() {
                    checksum += (position as u64 + 1) * u64::from(*label);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&tree);

    (batches, checksum)
}

/// `--structure`: a `KdTree` has no capacity distinct from its point count —
/// "size" means "built from `size` scattered points", the same domain
/// `run_mixed` itself uses.
pub fn build_structure(size: u32) {
    let (axes, labels) = scattered_points(size);
    let tree = KdTree::from_axes(axes, labels).expect("a benchmark tree is well-formed");

    std::hint::black_box(&tree);
}

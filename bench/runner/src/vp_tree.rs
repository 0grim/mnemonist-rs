//! The `vp-tree` mixed workload — a `VpTree<u32>` built once from a shuffled
//! `0..size`, metric `distance(a, b) = |a - b|` (as `f64`), then queried
//! repeatedly.
//!
//! # No `add` — this workload has no mutating op at all
//!
//! Unlike every earlier module in this batch, `VPTree` genuinely cannot be
//! updated after construction (`vp-tree.js`'s own constructor throws if you
//! try to build one without items, and there is no `add` anywhere in the
//! prototype) — see `vp_tree.rs`'s own module docs. So the tree is built
//! **once**, before the timed batches begin, from the same convention
//! `trie.rs`/`heap.rs` use for their empty starting structure — except here
//! the "start" is the full, populated tree, because there is no cheaper one.
//! Every timed op is therefore a query; `workload.kind` still drives which
//! of three query shapes runs, just none of them mutate.
//!
//! # Two query shapes, and the load-bearing radius split
//!
//! `neighbors(radius, query)` prunes a subtree via the triangle inequality
//! whenever `mu` and `radius` rule it out; a workload that always prunes or
//! never prunes would time only one branch. `SMALL_RADIUS` prunes
//! aggressively; `LARGE_RADIUS` prunes far less, reaching real matches —
//! both measured below before committing. `nearest_neighbors(k, query)` is
//! the module's other public query, exercising the bounded
//! max-heap-by-distance machinery (`crate::structures::heap::Heap`, reused
//! verbatim from upstream's own tie-break order) and its own, different
//! pruning bound (`tau`, tightened as the heap fills) — genuinely different
//! code from `neighbors`, not a third radius on the same path.
//!
//! # Domain, and why the items are shuffled rather than `0..size` in order
//!
//! Construction sorts each node's window by distance from that node's
//! vantage point (`inplace_quick_sort_indices`, ported verbatim from
//! upstream, fixed-pivot — the first element of the window). Feeding it
//! `0..size` in order is exactly quicksort's classic worst case: the vantage
//! point is an extreme value, so distances from it are already monotonic
//! *before* the sort runs, and a fixed first-element pivot degrades sharply
//! on already-sorted input. Measured, not assumed, before this was caught
//! (the `bit_set.rs` `rank` lesson `methodology.md` documents — an input
//! that looks innocuous and is quadratic in practice): building a
//! 300,000-item tree from sequential items took **over 45 seconds of CPU
//! time**. [`shuffled_items`] permutes `0..size` with a Fisher-Yates shuffle
//! over a fixed-seed instance of the same matched `XorShift32` the workload
//! stream itself uses (seeded `1`, independent of the workload's own
//! `--seed`, so changing `--seed` cannot silently change the tree's shape)
//! — its own JS twin (`bench/node/run.js`) reproduces the identical
//! permutation, so both sides build the identical tree.
//!
//! **Shuffling helps but does not fully flatten this cost — and that
//! remainder was verified to be upstream's own property, not a port-only
//! regression**, before `size` was set: a standalone probe against
//! `bench/upstream/vp-tree.js` itself (same shuffle, same metric) building
//! 80,000 items took ~2 seconds of wall time there too, the same order of
//! magnitude as this port's own ~0.9 seconds at that size. Construction
//! sorts by DISTANCE FROM A VANTAGE POINT at every level, not by raw value,
//! and that repeated re-sorting evidently still correlates with the fixed
//! pivot's weak spot often enough to cost more than `O(n log n)` — a
//! genuine property of the ported algorithm over a metric this
//! one-dimensional, not a Rust-only pathology. `size` = 50,000 (down from an
//! initial 300,000, chosen for the same reason `bit_set.rs`'s `rank` was
//! pulled rather than kept at a flattering size) keeps a full pass fast on
//! both sides. Measured at this domain: `SMALL_RADIUS` averages ~21 hits per
//! `neighbors` call, `LARGE_RADIUS` averages ~798 — two genuinely different
//! regimes over the same ~50,000-node tree, neither degenerate (0 or "all of
//! it").
//!
//! Op mix: 40% `neighbors` at `SMALL_RADIUS`, 40% `neighbors` at
//! `LARGE_RADIUS`, 20% `nearest_neighbors(K)` — not the batch's usual
//! 50/25/25, because there is no mutating op to give half the stream to;
//! splitting the two `neighbors` calls evenly is what actually forces both
//! pruning outcomes across half the workload each, with the heap-based
//! query filling the remaining fifth. Every branch contributes a
//! **position-weighted** checksum (`sort.rs`'s own convention) rather than a
//! plain sum: `nearestNeighbors` pins an exact tie-break order in upstream's
//! own test suite, and a plain sum would agree on the same *set* of results
//! while missing a divergence in their *order*.

use mnemonist_core::structures::vp_tree::VpTree;

use crate::workload::Workload;
use crate::xorshift::XorShift32;

/// `0..size`, Fisher-Yates shuffled with a fixed-seed `XorShift32` (seed 1,
/// independent of the workload's own `--seed`) — see the module docs for why
/// an in-order domain is quicksort's worst case for this module's
/// construction step.
fn shuffled_items(size: u32) -> Vec<u32> {
    let mut items: Vec<u32> = (0..size).collect();
    let mut rng = XorShift32::new(1);

    for i in (1..items.len()).rev() {
        let j = (rng.next() as usize) % (i + 1);
        items.swap(i, j);
    }

    items
}

/// Prunes aggressively at this workload's 50,000-item domain — measured
/// ~21 hits/call (see the module docs).
const SMALL_RADIUS: f64 = 10.0;

/// Prunes far less — measured ~798 hits/call, ~1.6% of the tree.
const LARGE_RADIUS: f64 = 400.0;

/// `nearest_neighbors`'s own fixed `k`.
const K: usize = 10;

fn distance(a: &u32, b: &u32) -> f64 {
    (f64::from(*a) - f64::from(*b)).abs()
}

/// One measured pass: build the tree once (untimed), then the whole
/// workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let items = shuffled_items(workload.size);
    let tree = VpTree::new(items, distance);

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let query = workload.a[i];

            // 40/40/20 rather than this batch's usual 50/25/25 -- see the
            // module docs for why: there is no mutating op to give half the
            // stream to, so the two `neighbors` radii (the load-bearing
            // split) each get 40%, with the heap-based query filling the
            // rest.
            let hits = match workload.kind[i] {
                0 => tree.neighbors(SMALL_RADIUS, &query, distance),
                1 | 2 => tree.neighbors(LARGE_RADIUS, &query, distance),
                _ => tree.nearest_neighbors(K, &query, distance),
            };

            for (position, neighbor) in hits.iter().enumerate() {
                checksum += (position as u64 + 1) * u64::from(neighbor.item);
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&tree);

    (batches, checksum)
}

/// `--structure`: a `VpTree` has no capacity distinct from its item count —
/// "size" means "built from a shuffled `0..size`", the same domain
/// `run_mixed` itself uses.
pub fn build_structure(size: u32) {
    let items = shuffled_items(size);
    let tree = VpTree::new(items, distance);

    std::hint::black_box(&tree);
}

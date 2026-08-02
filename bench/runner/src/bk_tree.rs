//! The `bk-tree` mixed workload — `u32` items over a shared
//! `BkTree<u32>`, with the metric `distance(a, b) = |a - b|`.
//!
//! # Why a numeric metric, not Levenshtein over strings
//!
//! Upstream's own test suite uses Levenshtein over short words, but a BK-tree
//! is metric-agnostic (`add`/`search` never inspect an item, only what
//! `distance` returns), and `|a - b|` is a genuine metric (non-negative,
//! symmetric, satisfies the triangle inequality) that both sides can compute
//! identically with zero risk of a Levenshtein port drifting apart in an edge
//! case, and keeps the metric itself effectively free so the timed loop
//! measures the tree, not the distance function.
//!
//! # The load-bearing parameter: search radius, both ways
//!
//! `search(n, query)` only descends into a child whose distance key falls in
//! `[d - n, d + n]`; a workload where every call prunes (n too small to ever
//! match a real gap between distances) or where no call ever prunes (n large
//! enough to always cover every child) would time only one branch of that
//! check. Two radii, drawn by op kind rather than fixed: `SMALL_RADIUS`
//! yields hits rarely (mostly pruned — measured below), `LARGE_RADIUS`
//! yields real hits reliably (genuinely descended into) without ever
//! covering more than a sliver of the tree. Both are exercised on the same
//! tree, built by the same `add` stream — one workload whose op mix forces
//! both outcomes of the range check, not two benchmarks glued together.
//!
//! # Domain, and why it is neither 1e6 nor far below it
//!
//! Two failure modes were found and rejected before committing to
//! `size` = 300,000, `ops` = 1e6 (the `bit_set.rs` `rank` lesson
//! `methodology.md` documents: sanity-check a workload's cost before
//! trusting it):
//!
//! * **Domain too small relative to `ops`** (tried: 2,000): every domain
//!   value collects thousands of `add` duplicates, and a duplicate is
//!   always zero distance from the previous one of the same value — so
//!   duplicates chain onto each other one node at a time, `add` and
//!   `search` both degrade towards O(chain length), and a 200,000-op run
//!   at this domain took **21 seconds** (100x the op count for 140x the
//!   wall time — superlinear, the same shape `bit_set.rs`'s `rank` trap
//!   had).
//! * **Domain too large relative to `ops`** (tried: 1,000,000, `ops` = 1e6):
//!   distances from the root are then almost all distinct, so nearly every
//!   `add` lands as a direct child of the root and the tree collapses to
//!   depth 1 — `search`'s recursive descent is real code that never runs,
//!   which defeats the point of measuring a *tree* rather than one wide
//!   hash map. Measured directly: `tree.size()` came back within 0.1% of
//!   the number of `add` calls, meaning almost no node has more than one
//!   level of children below it.
//!
//! `size` = 300,000 against 1e6 ops (500,000 `add` calls) sits between both:
//! measured `avg_found` at `SMALL_RADIUS` is ~4 (mostly pruned, occasional
//! genuine near-ties) and at `LARGE_RADIUS` is ~34 (reliably real matches,
//! still a small fraction of the ~500,000-node tree) — both regimes
//! non-degenerate, and a full 1e6-op pass completes in ~4 seconds.
//!
//! Op mix: 50% `add` (mutating, no checksum contribution), 25% `search` at
//! `SMALL_RADIUS` (pure read), 25% `search` at `LARGE_RADIUS` (pure read) —
//! both contributing the match COUNT, upstream's own `found.length`, not
//! the richer `Vec<Found<I>>` core returns.

use mnemonist_core::structures::bk_tree::BkTree;

use crate::workload::Workload;

/// Mostly pruned at this workload's 300,000-item domain — measured `avg_found`
/// ~4 per call (see the module docs).
const SMALL_RADIUS: i64 = 2;

/// Reliably yields real matches without covering more than a sliver of the
/// tree — measured `avg_found` ~34 per call, against a ~500,000-node tree.
const LARGE_RADIUS: i64 = 20;

fn distance(a: &u32, b: &u32) -> i64 {
    (i64::from(*a) - i64::from(*b)).abs()
}

/// One measured pass: fresh tree, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut tree: BkTree<u32> = BkTree::new();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let item = workload.a[i];

            match workload.kind[i] {
                0 | 1 => tree.add(item, distance),
                2 => checksum += tree.search(SMALL_RADIUS, &item, distance).len() as u64,
                _ => checksum += tree.search(LARGE_RADIUS, &item, distance).len() as u64,
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&tree);

    (batches, checksum)
}

/// `--structure`: a `BkTree` has no capacity distinct from occupied size —
/// "size" means "after `size` `add` calls over the `0..size` domain",
/// matching `run_mixed`'s own convention (`workload.size` is both the domain
/// bound and, here, the fill count).
pub fn build_structure(size: u32) {
    let mut tree: BkTree<u32> = BkTree::new();

    for i in 0..size {
        tree.add(i % size.max(1), distance);
    }

    std::hint::black_box(&tree);
}

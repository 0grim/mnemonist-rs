# kd-tree — evidence

Gate artifacts for `docs/modules/kd-tree.md`: test-to-gap table, fuzz grammar, full falsification
record, full benchmark tables.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/kd_tree.rs` — 9 tests:

| Test | Closes gap |
|---|---|
| `builds_the_tree_upstream_pins` | 1:1 transcription of `should be keep sane` |
| `builds_from_axes_directly_and_agrees_with_from_rows` | 1:1 transcription of the `fromAxes` test |
| `builds_from_axes_without_labels_using_positional_indices` | 1:1 transcription of the no-labels case |
| `k_nearest_neighbors_matches_brute_force_membership` | 1:1 transcription of the knn test |
| `linear_k_nearest_neighbors_matches_upstreams_pinned_case` | 1:1 transcription of the pinned `['five', 'four', 'one']` order |
| `k_is_clamped_to_size_rather_than_padding_with_nothing` | the `k` > size gap |
| `zero_k_is_rejected_with_upstreams_message` | the `k <= 0` gap (narrowed — see D-408) |
| `an_empty_tree_builds_cleanly_and_answers_no_queries` | the empty-tree gap (D-407) |
| `finds_neighbors_across_the_splitting_plane` | the "proven necessary" gap — see the falsification record |

## Fuzz grammar

* **Op alphabet:** `nearestNeighbor(query)` (weight 3) · `kNearestNeighbors(k, query)` (3) ·
  `linearKNearestNeighbors(k, query)` (3).
* **`.from`, not `new KDTree(...)`.** Upstream's own raw constructor takes an already-built
  internal shape (D-406), so this is the first module in the port whose `ModuleSpec` needs an
  alternate entry point: `static_factory()` names `"from"`, and `fuzz/oracle.js`'s `init` case
  grew an additive `staticFactory` field (`Ctor[name](...)` instead of `new Ctor(...)`) —
  optional, defaulted to the prior behaviour for every other module.
* **A dense 12×12 integer grid, not a sparse or wide-ranging one.** The sharp risk for this module
  is named directly — queries whose nearest neighbor lies across a splitting plane from the query
  point, precisely the case a naive implementation gets wrong — and a dense grid is the direct
  answer: many points share a coordinate on whichever axis the tree splits on (which is what puts a
  query close to a plane at all), and many points sit at genuinely equal squared distance from a
  query (forcing `kNearestNeighbors`' `[dist, visited++, pivot]` tie-break to actually run). Query
  points are drawn from a *wider* window than the grid (`-6..18` against a `0..12` grid) so some
  land well outside the point cloud and some deep inside it.
* **Observable state: `size`, `dimensions`, `pivots`, `lefts`, `rights`** — the tree's exact shape,
  compared on every generated construction.

**Measured evidence of splitting-plane crossings and genuine ties**
(500 sampled queries against a random 60-point instance of the grid):

```
kd-tree grammar_self_check: 191/500 queries had their true nearest neighbor across a naive
single-axis split; 100/500 had a genuine distance tie
```

## Falsification record (gate 6)

**Named first:** two targets, in order.

**Target 1 (as originally named): `builds_the_tree_upstream_pins`'s
`assert_eq!(tree.nearest_neighbor(&[8.0, 5.0]), Some(&"two"))`.** **The sabotage:**
`recurse_nearest`'s "go the other way too" branch (`if dx * dx < *best_distance`) forced to `false`
unconditionally, disabling backtracking into the sibling subtree entirely.

**Stayed green.** Investigated rather than swapped for an easier target: by hand, against the
tree's own pinned `pivots = [5, 1, 0, 3, 2, 4]`/`lefts = [2, 3, 0, 0, 6, 0]`/`rights = [5, 4, 0, 0,
0, 0]`, the root's primary descent for query `[8, 5]` goes directly to node 4 (`'two'` at `(9,
6)`), a leaf, on the very first recursive call — before the disabled branch is ever consulted. The
assertion is not incapable of expressing the defect; this specific query, against this specific
six-point tree, simply does not exercise the branch at all. A first attempt at a stronger native
regression test (`finds_neighbors_across_the_splitting_plane`, a 64-point diagonal line) *also*
stayed green under the same sabotage — investigated to the same root cause: every point's `x`
equalling its `y` lets the primary "trust the split" descent converge on the right answer by
construction, coordinate by coordinate, without ever needing to look at the other side.

**Target 2 (after investigation): the differential fuzzer's own `grammar_self_check`
(`crates/difffuzz/src/modules/kd_tree.rs`), whose dense-grid construction (not a line) is exactly
what the Fuzz section above measures as producing splitting-plane crossings.** Run against the
still-sabotaged core: **confirmed red** at `assertion left == right failed ... for [9.0, 3.0]:
left: Some(1.0), right: Some(0.0)` — the sabotaged tree's answer was a full unit of squared
distance worse than the true nearest neighbor.

`finds_neighbors_across_the_splitting_plane` was then rebuilt on the same kind of dense grid (a
10×10 grid, 200 sampled random queries, comparing `nearest_neighbor` specifically — not
`kNearestNeighbors`, whose recursion is a *different* function with its own independent copy of
this branch and would not have exercised the sabotage at all) and **confirmed red** at the intended
assertion. Reverted; **confirmed green again**: all 9 `kd_tree` unit tests pass, `cargo test
--workspace` clean.

## Bench tables

`bench/results.json` → `modules["kd-tree"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e5`** (current, post-fix) — 1e6 mixed `nearestNeighbor`/`kNearestNeighbors` (75/25) over
100,000 scattered 2-D points, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **755.10** | 927.85 | port 1.23× faster |

RSS delta MB **9.6** (port) vs 51.3 (upstream); structure-only RSS delta MB **0.2** vs 6.0; startup
ms **0.6** vs 15.3 (26×, reported separately, not throughput) — these are unaffected by the
allocation fix and unchanged from the original gate-10 measurement.

**Original (pre-fix) gate-10 measurement**, for context:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 2049.08 | **939.76** | upstream 2.2× faster |
| p99 ns/op | 2536.66 | **1375.66** | upstream 1.8× faster |
| min ns/op | 1677.52 | **828.60** | upstream 2.0× faster |

Checksum `723901217380`, identical on both sides for the original gate-10 measurement.

**Per-method isolation probe** (200,000 calls of each method alone, same tree, both sides),
pre-fix: `nearest_neighbor` alone is 331 ns/call here against upstream's 620 ns; `k_nearest_neighbors`
alone is 6.6 µs/call here against upstream's 2.1 µs.

**Allocation-counting probe** (`bench/runner/examples/kd_tree_alloc_probe.rs`, 100,000 scattered
2-D points, 20,000 calls of each method alone), pre-fix:

| method | ns/call | allocations/call | bytes/call |
|---|---|---|---|
| `nearest_neighbor` | 362.9 | **0.000** | 0 |
| `k_nearest_neighbors` | 8071.0 | **499.312** | 12,863 |

Same probe, re-run unchanged post-fix:

| method | ns/call | allocations/call | bytes/call |
|---|---|---|---|
| `nearest_neighbor` | 352.4 | **0.000** | 0 |
| `k_nearest_neighbors` | 1909.3 | **7.000** | 1,392 |

499.3 allocations per call became 7.0 (a 71× reduction); time per call fell 4.2×, from 8,071 ns to
1,909 ns. The self-ratio (`k_nearest_neighbors` cost ÷ `nearest_neighbor` cost) fell from 22.2× to
5.4×, against upstream's own 3.4×.

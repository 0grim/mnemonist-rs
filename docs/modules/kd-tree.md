# kd-tree

Upstream: `kd-tree.js` (270 LOC) · `test/kd-tree.js` — **109 lines, 5 `it` blocks**.

Port: `crates/mnemonist-core/src/structures/kd_tree.rs`. Bridge:
`crates/mnemonist-napi/src/kd_tree.rs`. Shim: `tests/bridge/kd-tree.js`. Fuzz spec:
`crates/difffuzz/src/modules/kd_tree.rs`.

A k-dimensional tree: a flat binary tree over fixed-size numeric points, splitting on one axis per
depth (round-robin over `dimensions`) at the median of whatever points remain in the current
window, stored as `pivots`/`lefts`/`rights` typed arrays the same shape `vp-tree`'s are. Built once
via `.from`/`.fromAxes` — there is no `add`, and (unlike `vp-tree`) no directly usable raw
constructor either: `function KDTree(dimensions, build)` takes an already-built internal shape only
those two static factories ever produce. Three query methods, each a DFS with its own copy of the
"can the other subtree possibly contain something closer?" pruning test: `nearestNeighbor`
(recursive, single best), `kNearestNeighbors` (recursive, bounded by
`crate::structures::fixed_reverse_heap::FixedReverseHeap` — the exact module upstream's own
`kd-tree.js` requires for this), and `linearKNearestNeighbors` (a plain scan, no pruning at all —
the ground truth the other two are checked against).

---

## What upstream tests

Five `it` blocks, all against one fixture: six 2D points, each `[label, [x, y]]`.

* **`should be keep sane`** — `KDTree.from(DATA, 2)`, then every one of the six points fed back
  into `nearestNeighbor` (each must find itself), plus one query, `[8, 5]`, that is not in the
  data set at all and must resolve to `'two'`. `tree.pivots`/`.lefts`/`.rights` are then asserted
  against literal `Uint8Array`s — the tree's exact shape, pinned byte-for-byte, the same discipline
  `vp-tree`'s own constructor test applies.
* **`should be possible to build a KDTree directly from axes`** — `KDTree.fromAxes(axes, labels)`
  with the same data reshaped by hand; same assertions, confirming the two construction paths
  agree exactly.
* **`should be possible to build a KDTree from axes and without labels`** — `fromAxes` with
  `labels` omitted; `nearestNeighbor` must then return the point's *positional index* rather than
  its string label.
* **`should be possible to retrieve knn`** — for every point, `kNearestNeighbors(1, point)` must
  equal `nearestNeighbor(point)`, and `kNearestNeighbors(2, point)`/`(3, point)` must equal (as
  `Set`s, order-insensitive) a brute-force top-`k` computed by the test file's own `knn` helper.
* **`should be possible to retrieve knn linearly`** — `linearKNearestNeighbors(1, point)` must
  again equal `nearestNeighbor(point)` for every point, and one *order-sensitive* pinned case,
  `linearKNearestNeighbors(3, [8, 3])` must equal exactly `['five', 'four', 'one']` — the only
  assertion in the file that depends on the tie-break the heap's `[dist, i]` tuple comparator
  produces.

## What upstream does NOT test

**`k <= 0`.** `kNearestNeighbors`/`linearKNearestNeighbors` both open with `if (k <= 0) throw`;
nothing in the suite ever passes one.

**`k` past the tree's size**, beyond the trivial case (every query in the suite uses `k <= 3`
against a 6-point tree). `k = Math.min(k, this.size)` clamps it, but the suite never asks for more
neighbors than exist.

**An empty tree**, or a zero-dimensional one. Neither `.from([], n)` nor `.from(data, 0)` appears
anywhere.

**The "go the other way too" branch, proven necessary rather than merely present.** The one
cross-plane-shaped query in the suite, `[8, 5]` in `should be keep sane`, turns out — verified by
hand against the pinned `pivots`/`lefts`/`rights` — to resolve on the very first recursive step
(the root's primary descent lands directly on `'two'`'s leaf node, which is closer than anything
else in the tree), so it does not actually require backtracking into the other subtree at all. See
Bugs/gate 6 below: this was discovered only by trying to *falsify* that branch and watching the
pinned assertion stay green.

**A distance metric other than squared Euclidean.** `squaredDistanceAxes` is not parameterised —
nothing to test here upstream, and nothing to diverge from either.

## What we test in addition

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
| `finds_neighbors_across_the_splitting_plane` | the "proven necessary" gap above — see Bugs/gate 6 |

## Bugs this found

**None**, after one investigated false start. `kd-tree.js`'s recursive descent and its two
independent copies of the pruning bound (`nearestNeighbor`'s and `kNearestNeighbors`') were read
closely, ported faithfully, checked against both fixed fixtures byte-for-byte on the first attempt,
fuzzed for 90-plus seconds at two seeds (1.62M operations total, zero divergences), and put through
gate 6's falsification below — which stayed green on its first target and was investigated to a
cause rather than accepted or swapped out (see Fuzz + bench). No genuine defect surfaced anywhere.
Reported plainly, per this project's "set — no upstream bugs, and that is the finding" precedent.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-406 | **The bridge exposes no direct constructor** — only `.from`/`.fromAxes`. | Upstream's raw `function KDTree(dimensions, build)` takes an already-built internal shape nothing but those two factories ever produces; no test calls it directly, and narrowing to how the module is actually used avoids inventing a bridge for a shape nothing needs. |
| D-407 | **An empty tree's `nearestNeighbor` returns `None`/`undefined`** rather than cascading through `undefined` arithmetic. | Untested upstream; the cascade is entirely internal (unlike `vp-tree`'s D-401, no caller-supplied function is involved), so there is no ambiguity to preserve by reproducing it. |
| D-408 | **`k <= 0`'s guard only fires for `k == 0`.** | `usize` cannot carry a negative value at all; the untested `NaN` fall-through (`NaN <= 0` is `false` in JS, so it does *not* throw and instead cascades) is not reproduced either, for the same reason. |
| D-409 | **`dimensions == 0` fails differently on each side** (panic here, `NaN`-cascade upstream) rather than being reconciled. | No test constructs one; there is no real "right answer" in upstream's own silent-garbage output to reproduce, so this is disclosed as a known gap instead of papered over with an invented guard. |
| D-410 | **`axes`/`labels`/`this.visited` are not exposed by the bridge.** | No test reads any of the three; `axes`/`labels` are reconstructable from the constructor arguments a caller already has, and `visited` is a diagnostic aside the fuzz harness measures a different way instead (see Fuzz, below). |

Full detail, `Status`/`Category`/`Verify` fields, for every entry: `planning/DECISIONS-CANDIDATES.md`
(D-406 through D-410; D-400 through D-405 belong to `vp-tree`, above).

## Fuzz + bench

### Fuzz

```
module=kd-tree seed=42  cases=9713  ops=970441  wall=90.0s  divergences=0
module=kd-tree seed=7   cases=6462  ops=650306  wall=60.0s  divergences=0
```

**1.62M operations across two seeds, zero divergences.** Reproduce with `target/release/difffuzz
--module kd-tree --seed 42 --cases 9713` (or `--seed 7 --cases 6462`).

* **Op alphabet:** `nearestNeighbor(query)` (weight 3) · `kNearestNeighbors(k, query)` (3) ·
  `linearKNearestNeighbors(k, query)` (3).
* **`.from`, not `new KDTree(...)`.** Upstream's own raw constructor takes an already-built
  internal shape (D-406), so this is the first module in the port whose `ModuleSpec` needs an
  alternate entry point: `static_factory()` names `"from"`, and `fuzz/oracle.js`'s `init` case
  grew an additive `staticFactory` field (`Ctor[name](...)` instead of `new Ctor(...)`) —
  optional, defaulted to the prior behaviour for every other module.
* **A dense 12×12 integer grid, not a sparse or wide-ranging one.** CLAUDE.md names the sharp risk
  for this module by name — "queries whose nearest neighbor lies across a splitting plane from the
  query point ... precisely the case a naive implementation gets wrong" — and a dense grid is the
  direct answer: many points share a coordinate on whichever axis the tree splits on (which is
  what puts a query close to a plane at all), and many points sit at genuinely equal squared
  distance from a query (forcing `kNearestNeighbors`' `[dist, visited++, pivot]` tie-break to
  actually run). Query points are drawn from a *wider* window than the grid
  (`-6..18` against a `0..12` grid) so some land well outside the point cloud and some deep inside
  it.
* **Observable state: `size`, `dimensions`, `pivots`, `lefts`, `rights`** — the tree's exact shape,
  compared on every generated construction.

**Measured evidence of splitting-plane crossings and genuine ties** (`cargo test -p difffuzz --lib
grammar_self_check_queries_land_across_the_splitting_plane -- --nocapture`, 500 sampled queries
against a random 60-point instance of the grid):

```
kd-tree grammar_self_check: 191/500 queries had their true nearest neighbor across a naive
single-axis split; 100/500 had a genuine distance tie
```

191 of 500 queries have a true nearest neighbor that a "just trust the first split" implementation
would miss entirely; 100 of 500 have more than one point at the exact minimum distance. Both of the
risks CLAUDE.md names for this module are measured occurring directly, not inferred from op
weights.

### Falsification of the port (gate 6)

**Named first:** two targets, in order.

**Target 1 (as originally named): `builds_the_tree_upstream_pins`'s
`assert_eq!(tree.nearest_neighbor(&[8.0, 5.0]), Some(&"two"))`.** **The sabotage:**
`recurse_nearest`'s "go the other way too" branch (`if dx * dx < *best_distance`) forced to `false`
unconditionally, disabling backtracking into the sibling subtree entirely.

**Stayed green.** Investigated rather than swapped for an easier target: by hand, against the
tree's own pinned `pivots = [5, 1, 0, 3, 2, 4]`/`lefts = [2, 3, 0, 0, 6, 0]`/`rights = [5, 4, 0, 0,
0, 0]`, the root's primary descent for query `[8, 5]` goes directly to node 4 (`'two'` at `(9,
6)`), a leaf, on the very first recursive call — before the disabled branch is ever consulted. The
assertion is not incapable of expressing the defect (contrast the three green falsifications
`docs/METHODOLOGY.md` §5 catalogues, none of which are this); this specific query, against this
specific six-point tree, simply does not exercise the branch at all. A first attempt at a stronger
native regression test (`finds_neighbors_across_the_splitting_plane`, a 64-point diagonal line)
*also* stayed green under the same sabotage — investigated to the same root cause: every point's
`x` equalling its `y` lets the primary "trust the split" descent converge on the right answer by
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

**What this is a finding about:** neither the pinned upstream fixture nor a naively-adversarial
native test (a line) is dense enough to force the one query pattern this branch exists for. The
project's differential fuzzer, built specifically because a fixed fixture and a hand-written test
share the same author's blind spots, caught what both missed on the first try — the clearest
demonstration in this port so far that a native unit test and a differential fuzz campaign are not
redundant instruments, even when both are written by the same person who wrote the code under test.

### Bench

`bench/results.json` → `modules["kd-tree"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e5`** — 1e6 mixed `nearestNeighbor`/`kNearestNeighbors` (75/25) over 100,000 scattered
2-D points (coordinates from a fixed-seed shuffle, not `0..size` on one axis — construction sorts
each level's window by raw axis value, the same fixed-pivot quicksort weak spot `vp-tree.rs`
documents). No `add`: the tree is built once, untimed. A single query shape already exercises both
outcomes of the cross-plane backtrack for real 2-D data, so no second radius parameter was needed
(contrast `bk-tree`/`vp-tree`). xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 2049.08 | **939.76** | upstream 2.2× faster |
| p99 ns/op | 2536.66 | **1375.66** | upstream 1.8× faster |
| min ns/op | 1677.52 | **828.60** | upstream 2.0× faster |
| RSS delta MB | **9.6** | 51.3 | |
| structure-only RSS delta MB | **0.2** | 6.0 | |
| startup ms | **0.6** | 15.3 | 26× (reported separately; not throughput) |

**A real, measured loss on p50/p99/min — this batch's sharpest.** Isolated with a standalone probe
(200,000 calls of each method alone, same tree, both sides): `nearest_neighbor` alone is 331 ns/call
here against upstream's 620 ns (the port wins, consistent with the rest of this batch), but
`k_nearest_neighbors` alone is 6.6 µs/call here against upstream's 2.1 µs — a genuine reversal, and
disproportionate: this port's own k-NN path costs **20×** its own `nearest_neighbor`, where
upstream's costs only **3.4×** its own. **Cause: unconfirmed.** `recurse_knn` heap-allocates a
fresh 3-element `Vec<f64>` per node visited into `FixedReverseHeap`'s backing store — a plausible
mechanism (V8's generational GC can bump-allocate the equivalent short-lived array far more cheaply
than repeated small `malloc`s), consistent with where the two sides' costs diverge, but not
confirmed with a profiler or allocation count, so it is labelled a hypothesis rather than a finding
— see `bench/runner/src/kd_tree.rs`'s own module docs for the full account. RSS and startup still
favour the port. Checksum `723901217380`, identical on both sides.

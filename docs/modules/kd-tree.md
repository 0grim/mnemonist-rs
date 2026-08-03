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
"Fuzz + bench" below: this was discovered only by trying to *falsify* that branch and watching the
pinned assertion stay green.

**A distance metric other than squared Euclidean.** `squaredDistanceAxes` is not parameterised —
nothing to test here upstream, and nothing to diverge from either.

## What we test in addition

`crates/mnemonist-core/src/structures/kd_tree.rs` — 9 tests: a 1:1 transcription of all five
upstream blocks as a baseline, `k` clamped to size rather than padding with nothing, a zero `k`
rejected with upstream's message (narrowed — see D-408), an empty tree building cleanly and
answering no queries (D-407), and a dedicated cross-splitting-plane test — see "Fuzz + bench" for
why that last one needed a dense grid rather than a hand-picked shape to actually exercise the
branch it is named for.

## Bugs this found

**None**, after one investigated false start. `kd-tree.js`'s recursive descent and its two
independent copies of the pruning bound (`nearestNeighbor`'s and `kNearestNeighbors`') were read
closely, ported faithfully, checked against both fixed fixtures byte-for-byte on the first attempt,
fuzzed for 90-plus seconds at two seeds (1.62M operations total, zero divergences), and put through
gate 6's falsification below — which stayed green on its first target and was investigated to a
cause rather than accepted or swapped out (see "Fuzz + bench"). No genuine defect surfaced anywhere.
Reported plainly, per this project's "set — no upstream bugs, and that is the finding" precedent.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-406 | **The bridge exposes no direct constructor** — only `.from`/`.fromAxes`. | Upstream's raw `function KDTree(dimensions, build)` takes an already-built internal shape nothing but those two factories ever produces; no test calls it directly, and narrowing to how the module is actually used avoids inventing a bridge for a shape nothing needs. |
| D-407 | **An empty tree's `nearestNeighbor` returns `None`/`undefined`** rather than cascading through `undefined` arithmetic. | Untested upstream; the cascade is entirely internal (unlike `vp-tree`'s D-401, no caller-supplied function is involved), so there is no ambiguity to preserve by reproducing it. |
| D-408 | **`k <= 0`'s guard only fires for `k == 0`.** | `usize` cannot carry a negative value at all; the untested `NaN` fall-through (`NaN <= 0` is `false` in JS, so it does *not* throw and instead cascades) is not reproduced either, for the same reason. |
| D-409 | **`dimensions == 0` fails differently on each side** (panic here, `NaN`-cascade upstream) rather than being reconciled. | No test constructs one; there is no real "right answer" in upstream's own silent-garbage output to reproduce, so this is disclosed as a known gap instead of papered over with an invented guard. |
| D-410 | **`axes`/`labels`/`this.visited` are not exposed by the bridge.** | No test reads any of the three; `axes`/`labels` are reconstructable from the constructor arguments a caller already has, and `visited` is a diagnostic aside the fuzz harness measures a different way instead (see Fuzz, below). |

## Fuzz + bench

### Fuzz

**1.62M operations across two seeds, zero divergences**:

```
module=kd-tree seed=42  cases=9713  ops=970441  wall=90.0s  divergences=0
module=kd-tree seed=7   cases=6462  ops=650306  wall=60.0s  divergences=0
```

Reproduce with `target/release/difffuzz --module kd-tree --seed 42 --cases 9713` (or
`--seed 7 --cases 6462`).

The op alphabet covers `nearestNeighbor`/`kNearestNeighbors`/`linearKNearestNeighbors`, all
constructed via `.from`, not `new KDTree(...)` — upstream's own raw constructor takes an
already-built internal shape (D-406), so this was the first module in the port whose `ModuleSpec`
needed an alternate entry point. Points are a dense 12×12 integer grid rather than a sparse or
wide-ranging one: many points share a coordinate on whichever axis the tree splits on (which is
what puts a query close to a plane at all), and many points sit at genuinely equal squared distance
from a query (forcing `kNearestNeighbors`' tie-break to actually run). Query points are drawn from a
*wider* window than the grid so some land well outside the point cloud and some deep inside it.
Observable state is `size`, `dimensions`, `pivots`, `lefts`, `rights` — the tree's exact shape,
compared on every generated construction. Full grammar: evidence file.

**Measured evidence of splitting-plane crossings and genuine ties** (500 sampled queries against a
random 60-point instance of the grid): 191 of 500 queries have a true nearest neighbor that a "just
trust the first split" implementation would miss entirely, and 100 of 500 have more than one point
at the exact minimum distance. Both named risks for this module are measured occurring directly, not
inferred from op weights. Full figures: evidence file.

**Falsification of the port (gate 6), two targets in order.** The first sabotage — disabling
`recurse_nearest`'s backtracking branch entirely — stayed green against both the pinned upstream
fixture assertion and a first native regression attempt (a 64-point diagonal line), and was
investigated rather than swapped for an easier target: by hand, against the tree's own pinned
shape, the query used resolves on the very first recursive call, before the disabled branch is ever
consulted, and every point on a diagonal line lets the primary descent converge coordinate by
coordinate without needing the other side either. The second target — the differential fuzzer's own
`grammar_self_check`, whose dense-grid construction is exactly what produces splitting-plane
crossings — is confirmed red against the same sabotaged core; a rebuilt native test on the same kind
of dense grid (10×10, 200 sampled queries) is then also confirmed red at the intended assertion.
Reverted; confirmed green again across all 9 unit tests and the full workspace suite. What this is a
finding about: neither the pinned upstream fixture nor a naively-adversarial native test (a line) is
dense enough to force the one query pattern this branch exists for — the differential fuzzer caught
what both missed on the first try, the clearest demonstration in this port so far that a native unit
test and a differential fuzz campaign are not redundant instruments, even when both are written by
the same person who wrote the code under test. Full record: evidence file.

### Bench

`bench/results.json` → `modules["kd-tree"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e5`** — 1e6 mixed `nearestNeighbor`/`kNearestNeighbors` (75/25) over 100,000 scattered
2-D points (coordinates from a fixed-seed shuffle, not `0..size` on one axis — construction sorts
each level's window by raw axis value, the same fixed-pivot quicksort weak spot `vp-tree.rs`
documents). No `add`: the tree is built once, untimed. A single query shape already exercises both
outcomes of the cross-plane backtrack for real 2-D data, so no second radius parameter was needed
(contrast `bk-tree`/`vp-tree`): the port is now 1.23× faster at p50 (755.10 vs 927.85 ns/op). Full
current table and the fix that got it there: evidence file and log.

`k_nearest_neighbors` used to heap-allocate a fresh 3-element `Vec<f64>` per node visited into
`FixedReverseHeap`'s backing store; `k_nearest_neighbors` and `linear_k_nearest_neighbors` now hold
`[f64; 3]` and `[f64; 2]` instead, which are `Copy`, so the sift-step clone is a stack copy rather
than a `malloc`. `TupleComparator` gained a matching `Comparator<[T; N], E>` impl, appended beside
the existing `Vec<T>` one rather than replacing it — the two are the same lexicographic rule, and
since every tuple here is exactly `N` long, the "shorter than `N`" case only the `Vec` impl needs is
unreachable from either. Full before/after allocation counts and the investigation history: log.

One caution against reading the current 1.23× as the full story: back-to-back measurements of the
identical code on the same host have shown up to 22% run-to-run drift, so the honest statement of
the improvement this fix produced is the port's own absolute time — roughly 2049 ns down to 755 ns —
rather than the ratio against upstream, which carries that drift. RSS and startup favour the port
throughout and are unaffected by any of this: 9.6 MB against 51.3 MB, and a 26× faster startup.

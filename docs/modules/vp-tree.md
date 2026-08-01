# vp-tree

Upstream: `vp-tree.js` (241 LOC) · `test/vp-tree.js` — **235 lines, 7 `it` blocks**.

Port: `crates/mnemonist-core/src/structures/vp_tree.rs`. Bridge:
`crates/mnemonist-napi/src/vp_tree.rs`. Shim: `tests/bridge/vp-tree.js`. Fuzz spec:
`crates/difffuzz/src/modules/vp_tree.rs`.

A vantage-point tree: a flat binary tree over a caller-supplied metric, storing the tree as four
parallel typed arrays exactly the way `bk-tree`'s sibling modules do it — `nodes` (item index),
`lefts`/`rights` (child node index + 1, `0` meaning "no child"), `mus` (the split radius). Built
once, at construction (`createBinaryTree`); there is no `add`. Every node picks a vantage point,
computes its distance to every remaining item, sorts by that distance, and splits at the median
(`mu`) into "closer than `mu`" (left) and "farther than `mu`" (right). A query (`nearestNeighbors`/
`neighbors`) walks the tree with a running bound and prunes a subtree the instant the triangle
inequality proves it cannot contain anything better than what has already been found — that pruning
decision, and whether it goes both ways, is the entire point of this module.

---

## What upstream tests

Seven `it` blocks:

* **Constructor validation**: `new VPTree(null)` throws matching `/distance/`; `new
  VPTree(Function.prototype)` (no items) throws matching `/items/`.
* **`should properly build the tree`** — the one test in this whole port that pins a
  vantage-point tree's *entire* internal shape byte-for-byte: 15 words, `levenshtein` distance,
  `tree.nodes`/`.lefts`/`.rights`/`.mus` each asserted against a literal `Uint8Array`/`Float64Array`.
  This is a stronger construction check than `bk-tree`'s own suite has room for, because `bk-tree`
  has no equivalent public shape to assert against.
* **`should also work in the worst case scenario`** — 8 items with heavy duplication (`abc` three
  times, `bde` twice, `cd` twice), an `identity` metric (`+(a !== b)`, i.e. `0` or `1` only) that
  forces constant ties, again asserted against the exact `nodes`/`lefts`/`rights`/`mus` arrays.
* **`should be possible to find the k nearest neighbors`** — `nearestNeighbors(2, 'look')` and
  `nearestNeighbors(5, 'look')` against the 15-word set, both asserted with `deepStrictEqual`
  against the *exact order* of the returned array, including a tie at distance 1 (`lock` before
  `book`) and a tie at distance 3 (`mack` before `back`).
* **`should be possible to find every neighbor within radius`** — `neighbors(2, 'look')` and
  `neighbors(3, 'look')`, checked via a helper that serializes each result to a
  `distance§item` string and compares as a `Set` — order-insensitive, unlike the k-NN test above.
* **`should be possible to create a tree from an arbitrary iterable`** — `VPTree.from(new
  Set(WORDS), levenshtein)`.
* **`should be possible to insert arbitrary items in the tree`** — items are `{value: '...'}`
  objects, distance unwraps `.value`; confirms the tree never inspects an item beyond what the
  distance function does with it.
* **`should return all nearest neighbors correctly (issue #147)`** — a regression test for a real
  upstream GitHub issue: two 2D points equidistant-and-identical (`[100, 100]` three times),
  `nearestNeighbors(3, [100, 100])` must return all three at distance `0`, not silently deduplicate
  or drop one via a heap that mishandles a full tie at the boundary.
* **`should work with medium scale random data`** — 10,000 random 3D vectors, `neighbors(50,
  query)` compared against a linear brute-force scan over the whole set. The only property-style
  test in the file, and the only one whose distance metric (`euclid2d`, real Euclidean distance)
  produces a genuinely continuous, rarely-tied distance space.

## What upstream does NOT test

**A distance function that throws.** Both `createBinaryTree` (at construction) and both query
methods call `this.distance(...)` from deep inside a loop; nothing in the suite ever provides a
metric that can fail, so no test observes what happens to a partially built tree, or to a
partially accumulated result array, when it does.

**`k` past the tree's size in `nearestNeighbors`.** Unlike `kd-tree.js`'s `kNearestNeighbors`
(which clamps `k = Math.min(k, this.size)`), `vp-tree.js` never clamps here at all — a `k` larger
than the tree holds simply never triggers the heap's trim, so every item comes back. No test asks
for more neighbors than exist.

**`k == 0`, or a query against an empty tree.** Neither is reachable from any `it` block; see
Deliberate divergences for what each does upstream.

**A distance function that mutates the tree, or calls back into it.** `this.heap`/`this.D` are
single instance fields reused across calls (see D-403); nothing in the suite's metrics has a
reason to reach into `this`.

**Exact pruning-branch coverage on the pinned fixtures.** `should properly build the tree` and
`should also work in the worst case scenario` pin construction; the k-NN/radius tests query real
data, but with only 15 (or 8) items and a handful of fixed queries, there is no guarantee — and no
assertion either way — that every `mus`-comparison branch in `nearestNeighbors`/`neighbors` gets
exercised by the suite as a whole. That is exactly what `grammar_self_check` in the Fuzz section
below measures directly instead of assuming.

## What we test in addition

`crates/mnemonist-core/src/structures/vp_tree.rs` — 9 tests:

| Test | Closes gap |
|---|---|
| `builds_the_tree_upstream_pins` | 1:1 transcription of the pinned 15-word construction |
| `builds_the_worst_case_tree_upstream_pins` | 1:1 transcription of the 8-item duplicate-heavy case |
| `finds_the_k_nearest_neighbors_in_the_upstream_order` | 1:1 transcription of both k-NN calls, order included |
| `finds_every_neighbor_within_radius` | 1:1 transcription of both radius calls |
| `returns_every_neighbor_at_zero_distance` | issue #147, both of its cases |
| `an_empty_tree_builds_cleanly_and_answers_no_queries` | the empty-tree gap, resolved as D-401 |
| `a_failing_distance_during_construction_leaves_no_tree_behind` | the throwing-distance gap, construction side |
| `a_failing_distance_during_a_query_propagates` | the throwing-distance gap, query side |
| `pruning_goes_both_ways_across_radii` | confirms a radius of `0` prunes strictly more than an unbounded one on the pinned 15-word tree |

The `k == 0` gap is closed by inspection (`try_nearest_neighbors` returns early) rather than by a
dedicated assertion beyond what `pruning_goes_both_ways_across_radii`'s neighbor already implies;
the reentrancy gap is a documented, deliberately unclosed divergence (D-403) — see below.

## Bugs this found

**None.** `vp-tree.js` is a careful, non-trivial implementation — the flat-array construction with
its `hi--`/median-interpolation arithmetic, and the dual-condition pruning bound in both query
methods, are exactly the kind of code most likely to hide an off-by-one. Neither close reading
during the port, the two upstream-pinned construction fixtures (both matched byte-for-byte on the
first attempt), 90-plus seconds of differential fuzzing at two seeds (1.37M operations total, zero
divergences), nor the gate 6 falsification below surfaced a genuine defect. Reported plainly, per
this project's own precedent (`planning/NOTES.md`'s "set — no upstream bugs, and that is the
finding").

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-400 | **Distance is passed per call, never stored on the struct.** | Same reasoning as `bk_tree.rs`: the JS callback belongs at the boundary. |
| D-401 | **An empty tree's query returns no results** rather than crashing the caller's own distance function on an `undefined` vantage point. | No test builds an empty tree and queries it; the crash this would reproduce lives entirely in a caller-supplied metric, with no single "correct" answer to pick. |
| D-402 | **`nearestNeighbors(0, query)` returns no results** rather than reading `undefined.distance`. | Untested; the crash is `mnemonist`'s own arithmetic, not a caller's. |
| D-403 | **A reentrant distance function sees independent state, not upstream's shared `this.heap`/`this.D`.** | This port's queries hold no tree-wide mutable state to protect (see D-404), so a reentrant call is simply a second, correct query rather than an interleaved, corrupted one — *more* correct than upstream, and disclosed as CLAUDE.md requires rather than left implicit. |
| D-404 | **The napi bridge holds no `RefCell` at all.** | Unlike `bk_tree.rs`, no method ever needs exclusive access post-construction — stated to make D-403's mechanism legible. |
| D-405 | **`this.D` is not exposed by the bridge.** | No test reads it; the same information (whether a query pruned anything) is measured directly in the fuzz harness instead by wrapping the distance function with a counter. |

Full detail, `Status`/`Category`/`Verify` fields, for every entry: `planning/DECISIONS-CANDIDATES.md`
(D-400 through D-405; D-406 onward belong to `kd-tree`, below).

## Fuzz + bench

### Fuzz

```
module=vp-tree seed=42  cases=8256  ops=819286  wall=90.0s  divergences=0
module=vp-tree seed=7   cases=5532  ops=552919  wall=60.0s  divergences=0
```

**1.37M operations across two seeds, zero divergences.** Reproduce with `target/release/difffuzz
--module vp-tree --seed 42 --cases 8256` (or `--seed 7 --cases 5532`).

* **Op alphabet:** `nearestNeighbors(k, query)` (weight 5) · `neighbors(radius, query)` (5).
* **Items and queries are integers in `0..24`**, with `distance(a, b) = |a - b|` — reusing
  `bk-tree`'s own `bkAbsDiff` oracle factory rather than adding a near-duplicate. This is the
  answer to the risk CLAUDE.md names for this module by name: a wide item range would make every
  distance from a vantage point distinct, so the median split would never have to choose between
  two *equal* distances, and the "genuine near-ties" this module's brief demands would never occur.
  With up to 80 items packed into 24 distinct values, repeated collisions on the same distance from
  any given node are constant.
* **`neighbors`' radius is drawn across the whole possible span, `0..=24`**, specifically so a
  campaign's radii include both extremes.
* **Observable state: `size`, `nodes`, `lefts`, `rights`, `mus`** — the tree's exact shape,
  compared byte-for-byte on every one of the thousands of randomly generated constructions in a
  campaign, not only the two fixed fixtures the native tests above pin. This is stronger
  construction coverage than `bk-tree`'s campaign has room for, because `VPTree` exposes real
  getters for its internal arrays where `BKTree` has none.
* **Deliberately excluded:** a throwing distance function (`|a-b|` cannot throw) and non-integer
  items — both covered by native tests instead, per Deliberate divergences.

**Measured evidence that near-ties occurred and the pruning decision went both ways** (`cargo test
-p difffuzz --lib grammar_self_check_radius_spans_full_pruning_and_none -- --nocapture`, an
80-item tree built from the same narrow-range distribution, every `(radius, query)` pair the
grammar's own ranges cover, distance calls counted directly):

```
vp-tree grammar_self_check: 396/600 queries pruned at least one node; 204/600 visited every node
(radius large enough that no pruning was possible)
```

396 of 600 sampled queries took the "skip this subtree" branch at least once; the other 204 took
the "search everything" path with zero pruning possible at all — both branches of the pruning
decision are live, not merely reachable in principle.

### Falsification of the port (gate 6)

**Named first:** `finds_the_k_nearest_neighbors_in_the_upstream_order`'s
`assert_eq!(neighbors, vec![... "lock", "book", "bock", "mack", "back" ...])` for
`nearestNeighbors(5, 'look')`.

**The sabotage:** in `try_nearest_neighbors`'s `d < mu` branch, the push order onto the traversal
stack was reversed — pushing the right subtree before the left, rather than left-before-right (the
`else` branch already pushes right-before-left, so this made both branches push in the same
order).

**Confirmed red**, at exactly the named assertion — and more sharply than a mere reordering: the
sabotaged run returned a *different result set*, not just a different order (`shock` in place of
`mack`), because visiting subtrees in a different sequence changes how `tau` (the running best-`k`
bound) tightens, which changes which later subtrees get pruned. Reverted; **confirmed green
again**: all 9 `vp_tree` unit tests pass, `cargo test --workspace` clean.

This is the sharpest kind of gate 6 result available: the sabotaged assertion did not merely
disagree on tie order, it proved that traversal order is load-bearing for *correctness*, not only
for which of several equally-valid ties comes first.

### Bench

**Not run.** Gate 10 needs an idle machine and is batched into a separate quiet pass (§7.3); this
unit is deliberately not in `tests/scope.txt` until then. Gates 1–9 are green.

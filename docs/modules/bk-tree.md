# bk-tree

Upstream: `bk-tree.js` (180 LOC) · `test/bk-tree.js` — **82 lines, 6 `it` blocks, ~10 assertion
statements**.

Port: `crates/mnemonist-core/src/structures/bk_tree.rs`. Bridge:
`crates/mnemonist-napi/src/bk_tree.rs`. Shim: `tests/bridge/bk-tree.js`. Fuzz spec:
`crates/difffuzz/src/modules/bk_tree.rs`.

A Burkhard-Keller tree: every node is an item plus a table of children keyed by their **distance**
from that item, where distance comes from a caller-supplied metric. `add` walks down by distance
until it finds an empty slot; `search` is a bounded DFS that only descends into a child whose
distance key falls in `[d - n, d + n]`. Not a `Map`-backed module — `node.children` is a plain object
keyed by the numeric distance, and nothing here ever enumerates it, only probes individual,
known-in-advance keys — so this is the first module in the port that is a genuine tree shape rather
than an `OrderedMap`.

---

## What upstream tests

Six `it` blocks, all against `leven`'s Levenshtein distance over short lowercase words:

* **Constructor validation**: `new BKTree(null)` throws, matching `/distance/`.
* **`add`**, three items, checked only through `size`.
* **`clear`**, checked only through `size`.
* **`search`**, the file's one substantive block: three items added, then `search(1, 'mello')`
  (one hit) and `search(2, 'mello')` (two hits, **in a specific order** — `hello` at distance 1
  before `yellow` at distance 2). This is the only assertion in the file that depends on traversal
  order, and it is `deepStrictEqual` against the whole array, so the order is genuinely pinned, not
  incidental.
* **Arbitrary objects** — the same two `search` calls, with `{value: '...'}` items and a distance
  function that unwraps `.value`. Confirms the tree never inspects `item` itself.
* **`BKTree.from`**, two items from an array, checked only through `size`.

## What upstream does NOT test

**Any collision at distance 0.** No test ever adds two items whose mutual distance is `0` — every
example word in the file is distinct from every other by construction. `add`'s loop has no special
case for `d === 0`; a second item at distance 0 from an existing node becomes that node's `children[0]`
child, same as any other distance, but nothing confirms it.

**The search order beyond one node's children.** The one order-sensitive test has exactly one node
(the root) with two children at distinct distances. Nothing confirms the ascending-push/
descending-pop rule recurses correctly into a *grandchild*'s own children, or interacts with a
sibling subtree explored earlier on the stack.

**Never called or reached at all:**

1. **`search` on an empty tree** (`this.root` falsy) — the `if (!this.root) return [];` guard.
2. **A negative, fractional, or `NaN` distance.** Every distance function in the file — `leven`
   included — returns a non-negative integer; nothing exercises what upstream does with anything
   else (it would coerce the value into an object-property key via `ToPropertyKey`, which
   stringifies *anything*).
3. **A distance function that throws.** Both `this.size++`/`node.children[d] = ...` in `add`, and
   the whole traversal in `search`, are textually after the `this.distance(...)` call that can
   throw — nothing confirms the tree is left untouched, or that a throw mid-search discards the
   partial `found` array.
4. **`n` as anything but a small positive integer.** `search`'s `n` is used directly as a loop bound
   (`for (i = d - n, l = d + n + 1; ...)`); a huge or negative `n` is never passed.
5. **`toJSON()`** (returns `this.root` — a real object, not opaque) and **`inspect()`**.
6. **Duplicate items** at the *same* resolved slot — `size` counts every successful insert, so two
   identical items land at two different nodes (whichever distance-0 slot is free at each level);
   this is implied by the module's design but never asserted.

## What we test in addition

`crates/mnemonist-core/src/structures/bk_tree.rs` — 8 tests: a 1:1 reproduction of all five
substantive upstream blocks, a search with no root returning nothing, search visiting
higher-distance children before lower-distance ones (four nodes at four distinct distances from the
root, confirming the rule holds one level deeper than upstream's own test reaches), a failing
distance leaving `add` with no trace whether it fails on the first call or during descent, the
search-side mirror discarding the partial result, the same item added twice both being counted, and
`clear` resetting `size` and forgetting every node.

Gaps 2, 4 and 5 are stated rather than closed: 2 and 4 are the bridge's documented narrowing (a real
`f64` distance and `n`, not upstream's implicit `ToPropertyKey` string coercion) — see Deliberate
divergences; 5 has no upstream assertion anywhere.

## Bugs this found

**None.** `bk-tree.js` is a small, careful implementation; nothing in the port's reading of it, its
native tests, or 1.3M clean fuzz operations surfaced a genuine upstream defect. This is a real
finding in its own right for a require-closure this size: not every unit hides a bug, and reporting a
clean result plainly is part of the porting discipline this project holds itself to (see
`docs/modules/set.md`, which reports the identical outcome for that module).

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-300 | **Not a T3 module — no `Map`, no `OrderedMap`.** | `node.children` is a plain `HashMap<i64, Node<I>>` here, matching upstream's plain-object-keyed-by-number exactly: `add` does one `get`/`insert` at an exact distance, `search` probes a bounded numeric range one value at a time, and nothing ever iterates the *keys* of a children table. No ordering machinery is needed because upstream's own algorithm never needs one either. |
| D-301 | **`distance` is fallible at the core level.** | `try_add`/`try_search` take `FnMut(&I, &I) -> Result<i64, E>` so a JS distance function that throws propagates as a real `Err`, leaving the tree exactly as it was — both of upstream's mutations are textually after the call that can throw, in every path through both loops. `add`/`search` are the infallible convenience for a Rust caller whose distance cannot fail. |
| D-302 | **The bridge refuses a distance function that re-enters the tree, rather than serving it half-built state.** | `distance` is called from *inside* both `add`'s descent and `search`'s traversal, holding the bridge's `RefCell` borrow for the whole call — the same shape as `bit_vector`'s growth-policy re-entrancy (B-31). A distance function that calls back into the same tree meets that outstanding borrow and gets a catchable `REENTRANT_DISTANCE` error. Upstream would instead serve such a call from a tree mid-traversal and get whatever half-built state it finds. Narrower than upstream, and recorded rather than hidden — the same trade `bit_vector.rs` makes. |
| D-303 | **`n` and `distance`'s return value are `i64`/`f64`, not upstream's implicit string-keyed coercion.** | No test anywhere gives `distance` a reason to return anything but a small non-negative integer; reproducing `ToPropertyKey`'s full stringification would need a string-keyed children table for a case no test can observe. Stated as a narrowing rather than silently mismodelled. |
| D-304 | **`toJSON()`/`inspect()` are not ported.** | Node/JSON display conveniences with no upstream assertion. |
| D-305 | **The fuzz grammar excludes a throwing distance and string/object items.** | `Math.abs` (this grammar's distance) cannot throw, so the fallible path is covered by `mnemonist_core::structures::bk_tree`'s own native tests instead, which control the failure directly rather than hoping a generated program provokes it. Integers keep the metric a one-line, unmistakably-correct mirror on both sides; `mnemonist_napi::bk_tree`'s bridge is exercised against strings and `levenshtein` by the original suite, and against `Item` objects by core's own tests. |

## Fuzz + bench

### Fuzz

**1.33M operations, zero divergences:**

```
module=bk-tree seed=42  cases=13238  ops=1325206  wall=90.0s  divergences=0
```

Reproduce with `target/release/difffuzz --module bk-tree --seed 42 --cases 13238`.

The op alphabet covers `add`/`search`/`clear`. Items and queries are small integers in a 12-wide
range, with `distance(a, b) = |a - b|` (a real metric, cheap to mirror identically on both sides).
This is the deliberate answer to the sharp risk named for this module: a wide or random alphabet
would generate a stream of unrelated items that never collide on distance, and a tree that never
grows past one child per node tests almost nothing about the algorithm. Twelve values is narrow
enough that repeated `add`s land on the same distance from any given node constantly, which is the
*only* way a node ever grows more than one child. `search`'s radius reaches up to twice the item
range, so most calls visit the whole tree — standing in for an observation of `root`'s shape, which
core exposes no direct equivalent of. Observable state is `size` only, deliberately thin —
`search`'s return value, run after every op with a wide radius, reveals the same information `root`
would. Deliberately excluded: a throwing distance function (`Math.abs` cannot throw) and
non-integer items — both covered by native tests instead.

**Falsification of the port (gate 6):** the assertion named first was
`search_visits_higher_distance_children_before_lower_distance_ones`'s
`assert_eq!(order, vec!["aaaa", "bbba", "bbaa", "baaa"])`. The sabotage — reversing the child-probing
loop in `try_search` from ascending to descending, reversing which children a LIFO stack pops first
— is confirmed red at exactly the named assertion, with exactly the predicted reversed order
(`["aaaa", "baaa", "bbaa", "bbba"]` against `["aaaa", "bbba", "bbaa", "baaa"]`). Reverted; confirmed
green again (all 8 `bk_tree` unit tests pass, `cargo test --workspace` clean).

### Bench

`bench/results.json` → `modules["bk-tree"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-3e5`** — 1e6 mixed `add`/`search` (50/25/25) over a 300,000-item domain, metric
`distance(a, b) = |a - b|` rather than upstream's own Levenshtein: a BK-tree is metric-agnostic, and
a numeric metric both sides compute identically carries zero risk of a Levenshtein port drifting
apart in an edge case. The load-bearing split is the search radius: 25% of ops search at a small
radius that mostly prunes, 25% at a larger radius that mostly descends — both exercised on the same
tree built by the `add` stream, not two benchmarks glued together. The domain size (300,000) was
chosen by ruling out two failure modes first — see the log for the measurements that ruled out
2,000 (too small) and 1,000,000 (too large). xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **3531.49** | 6855.01 | 1.9× faster |
| p99 ns/op | **8870.66** | 17500.31 | 2.0× faster |
| RSS delta MB | **106.3** | 522.9 | |
| structure-only RSS delta MB | **0.1** | 6.9 | |
| startup ms | **0.6** | 15.0 | 26× (reported separately; not throughput) |

**No regressions.** Checksum `9589167`, identical on both sides — both trees pruned and descended
at the same points for the same queries.

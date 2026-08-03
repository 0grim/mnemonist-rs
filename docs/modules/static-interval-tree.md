# static-interval-tree

Upstream: `static-interval-tree.js` (387 LOC) + `utils/typed-arrays.js` ·
`test/static-interval-tree.js` — **95 lines, 4 `it` blocks, 13 assertion statements**.

Port: `crates/mnemonist-core/src/structures/static_interval_tree.rs`.
Bridge: `crates/mnemonist-napi/src/static_interval_tree.rs`.
Shim: `tests/bridge/static-interval-tree.js`.

**A read-only module, stated up front:** neither public query method mutates a built tree, so
`observe()`'s state (`size`/`height`/`tree`/`augmentations`) never changes after construction. The
entire signal both the native tests and the fuzz campaign carry is in each query's **result** —
the matched intervals, in traversal order — which is enough: a wrong prune decision or a wrong
insertion-order tie-break shows up the moment one query's result differs.

---

## What upstream tests

Four `it` blocks, and every one of them is doing real work despite the small count:

* **Construction from two different iterable shapes.** `StaticIntervalTree.from(array)` on five
  `[start, end]` pairs, and `StaticIntervalTree.from(map)` on a two-entry `Map` — checking `size`
  and `height` after each. The `Map` case is the only place upstream's own suite exercises
  `.from`'s iterable resolution on anything but a plain array.
* **Point queries, five of them** against the same five-interval tree: a point that hits nothing,
  a point inside two overlapping intervals, a point exactly on one interval's own boundary
  (`intervalsContainingPoint(0)` against `[0, 1]`), and two more ordinary interior hits.
* **Interval-overlap queries, twice**: a query that partially overlaps two intervals, and one wide
  enough to overlap every interval in the tree.
* **Custom getters**, resolving `start`/`end` from `{start, end}` objects instead of `[start, end]`
  pairs, re-run through one point query and one interval query.

## What upstream does NOT test

**The empty case — the whole regime**

1. **Zero intervals is never constructed.** Every tree in the file has five real intervals (or
   two, for the `Map` case). This is the precondition for B-100: `buildBST` is called
   unconditionally from the constructor, and nothing upstream ever supplies it a length of zero.

**Tree shape, beyond five intervals**

2. **Ties in the sort key (equal `start` values) never occur.** All five `BASIC_INTERVALS` have
   distinct starts. Upstream's own construction comment states the sort is meant to be stable
   (matching `%TypedArray%.prototype.sort`'s ES2019+ stability guarantee), but nothing in the
   suite has two intervals sharing a `start` to prove it.
3. **A tree past five intervals is never built**, so a height greater than 3 — and the deeper
   recursion, more subtree pruning, and larger augmentation array that come with it — is
   unexercised.
4. **A closed-interval boundary is tested on one side but not exhaustively.** `[0, 1]` is queried
   at its own start (`intervalsContainingPoint(0)`); no test queries an interval's own **end**
   value, or a point one unit outside either boundary, to confirm the closed-both-ends semantics
   the upstream file's own header comment states.
5. **A non-overlapping interval query — one that matches nothing at all — is never tried.** Both
   `intervalsOverlappingInterval` calls in the file hit at least one interval.

**Never called at all**

6. **A tree large enough to overflow the traversal's scratch stack** is not reachable through five
   or even fifty intervals at any realistic depth — `StackOverflow` has no upstream-reachable
   input at all, native or fuzzed, and is kept as a proper `Err` only because upstream's own
   `FixedStack.push`-equivalent would throw for the same reason if it were ever reached.
7. **A pointer array too large to index** (`length + 1` past what a 32-bit-indexable typed array
   can address) is never constructed — the same class of guard `vector`'s
   `a_pointer_vector_too_large_to_index_is_refused` pins for a different module.

## What we test in addition

`crates/mnemonist-core/src/structures/static_interval_tree.rs` — 9 unit tests beyond
`reproduces_the_upstream_suite` (the 1:1 port of all four upstream blocks), closing every gap
above: zero intervals refused rather than silently accepted (B-100), a second, isolated height
computation, that a resolved `(start, end)` pair rather than the getter itself is what the core
crate carries forward, a single-interval tree verified node-for-node against Node, ties in `start`
broken by original insertion order, both interval boundaries closed (plus a point just outside
each), a non-overlapping query finding nothing, fifty intervals with every point checked, and a
length too large to index refused. Full test-to-gap mapping: evidence file.

**Differential fuzzing** covers gaps 2 and 3 far more thoroughly than any hand-written test could:
every generated tree has up to 40 intervals with repeated starts, so the stable tie-break is
exercised on essentially every case rather than the one hand-picked example above.

**Still untested, stated rather than glossed:** gap 6 (`StackOverflow`) has no test anywhere in
this port either, native or fuzzed — it is not reachable by construction from any input this
module's own guards allow through, so there is nothing dishonest about its absence, only a gap
worth naming so it is not mistaken for an oversight.

## Bugs this found

**B-100 — constructing from zero intervals crashes with an unrelated `TypeError`.**
Verified against Node 24.18.1. `buildBST` runs unconditionally, even for
`length === 0`:

```js
buildBST(intervals, endGetter, indices, tree, augmentations, 0, 0, length - 1);
//                                                              i  low  high
```

With `length === 0`, `high` is `-1`. Inside, `mid = (0 + (-1 - 0) / 2) | 0` truncates to `0`
(`-0.5 | 0` is `0`), so `current = sortedIndices[0]` reads one past the end of a **zero-length**
typed array — `undefined`. `tree[i] = current + 1` becomes a harmless dropped `NaN` store (`tree`
is zero-length here too), but the very next line, `intervals[current][1]`, indexes `intervals`
with the property name `"undefined"` (absent on a plain array), giving `undefined[1]` — and
indexing `undefined` throws:

```js
> new StaticIntervalTree([])
TypeError: Cannot read properties of undefined (reading '1')
```

There is no guard anywhere upstream that catches empty input before this point — the message says
nothing about the input being empty, and the stack trace points three frames down into a helper
the caller never sees. This port reproduces the *outcome* (construction fails) as
`Err(Error::EmptyIntervals)` rather than the *mechanism*: a Rust panic unwinding across the napi
FFI boundary would abort the whole Node process (napi 3.12 does not `catch_unwind` a synchronous
call), which is a worse failure than the catchable `TypeError` it would stand in for, and
`#![forbid(unsafe_code)]` plus Rust's own bounds checking make an honest reproduction of
"index one past a zero-length array" impossible without one.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **Zero intervals is a clean `Err`, not a reproduced panic.** | See B-100 above — the mechanism has no honest Rust expression; the outcome (construction fails, informatively) is the faithful port. |
| — | **`startGetter`/`endGetter` are resolved once, at construction, not re-invoked on every visited node.** | Upstream calls them afresh on every node the query touches, in both query methods. Both getters are pure functions of an immutable stored interval in every case this port models — the original suite's own default and its one custom-getter test both fit that description — so re-invoking at query time can only reproduce the same `(start, end)` pair construction already computed. `mnemonist-core` takes the resolved bounds once ([`StaticIntervalTree::new`]'s `bounds` parameter); the getters themselves are a JS-value concern the core crate never sees. The bridge (`crates/mnemonist-napi/src/static_interval_tree.rs`) is where a getter actually runs: once per stored interval at construction, and once more per call for `intervalsOverlappingInterval`'s own query argument, which is not one of the stored intervals and so was never resolved in advance. |
| — | **`tree` and `augmentations` are not exposed to JS.** | They are public typed arrays upstream; napi can only hand out a copy, which would silently break the write-through a real caller could otherwise rely on — the same call `sparse-set`'s and `sparse-map`'s bridges make for `dense`/`sparse`/`vals`. Both are `pub` on the core type, and the differential fuzzer compares both slot for slot after construction, so the representation is verified even though no JS caller can reach it directly. |
| — | **`StaticIntervalTree.from`'s iterable resolution goes through upstream's real `Array.from`, not `obliterator/foreach`.** | Most of this port's other `.from()` statics route through `obliterator/foreach`'s five-branch dispatch, and this one deliberately does not, because the two are **not interchangeable** for this module's one `Map` test case: a `Map` owns a `.forEach` method, which `obliterator/foreach` prefers over `Symbol.iterator`, while `Array.from` always prefers `Symbol.iterator` when one exists. A `Map`'s default iterator yields `[key, value]` pairs — exactly the `[start, end]` shape this module wants — while its own `.forEach` invokes a callback with `(value, key, map)`. Routing `StaticIntervalTree.from(map)` through `obliterator/foreach` here would silently swap `start` and `end`, and it is exactly upstream's own `Map` test (gate 4) that would have caught it wrong. |

## Fuzz + bench

### Fuzz

Two campaigns, two seeds, **1,454,763 operations, zero divergences**:

```
module=static-interval-tree seed=42       cases=7143 ops=715440 wall=60.0s divergences=0
module=static-interval-tree seed=20260801 cases=7377 ops=739323 wall=60.0s divergences=0
```

Reproduce with `target/release/difffuzz --module static-interval-tree --seed 42 --cases 7143`.

Constructors draw 1..40 intervals, each `[start, start + delta]` with `start, delta` in
`0..150`/`0..50` — always well-formed and closed, and with starts repeating often enough that the
stable-sort tie-break is exercised on essentially every generated tree, not just the one
hand-picked case in the native suite. The op alphabet covers `intervalsContainingPoint`/
`intervalsOverlappingInterval` (query bounds enforced `a <= b` by construction). Query ranges are
drawn from `0..150`, wider than any constructed interval's own range, so "contains nothing" is a
routine outcome rather than an edge case. Observable state, compared after construction (this
module's queries never mutate it): `size`, `height`, and both augmented arrays, **`tree`** and
**`augmentations`**, encoded exactly as the oracle encodes each JS typed array. Every query's own
**result** — the matched interval list, in traversal order — is compared after every op, which is
what carries the whole signal past construction on a read-only module. Full grammar: evidence file.

A harness bug shared with `vector` (`serde_json`'s float parser not always correctly rounded for
JSON responses carrying `f64`-encoded query bounds) was found and fixed during this campaign's
first run; full account: log.

**Falsification of the port (gate 6):** the sabotage, `intervals_containing_point`'s match
condition tightened from `point >= start && point <= end` to `point > start && point <= end`
(excluding a point exactly on an interval's `start`, a plausible "shouldn't `start` be exclusive?"
cleanup), is confirmed red on the campaign's very first case — a divergence on
`intervalsContainingPoint(28)`, port returning `[]` against upstream's `[[28,41]]`. Reverted;
confirmed green again. Full record: evidence file.

### Bench

`bench/results.json` → `modules["static-interval-tree"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e5`** — 1e6 mixed `intervalsContainingPoint`/`intervalsOverlappingInterval` (50/50) over
100,000 overlapping intervals, each `(start, start + LENGTH)` with `LENGTH` **0.1%** of the
domain — `intervalsContainingPoint` averages ~101 matches per call and
`intervalsOverlappingInterval` averages ~202, both a real, meaningful fraction of the
100,000-interval tree pruned around, not 0 and not "the whole set" (see the log for why 0.1% was
chosen over an initial 10% attempt). No `add`: the tree is built once (untimed), same shape as
`vp-tree`/`kd-tree`; `new` itself sorts by start with a proper comparison sort, not the fixed-pivot
`inplace_quick_sort_indices` those two modules have to guard against, so no input-order trap
applies here. Position-weighted checksum, since neither query method sorts its own output.
xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **1603.03** | 3477.27 | 2.2× faster |
| p99 ns/op | **2361.25** | 5558.10 | 2.4× faster |
| RSS delta MB | **11.1** | 121.0 | |
| structure-only RSS delta MB | **0.2** | 6.2 | |
| startup ms | **0.6** | 15.4 | 26× (reported separately; not throughput) |

**No regressions.** Checksum `639629382466648`, identical on both sides — both trees visited and
pruned the same subtrees for the same queries.

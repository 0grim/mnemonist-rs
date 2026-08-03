# set

Upstream: `set.js` (356 LOC, **14 exported functions**) · `test/set.js` — **194 lines,
15 `describe` blocks, 16 `it` blocks, 25 assertion statements**.

Port: `crates/mnemonist-core/src/structures/set.rs` (`OrderedSet<T>` + the fourteen functions),
built on `crates/mnemonist-core/src/map/mod.rs`.
Bridge: `crates/mnemonist-napi/src/set.rs`, `crates/mnemonist-napi/src/js_key.rs`.
Shim: `tests/bridge/set.js`.

Two things about this unit are worth stating before anything else, because both were checked rather
than assumed and both would have led the port somewhere different if taken on faith.

**It is not a `Map`-backed module.** The filename suggests it belongs to that family, and the audit in
`crates/mnemonist-core/src/map/mod.rs` covers every module that is one. `set.js` has **zero**
`new Map(` and **six** `new Set(`. It holds no state, exports no constructor, and needs no storage
from the port. The capability it does need is native JS `Set` **at the boundary** — read sets in,
hand a set back, mutate the caller's own set in place — which is coercion work in `mnemonist-napi`,
and the reason core takes and returns an ordinary insertion-ordered set that has never heard of
JavaScript.

**Iteration order is observable, and it is most of the contract.** Eight of the fifteen `describe`
blocks assert `Array.from(result)` against an *ordered* array. That is not incidental: it means
`intersection`'s choice of which argument to iterate, `disjunct`'s phase order and `Set.add`'s
no-move-on-reinsert are all pinned, and a port that treated a set as unordered would fail the
original suite outright rather than subtly.

---

## What upstream tests

Sixteen `it` blocks, one per exported function plus a `variadic` case for each of the two variadic
ones, over sets of two to six small integers:

```js
var A = new Set([1, 2, 3]), B = new Set([2, 3, 4]);

assert.deepStrictEqual(Array.from(functions.intersection(A, B)), [2, 3]);
assert.deepStrictEqual(Array.from(functions.union(A, B, C, D)), [1, 2, 3, 4, 5, 6]);
functions.disjunct(A, new Set([2, 3]));  assert.deepStrictEqual(Array.from(A), [1, 3]);
assert.strictEqual(functions.jaccard(new Set('contact'), new Set('context')), 4 / 7);
```

Characterising the shape of that coverage:

* **Every one of the fourteen exports is called at least once.** Unusually complete for this
  library — most upstream test files leave methods untouched — and it is why the original suite is
  a genuinely useful gate here.
* **Almost every set is three or four small integers.** The two exceptions are `new Set('contact')`
  and `new Set('context')` in the `jaccard` and `overlap` blocks, which are the only string members
  and the only pair of sets with *different* sizes.
* **One empty set, used in three blocks** (`intersectionSize`, `unionSize`, `jaccard`, `overlap` —
  always as the second argument).
* **Assertions are `deepStrictEqual` on `Array.from`, or `strictEqual` on a number or boolean.**
  Never on the object's identity, and never on anything about the returned set beyond its contents
  in order.

## What upstream does NOT test

This is the section that carries the weight. Everything below is reachable through the public API
and never exercised by the original suite.

**Order, where the obvious order and the real one differ**

1. **`intersection` iterates its *smallest* argument**, so the result's order follows whichever
   argument that was. Untested: the two-set block uses two size-3 sets, where the smallest is the
   first and the distinction vanishes; the variadic block intersects down to a single member, where
   order is not visible at all. Confirmed against Node 24.18.1:
   `intersection(new Set([3,2,1]), new Set([1,2]))` is `[1, 2]` — B's order, not A's.
2. **A size tie goes to the first argument**, because the comparison is strict `<`. Never reached.
3. **`symmetricDifference(B, A)`** is never called; only one argument order is checked, so the
   "A's half then B's half" rule is half-tested.
4. **Re-adding a present member must not move it**, and delete-then-add must. Neither is checked,
   and they are the two halves of the same rule.

**Identity and freshness**

5. **No returned set is ever checked for being a `Set`.** `Array.from` accepts any iterable, so a
   port returning an array-like would pass all sixteen blocks.
6. **`difference(A, ∅)` returns `new Set(A)` — a copy.** Nothing asserts it is not `A` itself, so a
   port returning the argument would pass while aliasing two variables the caller believes are
   independent.
7. **The four mutating functions are never checked for mutating the *caller's object*** rather than
   producing an equivalent one, nor for *how* they mutate it. See "Bugs / near-misses" below: the
   difference is invisible to the original suite and visible to any iterator already open.

**Regimes never entered**

8. **Neither variadic function is ever called with one argument**, so neither
   `needs at least two arguments` throw is reached.
9. **Neither is ever called with more than four.**
10. **No function is ever applied to its own argument** — `subtract(A, A)`, `disjunct(A, A)`,
    `intersect(A, A)`, `add(A, A)`. All four are defined, two of them iterate a collection they are
    deleting from, and none is checked.
11. **`NaN` and `-0` are never members.** They are SameValueZero's only two special cases, which is
    the exact rule a `Set` keys on.
12. **A number and its string are never both members.** `new Set([1, '1'])` has two members; a port
    that keyed on a stringified form would conflate them and still pass.
13. **`isSubset` is never called with an empty set**, in either position.
14. **`jaccard(∅, ∅)` and `overlap(∅, ∅)`** are never called. Both answer `0` rather than dividing
    by zero — a convention, not a bug, and one a "cleaner" port would turn into `NaN`.
15. **`intersectionSize` is only ever called with the larger set first**, so the internal swap that
    makes it walk the smaller one is only exercised in one direction.

## What we test in addition

Every gap above is closed by a mix of Rust unit tests in `crates/mnemonist-core/src/structures/set.rs`
and `tests/boundary/set.js` (differential against vendored upstream): iteration order following the
smallest argument and its tie-break, both argument orders of `symmetricDifference`, both halves of
the re-add/delete-then-add rule, identity checks (`instanceof Set`, `notStrictEqual` on `difference`'s
copy), the mutating four applied to their own argument, arity throws at one and at zero arguments,
eight-way intersection/union, `NaN`/`-0` as members, `1` vs `'1'` distinguished, empty-set subset/
superset/ratio cases, and both `intersectionSize` argument orders. Full test-to-gap mapping:
evidence file.

Plus the differential fuzzer: 37,853 programs and 1,531,479 operations across two 60-second
campaigns, zero divergences.

## Bugs this found

**None in upstream.** `set.js` is 356 lines of straightforward code with no shared mutable state, no
typed arrays, no index arithmetic and no re-entrancy, and reading it statement by statement turned
up nothing to file. That is worth recording as an outcome rather than omitting: two of the three
units read closely this way produced verified upstream bugs (BUG-SORT-1 and BUG-SORT-2 in `sort`), and this
one genuinely does not.

Three things that *look* like bugs and are not, each checked against Node 24.18.1:

* **`jaccard(∅, ∅)` is `0`, not `NaN` or `1`.** The `if (I === 0) return 0` guard fires before the
  division. Same for `overlap`. A convention, and reproduced.
* **`intersection`'s result order depends on which argument was smallest.** Surprising, documented
  above, and correct — it falls out of the optimisation of iterating the smallest set.
* **`difference(A, B)` with an empty `B` returns a copy of `A` rather than `A`.** Deliberate: the
  function's contract is to return a new set.

### One near-miss in the port, caught by the boundary spec

The first bridge sketch for the four mutating functions was: read `A`, compute the answer,
`A.clear()`, re-add every member. It passes all sixteen upstream blocks. It is observably wrong:

```js
var A = new Set([1, 2]);
var it = A.values();
it.next();                              // consumes 1
functions.add(A, new Set([2, 3]));
Array.from(it);
// upstream           [2, 3]
// clear-and-rebuild  [1, 2, 3]   ← the re-inserted 1 is visited again
```

A JS `Set` iterator is live: entries appended after it was created are visited, deleted ones are
skipped. `clear()` empties the entry list without detaching the iterator, so every re-inserted
member is seen a second time. Measured, and now the subject of
`tests/boundary/set.js` "should reach a live iterator through add/delete, not clear-and-rebuild".

## Deliberate divergences

### DIV-SET-1 — the four mutating functions replay `add`/`delete` rather than rebuilding

Core returns the `SetOp` trace it applied, in upstream's own call order, and the bridge makes
exactly those calls on the caller's object. The alternative — computing the final member list and
rebuilding — is simpler, passes the original suite, and is wrong for the reason measured just above.

The one place the port still differs: the `add` and `delete` **handles are fetched once**, before
the first call, so a member's own side effects cannot divert the rest of the trace. Upstream
resolves `A.add` on every call and so could be diverted. Nothing in the original suite goes either
way, and one lookup is the honest reading of `A.add(x)` repeated in a loop.

### DIV-SET-2 — object members are refused

`JsKey`'s existing limit, unchanged: `Set` compares objects by identity and no identity hash for a
JS object is reachable from Rust. The two implementable designs — a hidden `Symbol` tag, or an
association list probed with `napi_strict_equals` — each cost something real, and are argued in
`crates/mnemonist-napi/src/js_key.rs`. **`test/set.js` uses numbers and single characters only.**
The refusal names the limitation rather than silently conflating two distinct objects, which is what
a port keyed on anything weaker would do.

### DIV-SET-3 — the variadic pair goes through an array, and the arity check stays in core

napi has no variadic parameter. `intersection` and `union` take a `Vec` and `tests/bridge/set.js`
spreads into it — arity glue in the shim, and the same role
`crate::statics` plays for `X.of`. The `needs at least two arguments` check is in
`mnemonist-core`, so upstream's threshold and its exact message live in one place and the shim
forwards whatever it was handed, including nothing.

### DIV-SET-4 — upstream's three `===` shortcuts are implemented, and unreachable from JavaScript

`intersection` skips `set.has(item)` when `set === smallestSet`; `isSubset` returns `true` when
`A === B`; `intersectionSize` returns `A.size` on the same test. Core implements all three with
`std::ptr::eq`, so a Rust caller passing one reference twice takes upstream's path. The **bridge
cannot**: two arguments that are the same JS `Set` become two separate `OrderedSet`s when read.

Unobservable, and worth showing rather than asserting. When the identity holds, the check each
shortcut skips is `smallest.has(member)` for a member drawn from `smallest` — `true` by
construction — or a count of `A`'s members that are in `A`, which is `A.size`. Every path produces
the same answer. `tests/boundary/set.js` passes one object twice to all six affected functions and
compares against upstream.

### `disjunct`'s write order

`disjunct` evaluates `!A.has(member)` against the original `A` **before** any deletion, then emits
its trace add-then-delete, in upstream's own call order — faithful reproduction of the sequence
upstream makes, even though nothing in either test file distinguishes it from a delete-then-add
trace producing the identical result and order (see the log for how that was established). What
*is* load-bearing is that the membership test runs before any deletion: deleting first would make
every member of `A ∩ B` pass the `!A.has` test, get re-added, and turn the answer into `A ∪ B`. That
is pinned by `set.rs::disjunct_decides_what_to_add_before_it_deletes_anything` and by
`tests/boundary/set.js`, and it is the one part of `disjunct`'s ordering this port's own gate-4
falsification confirms is load-bearing (see "Fuzz + bench").

## Fuzz + bench

### Fuzz

Two campaigns, both clean, **1,531,479 operations, zero divergences**:

| seed | cases | ops | wall | divergences |
|---|---|---|---|---|
| 42 | 20,024 | 812,141 | 60.0s | 0 |
| 20260801 | 17,829 | 719,338 | 60.0s | 0 |

All fourteen exports are in the alphabet — `set.js` has no function the grammar omits. Sets hold
0–8 members drawn from a pool of ten (six small integers, the string `"1"`, `"a"`, `"b"` and `""`),
with sizes varying freely so that `intersection`'s smallest-argument rule and its first-wins
tie-break are reached constantly. The variadic pair generates **one** set as well as two to five, so
both arity throws are compared as results rather than excluded. Full grammar and exclusions in
`fuzz/log.txt`.

This is the second free-function module, so the campaign compares no observable state — there is
none. Here that matters more than it did for `sort`: `add`, `subtract`, `intersect` and `disjunct`
all return `undefined` and do their whole job to their first argument, so **without the argument
echo, four of the fourteen functions would be compared against nothing at all**.

**Falsification (gate 6).** Two sabotages, both chosen so `test/set.js` stays green under them —
they measure what the campaign adds over the original suite rather than what it duplicates —
confirmed 16 passing under each, and green again after revert: `intersection` iterating the first
set instead of the smallest (order-only, invisible to any comparison that treats a set as unordered,
found in 109 cases/0.1s, shrunk to one op) and `add` inserting `B`'s members in reverse order (also
order-only, found in 82 cases/0.1s). Both seeds are committed in
`crates/difffuzz/proptest-regressions/set.txt` with a provenance block saying they came from
sabotages and not from real port defects. Full record: evidence file.

**Falsification of the port (gate 6, on the original suite), separate, is what proves the original
test file exercises Rust rather than a JS fallback:** making `disjunct` delete before deciding what
to add breaks `test/set.js`'s "#.disjunct → should properly disjunct the second set to the first."
(16 passing → 15 passing, 1 failing, that block and only that block). Reverted, back to 16 passing.
The other fifteen blocks staying green is the useful part — the sabotage is local to one function
and the suite localised it. Full record: evidence file.

### Bench

`bench/results.json` → `modules["set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 500 samples/side.

`set.js` has no instance and no per-element op stream, so this reuses `sort`/`suffix-array`'s
`drain` shape: one measured sample per **`union`** call — the representative choice out of the
fourteen free functions this module exports; see `bench/runner/src/set_ops.rs`'s own module docs
for why `union` rather than `disjunct` (this module's most intricate function, already covered
above by its own falsification) or `intersection` (which only ever walks its smallest argument, so
its cost does not scale the same way with both inputs).

**`union-2e4x50`** — `union(A, B)` of two 20,000-element sets drawn from the SAME `0..20,000`
domain (guaranteeing real overlap and internal duplicates by the birthday bound, so the dedup path
`OrderedSet::add` takes is genuinely exercised rather than benchmarking two sets that never
overlap): the port is 1.9× faster at p50 (13.2 vs 25.0 ns/op) and 5.1× faster at p99 (17.6 vs 89.2).
No regressions, and the widest p99 margin in this group. Full table: evidence file.

A fresh native JS `Set` per pass (`new Set()` called eagerly for both `A` and `B`, then again inside
`union` itself for the result) is a plausible, but unconfirmed, source of upstream's heavier tail:
each `union` call upstream makes constructs one `Set` and does one `.add` per member visited, all
through general-purpose `Set` machinery, while the port's `OrderedSet` is backed by the same
`OrderedMap` every `Map`-backed module in this project already shares.

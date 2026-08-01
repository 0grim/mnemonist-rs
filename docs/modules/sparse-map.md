# sparse-map

Upstream: `sparse-map.js` (243 LOC) + `utils/typed-arrays.js` (187 LOC, only `getPointerArray`
reachable) · `test/sparse-map.js` — **139 lines, 9 `it` blocks, 32 assertion statements**.

Port: `crates/mnemonist-core/src/structures/sparse_map.rs`,
`crates/mnemonist-core/src/cursor/mod.rs`, `crates/mnemonist-core/src/utils/typed_arrays.rs`.
Bridge: `crates/mnemonist-napi/src/sparse_map.rs`, `crates/mnemonist-napi/src/cursor.rs`.

`SparseSet` with a payload, and the payload is where everything interesting is. The module
inherits the dense/sparse pair, the O(1) `clear`, and all three of `sparse-set`'s out-of-range
defects (B-8, B-9, B-10) unchanged. It adds one of its own that needs **no** out-of-range input at
all — `delete` moves the key and leaves the value behind — and the upstream suite cannot see it
because of a single structural property of that suite: it only ever deletes from a one-element map.

---

## What upstream tests

Nine `it` blocks, all on a map of length 10:

```js
var map = new SparseMap(10);
map.set(3, 14); map.set(4, 22); map.set(3, 35);
assert.strictEqual(map.size, 2);
assert.strictEqual(map.get(3), 35);
assert.strictEqual(map.get(12), undefined);
// …the same three assertions again against `new SparseMap(Uint8Array, 10)`…
map.set(3, 14); map.delete(3); map.delete(4);        // the whole of deletion coverage
map.forEach(function (value, key) { … });
assert.deepStrictEqual(obliterator.take(map.keys()),    [3, 6, 9]);
assert.deepStrictEqual(obliterator.take(map.values()),  [13, 22, 8]);
assert.deepStrictEqual(obliterator.take(map.entries()), [[3, 13], [6, 22], [9, 8]]);
```

Characterising the shape of that coverage — it is genuinely better than `sparse-set`'s, and it
still misses the module's worst bug:

* **Both constructor signatures are used**, which `sparse-set` had no equivalent of. `new
  SparseMap(10)` and `new SparseMap(Uint8Array, 10)` each get a block. This is real coverage of the
  overload.
* **All three iterators are exercised**, one block each, and `forEach`'s two-argument callback is
  destructured — so this suite *does* inspect the second callback argument, which `sparse-set`'s
  does not.
* **One length, 10, in all nine blocks.** One pointer width, one path through `getPointerArray`.
* **The largest map holds six entries; the largest member used is 12**, and that one only to check
  that an out-of-range *read* is absent.
* **Deletion is only ever from a one-element map.** `map.set(3, 14); map.delete(3);
  map.delete(4);` is the entire deletion coverage, and the swap in `delete` is therefore always a
  self-assignment.
* **Every value is a small non-negative integer**, so the `Uint8Array` block never truncates:
  its largest value is 35.
* **Iteration is drained immediately.** `obliterator.take(map.<iter>())` creates a cursor and
  exhausts it in one expression, so nothing about a cursor's *state* is observable.

## What upstream does NOT test

Everything below is reachable through the public API and never exercised by the original suite.

**The one that matters**

1. **`delete` on a map with more than one entry is never performed.** This single omission hides
   B-11 completely: the swap moves the last *member* into the hole and leaves the last *value*
   where it was, so the moved member inherits the deleted member's value. On a one-element map the
   swap is `dense[0] = dense[0]` and the missing value move is invisible. **Measured: "fixing" the
   bug leaves this suite at 9 passing, 0 failing.**
2. **Iteration order after a delete is never checked**, which is the only other externally visible
   consequence of the swap.
3. **A value is never read back after any delete.** `map.get` is called after `delete` exactly
   once, on a member that was *not* in the map.

**Return values and chaining**

4. **`delete`'s return value is never used.** It is the module's only method with a meaningful
   boolean result; the block that calls it twice asserts only `size`.
5. **`set`'s return value is never used.** Upstream returns `this` for chaining.

**The value store, beyond the two shapes it is constructed with**

6. **The `Uint8Array` block never stores a value ≥ 256**, so the truncating value store — the
   entire reason to pass a value constructor — is never triggered. Its largest value is 35.
7. **The 16- and 32-bit value stores are never constructed.**
8. **The `Array` store's growth past the map's length is never reached.** `new Array(length)` is
   not fixed-length, and `vals[size] = value` with `size >= length` *extends* it. That is a
   different behaviour from the typed store's dropped write, and neither is tested.
9. **`vals` is never read**, so nothing pins what the store actually contains after any operation.

**The structure's algorithm**

10. **The swap-with-last is never observed** (see 1–3).
11. **Deleting the last entry, then a member that was moved, is never done.**
12. **Re-setting after `clear` is never done.** `clear` is O(1) and leaves live-looking debris in
    all three arrays; that the debris stays unreachable is asserted once and never re-tested after
    the map is used again.
13. **The map is never filled to capacity.**

**The typed-array width machinery**

14. **Only one length is ever constructed: 10.** `getPointerArray(10)` returns `Uint8Array`, so the
    16-bit and 32-bit index branches are never reached and the `length > 2³²` throw is never
    reached.
15. **The truncating `dense` store is never triggered**; that needs a member ≥ 256.

**Out-of-range members — the whole regime**

16. **`has(12)` and `get(12)` are the only out-of-range calls in the file, and both are reads.** No
    out-of-range `set` is ever performed, which hides all of:
    * `set(m)` past the end **corrupts the map**, exactly as `SparseSet.add` does (B-8) — plus a
      wrinkle of its own, since the *value* still lands even when the key does not;
    * therefore **`size` can exceed `length`**;
    * `delete` past capacity writes `dense` but not `sparse` (B-10);
    * and `new SparseMap(0)` accepts entries it can never find while its `Array` store grows one
      slot per `set`.

**Iteration — everything except three immediate drains**

17. **Mutation during iteration is never performed.** The hybrid capture (DESIGN.md §3.4) means an
    element write mid-walk *is* visible and a length change is *not*; neither half is tested. On
    this module the visible result of a mid-walk `delete` is a **mismatched pair**, which is B-11
    at its sharpest.
18. **A cursor is never re-drained**, so D-06 non-restartability is unobserved.
19. **`[...map]` is never used.** The suite reaches the cursors only through the three named
    methods, so the collection-level `Symbol.iterator` — the *factory* half of D-07, and the half
    napi does not provide — has **zero** coverage, despite being the last line of the module. It is
    also aliased to `entries`, not `values`, and nothing checks that.
20. **The three projections are never compared against each other on the same map**, so the states
    where they disagree — `size > length` — are entirely unreached.
21. **Any iterator on an empty map, or after `clear`, is never called.**
22. **The `undefined` window is never reached**, which follows from 16 and 17 together.

**`forEach`**

23. **`scope` is never passed**, so the `arguments.length > 1 ? scope : this` branch is untested.
24. **`forEach` on an empty map is never called**, and neither is a `forEach` whose callback
    mutates — which matters here, because this `forEach` re-reads `this.size` every iteration while
    the cursors freeze it.

**Never called at all**

25. `inspect()` and the `nodejs.util.inspect.custom` symbol. ~20 LOC of the module.

## What we test in addition

Rust native tests, mapped to the gaps above.

`crates/mnemonist-core/src/structures/sparse_map.rs` — 20 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all nine upstream blocks, both constructor signatures, as a baseline |
| `delete_moves_the_key_but_not_the_value` | **1, 2, 3, 9, 10** — the headline defect, pinned on `get`, on iteration order *and* on `vals` slot by slot |
| `the_stale_value_is_not_an_artefact_of_the_array_store` | 1 — the same defect through a typed store, so it is attributed to the swap and not to the store |
| `deleting_the_last_entry_hides_the_defect` | 11 — and it is named for what it demonstrates: the branch upstream tests is the branch where the bug is invisible |
| `setting_a_present_member_overwrites_in_place` | 5 — asserts both index arrays are unchanged, not just `size` |
| `clear_leaves_stale_entries_that_stay_unreachable` | 12 — asserts the debris is *there*, in `dense` and in `vals`, and unreachable |
| `reads_out_of_range_report_absence` | 16 (the `has`/`get`/`delete` half) |
| `an_out_of_range_set_corrupts_the_map_exactly_as_upstream_does` | 15, 16 — the compound defect, pinned value by value, including the value landing where the key did not |
| `an_array_value_store_outgrows_the_map_it_belongs_to` | **8, 20, 22** — `keys()` gapping while `values()` still yields real data, on the same map, at the same ordinals |
| `a_typed_value_store_drops_the_write_and_gaps_with_the_keys` | 8, 20 — the contrasting store, where both sides gap together |
| `typed_values_truncate_at_their_own_width` | 6, 7 — all three widths, and the independence of the value width from the index width |
| `a_delete_past_capacity_writes_dense_but_not_sparse` | 16 — B-10 on this module's arrays |
| `cursors_do_not_restart_but_the_map_can_be_walked_again` | 18, 19 — both levels of D-07, and all three projections |
| `a_delete_during_iteration_is_visible_and_desynchronises_the_pair` | **17** — B-11 and D-08 in one assertion: the walk yields `(3, 20)`, a key and a value that were never set together |
| `a_set_during_iteration_is_not_visible_to_the_cursor` | 17 — the frozen-length half |
| `picks_one_pointer_width_for_both_index_arrays` | 14 — five lengths across both width boundaries |
| `rejects_a_length_no_pointer_array_can_index` | 14 (the throw), for both constructors |
| `a_zero_length_map_finds_nothing_but_still_accumulates_values` | 8, 16, 21, 22 — the degenerate end, where the two stores diverge most |
| `fills_to_capacity_without_running_off_the_end` | 13 |
| `projections_do_not_answer_for_each_other` | the projection accessors, so a mis-projected step is a `None` rather than a plausible-looking value |

`crates/mnemonist-core/src/utils/typed_arrays.rs` — two new tests for this module:
`to_uint32_wraps_where_an_as_cast_would_saturate` (17 inputs, pinned against Node) and
`to_uint32_then_a_narrowing_store_is_the_js_element_store`. Both exist because Rust's `as` cast
**saturates** where JS's `ToUint32` **wraps**: `-1` must become `255` in a `Uint8Array`, and
`as u32 as u8` gives `0`.

`crates/mnemonist-core/src/cursor/mod.rs` — one new test,
`a_projection_selects_which_walk_without_a_second_impl`, plus `Step::map` coverage in
`step_projections`.

The **differential fuzzer** then covers gaps 1–22 continuously rather than at hand-picked points,
with `vals` in the observed state so the value store is compared slot for slot after every
operation of every generated program.

**Still untested, stated rather than glossed:** gap 25 (`inspect`, not bridged), gaps 23/24 in
their `arguments.length` form (see the divergence table), non-numeric values (the T3 boundary, see
the table), and the signed/floating value constructors, which the bridge refuses with an error
that says so.

## Bugs this found

**B-11 — `SparseMap.delete` moves the key and leaves the value behind.**
`status: verified against Node 24.18.1`. This is a plain correctness bug on entirely in-range
input, not an edge case. Upstream's `delete` is `SparseSet`'s swap-with-last, copied verbatim:

```js
index = this.dense[this.size - 1];
this.dense[this.sparse[member]] = index;   // the last MEMBER moves into the hole
this.sparse[index]              = this.sparse[member];
this.size--;                               // and `vals` is never touched
```

`SparseSet` has no values, so the swap is complete there. `SparseMap` has three parallel arrays and
moves one of them. Measured on real Node:

```js
var m = new SparseMap(10);
m.set(3, 'a'); m.set(4, 'b'); m.set(5, 'c');
m.delete(3);
m.get(5)        // 'a'   — should be 'c'
Array.from(m)   // [[5, 'a'], [4, 'b']]
```

Reproduced, not fixed. It holds for a typed value store too (`vals = [11, 22, 33]`,
`get(5) === 11`), so it is the swap and not the `Array`.

**Why it survived upstream, measured rather than asserted.** The test file deletes exactly twice,
both times from a map holding one entry, where `this.sparse[member]` is `0` and
`this.dense[this.size - 1]` is the same member — a self-assignment. Sabotaging the port to *fix*
B-11 leaves `tests/run.sh test/sparse-map.js` at **9 passing, 0 failing**, while turning **four**
of our native tests red and being caught by the differential fuzzer in 3.0 seconds. That
measurement is the clearest statement of the rigor gap this project has produced: the suite is not
weak in an obvious way — it covers both constructors and all three iterators — it just never builds
a map big enough for its own delete to do anything.

**B-8, B-9 and B-10 all apply unchanged**, since `has`, `set`'s insert path and `delete`'s swap are
`sparse-set`'s code. `status: verified against Node 24.18.1` for this module specifically:
`new SparseMap(10); set(300, 7)` gives `size === 1`, `dense === [44, 0, …]`, `sparse` untouched,
`vals === [7, <9 holes>]`, `has(300) === has(44) === false`; and `new SparseMap(3)`, set
`0/1/2/99`, `delete(1)` leaves `dense = [0, 0, 2]`, `sparse = [0, 1, 2]` and `sparse.undefined = 1`.

**A behaviour that is not a bug but is worth recording: the two value stores diverge past
`length`.** `new Array(length)` is not fixed-length, so `vals[size] = value` with `size >= length`
**extends** it, where the same store into a typed array is dropped. Measured:

```js
var a = new SparseMap(2);            var t = new SparseMap(Uint8Array, 2);
a.set(100,1); … a.set(103,4);        t.set(100,1); t.set(101,2); t.set(102,3);
a.vals    // [1, 2, 3, 4]            t.vals    // [1, 2]
[...a]    // [[100,1],[101,2],       [...t]    // [[100,1],[101,2],
          //  [undefined,3],                   //  [undefined,undefined]]
          //  [undefined,4]]
```

So on an `Array`-backed map, `keys()` yields `undefined` at ordinals where `values()` still yields
real data. Same map, same frozen length, two iterators disagreeing about where the data ends. Not
filed — it follows from JS array semantics rather than from a mistake — but it is the reason the
port models the value store as two shapes rather than one width, and the reason the fuzzer
generates all four constructors.

**What the fuzzer found: nothing new.** Two campaigns, 2.65 M operations, zero divergences —
the expected outcome for a faithful port (D-33), and the same result as the two previous modules.
B-11 was found by reading `delete` and asking what happened to the third array. What the fuzzer is
for is the other direction, and it was proven to work in that direction twice (see Fuzz, below) —
including on B-11 itself.


### B-31 — `&self` on a `Freeze` type was `noalias readonly` (fixed 2026-08-01)

This bridge held a bare core value, so `&self` compiled to a `noalias readonly` pointer and LLVM was
entitled to hoist reads across the JS callback — which it did. It now holds `RefCell<Core>`, which
is not `Freeze`, and every `&mut self` method became `&self` + `borrow_mut()`. The borrow is taken
per step and released before the callback runs, so a re-entrant callback never meets an outstanding
borrow. See `planning/NOTES.md` B-31 and `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **`delete` does not move the value.** | B-11, reproduced bug-for-bug. Fixing it would be a silent behavioural divergence on in-range input, and `get` after `delete` is observable. Pinned by four native tests and by a committed fuzz seed, so a future "cleanup" fails loudly. |
| — | **Values are JS numbers (`f64`), not arbitrary JS values.** | The core is generic over the value type; the bridge instantiates it at `f64`. Arbitrary values are DESIGN.md §3.3's T3 tier — a per-slot `Ref` and an `Env` to drop it — and this module does not reach for it. The upstream test file stores only numbers. `map.set(3, 'x')` throws here and works upstream. |
| — | **Only `Array`, `Uint8Array`, `Uint16Array` and `Uint32Array` are accepted as `Values`.** | `PointerVec` models the three unsigned widths. `Int8Array`, `Float64Array` and the rest are refused with an error naming the gap, rather than silently coerced into the nearest supported width — which would be a wrong answer dressed as a right one. |
| — | **`Values` is resolved by identity, not by name.** | `strict_equals` against the real `globalThis.Uint8Array`, because `{name: 'Uint8Array'}` is trivial to forge and reading `.name` would accept it. |
| — | **The constructor branches on "was a second argument passed", not on `arguments.length`.** | napi cannot see `arguments.length`. The two agree on every call except `new SparseMap(x, undefined)`, where upstream sees two arguments and this sees one. Same blind spot as `forEach`'s `scope`, below. The two shapes upstream *throws* on are reproduced: `new SparseMap(Ctor)` reaches `getPointerArray(NaN)` and so throws the pointer-array message verbatim, and `new SparseMap(10, 20)` reaches `new (10)(20)`. |
| D-09 | **The shrink window is reproduced (Option A), not collapsed.** | As `sparse-set`. Reachable here through `keys()` on any map whose `size` has run past `length`. |
| — | **`entries()` is the one cursor whose `Yield` is not an `Either`.** | Upstream builds `[dense[i], vals[i]]` and yields **the array**, so a missing half is `undefined` *inside* a yielded value and the step itself never gaps. `Option::None` therefore keeps its plain meaning of `{done: true}` on that cursor alone. |
| D-06 | **No collection implements `IntoIterator`.** | It would hand out a fresh iterator per `for` loop and silently restart. |
| D-07 | **`Symbol.iterator` is installed from Rust, and aliased to `entries`.** | Not to `values` — that is what upstream aliases, and getting it wrong would leave `[...map]` yielding bare numbers with no upstream test to catch it, since the suite never spreads a map. The table in `crates/mnemonist-napi/src/cursor.rs` carries the method name per class for exactly this reason. |
| — | **Three walks, one `Sequence` impl, projection carried in `Frozen`.** | A type may implement a trait once, and `keys`/`values`/`entries` are three copies of one closure over one frozen `size`. `CursorState::open_projected` replaces the frozen payload while still taking the length from `freeze()`, so the "no window between the two reads" guarantee is unchanged. |
| — | **The Rust-side `Iterator` impl skips gaps rather than stopping.** | As `sparse-set`. The faithful three-way primitive is `step()`. |
| — | **`set` returns `bool` in core; the bridge returns `this`.** | Core reports whether the member was newly inserted, which upstream exposes only through `size`. |
| — | **`dense`, `sparse` and `vals` are not exposed to JS.** | They are public upstream and a JS caller can write *through* them; napi can only hand out a copy, which would silently break write-through. All three are exposed in Rust and compared slot for slot by the fuzzer. |
| — | **`forEach(cb, undefined)` binds `this` to the map.** | Upstream keys off `arguments.length > 1`, which napi's typed signature cannot see. The omitted-argument case — the only one the original suite uses — is exact, and passing a real scope object is exact. |
| — | **`new()` returns `Result`, and validates before allocating.** | Upstream throws for `length > 2³²`, and reaches that throw inside `getPointerArray` *before* `new Values(length)` runs. Allocating first and validating after was the port's original shape; it turned an `Err` into a 34 GB allocation abort, which is how the ordering got noticed. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion. |

## Fuzz + bench

### Fuzz

```
module=sparse-map seed=42       cases=17268 ops=1789276 wall=120.0s divergences=0
module=sparse-map seed=20260801 cases=8330  ops=860398  wall=60.0s  divergences=0
```

Two campaigns, two seeds, **2.65 M operations, zero divergences**. Reproduce with
`target/release/difffuzz --module sparse-map --seed 42 --cases 17268`.

* **Op alphabet:** `set(m, v)` (weight 5) · `delete(m)` (2) · `has(m)` (2) · `get(m)` (2) ·
  `clear()` (1) · `$iter("keys")` / `$iter("values")` / `$iter("entries")` (1 each) · `$next()` (3)
  · `$spread()` (1).
* **Observable state, compared after every op:** `size`, `length`, `dense`, `sparse` **and
  `vals`**. `vals` is what makes B-11 checkable rather than inferable: a port that "tidied up" the
  missing value move would still agree on `size`, on both index arrays and on every `has`.
* **Constructors:** both upstream signatures, all four supported value stores. A JS constructor
  cannot travel over JSON, so it goes as `{"$global": "Uint8Array"}` and `fuzz/oracle.js` resolves
  it against the real global — deliberately for `init`'s constructor arguments only, never for an
  op's.
* **Lengths:** `0..=400`, as `sparse-set` and for the same two reasons.
* **Members:** `0..length + 64`, so roughly one in eight is out of range.
* **Values:** `0..=1000`, so an 8-bit value store truncates on a good fraction of its writes.
* **Program length:** 1..200 ops.
* **Deliberately narrowed: the values.** Integers, not every JS number. The bridge takes `f64` and
  this spec takes `u32`, because the oracle compares JSON and a float that renders `13.0` on one
  side and `13` on the other is a false divergence rather than a finding. Nothing in the op
  alphabet is excluded.

All three iterator factories are generated because the three projections **disagree** exactly where
a port goes wrong — once `size` has run past `length`, `keys()` gaps on `dense`, `values()` may
still have real data from a grown `Array` store, and `entries()` never gaps at all. Folding them
into one op would leave every such program still passing.

**The fuzzer was falsified twice, once per half of this module's new grammar.** Both sabotages were
reverted and both seeds are committed with provenance in
`crates/difffuzz/proptest-regressions/sparse-map.txt`, where proptest replays them before any novel
case on every subsequent run.

**A — the value half.** Sabotage: `delete` "fixed" to move the value along with the key. Not a
typo — the change a reader makes on purpose, having decided the omission was an oversight. Caught
in **1,474 cases (3.0 s)**, shrunk from 200 ops to three:

```js
var s = new SparseMap(5);
s.set(0, 0);
s.set(1, 1);
s.delete(0);   // port vals [1,1,…], upstream [0,1,…]
```

This is the most instructive seed in the repo, because **the same sabotage leaves the upstream
mocha suite at 9 passing, 0 failing.**

**B — the store half.** Sabotage: the `Array` value store made unable to grow, i.e. given the typed
store's dropped-write behaviour. That is the natural Rust assumption — a `Vec` allocated at
`length` has a length. Caught in **1,147 cases (4.6 s)**, shrunk to a **single** operation:

```js
var s = new SparseMap(0);
s.set(0, 0);   // port vals [], upstream [0]
```

One call past the constructor.

### Falsification of the port (gate 6)

Separate from the fuzzer falsifications: gate 6 asks that sabotaging the core turns the *original
mocha suite* red, proving it exercises Rust rather than a JS fallback.

**The assertion the sabotage had to break was named first:**
`should be possible to create an iterator over the map's values` —
`assert.deepStrictEqual(obliterator.take(map.values()), [13, 22, 8])`, at `test/sparse-map.js:127`.
Chosen because it reaches the projection machinery, which is the code this unit adds.

**The sabotage:** the `Values` projection reading `dense` instead of `vals` — the copy-paste error
that three near-identical upstream closures invite, and the one a port writes by adapting `keys()`
into `values()` and forgetting one identifier.

**Confirmed red**, and red in precisely the named place: `8 passing, 1 failing`, the failure being
that assertion, with `actual` `[3, 6, 9]` against `expected` `[13, 22, 8]`. Reverted; **confirmed
green again**: `9 passing`.

**And a second falsification that was expected to stay green, and did.** Gate 6's own lesson is
that a check which cannot fail is a second green light — so it is worth knowing *which* sabotages
this suite cannot catch, not only that some can. Fixing B-11 in the core leaves the suite at
**9 passing, 0 failing** while turning **four** native tests red
(`delete_moves_the_key_but_not_the_value`, `the_stale_value_is_not_an_artefact_of_the_array_store`,
`a_delete_past_capacity_writes_dense_but_not_sparse`,
`a_delete_during_iteration_is_visible_and_desynchronises_the_pair`) and being caught by the
differential fuzzer in 3.0 seconds. Both numbers were measured, not reasoned about.

### Bench

**Not yet run.** Gate 10 is deliberately batched into a separate quiet serial pass — a benchmark
taken under load is not a slow benchmark, it is a wrong one, and this host has already demonstrated
a contended run inflating both sides 2–3× (`planning/NOTES.md`, H+5). This module is therefore
**not** in `tests/scope.txt`: gates 1–9 are green and gate 10 is outstanding, which is exactly what
`tests/verify.sh` will report. See `planning/DESIGN.md` §7.3.

### `$forEach` — the op that was missing (added 2026-08-01, B-31)

`sparse-map`'s grammar had no `forEach` op at all. That omission is what let B-31 — a `forEach`
callback mutating the collection it is walking — through 2.65 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it.

`$forEach(method, rule, limit)` now walks the instance with a callback that calls back into it.
The compared result is the sequence of callback argument pairs, so the walk's **shape** is checked
and not only the state it leaves behind. This module's mutations:

* `delete(a1)`, `set(a1, a0)` and `clear()`, all uncapped.

`set` writes back the pair it was just handed, so it overwrites rather than grows and is safe
uncapped even though this module's bound — like `sparse-set`'s — is live. Note the callback
argument order: upstream passes the **value** first and the key second, which is why `set`'s rule
is `arg1,arg0`.

**What it does not reach, stated so the campaign is not over-read.** `difffuzz` compares
`mnemonist-core` against upstream JS; the napi bridge, where B-31's hoisted read actually lived, is
not in that loop. No op alphabet can catch that class of bug here. The specs that do are
`tests/boundary/reentrancy.js`, which drive the real addon with real JS callbacks — red on the
pre-fix bridges, green after.

One deliberate narrowing, mirrored on both sides: a selected callback argument that is `undefined`
skips the mutation. Feeding it back in reaches upstream's `NaN`-indexed swap, which `usize` cannot
express and the core does not model. Fully disclosed in `fuzz/log.txt`.

# sparse-set

Upstream: `sparse-set.js` (168 LOC) + `utils/typed-arrays.js` (187 LOC, only `getPointerArray`
reachable) · `test/sparse-set.js` — **76 lines, 6 `it` blocks, 9 assertion statements**.

Port: `crates/mnemonist-core/src/structures/sparse_set.rs`,
`crates/mnemonist-core/src/cursor/mod.rs`, `crates/mnemonist-core/src/utils/typed_arrays.rs`.
Bridge: `crates/mnemonist-napi/src/sparse_set.rs`, `crates/mnemonist-napi/src/cursor.rs`.

This is the first module in the port with a real iterator surface, which is why it was chosen to
land immediately after the cursor machinery (DESIGN.md §3.4/§3.6/§3.7). It is also, unexpectedly,
the module that makes the `undefined` shrink window of §3.7 reachable through the **public API** in
two calls — see below.

---

## What upstream tests

Six `it` blocks over a single set of length 10:

```js
var set = new SparseSet(10);
set.add(3); set.add(4); set.add(3);
assert.strictEqual(set.size, 2);
assert.strictEqual(set.length, 10);
// …has(3)/has(1), delete(3)+delete(4) then size, clear() then size+has…
set.forEach(function (number) { assert.strictEqual(number, array[i++]); });
assert.deepStrictEqual(obliterator.take(set.values()), [3, 6, 9]);
```

Characterising the shape of that coverage:

* **One length, 10, in all six blocks.** So one pointer width, one code path through
  `getPointerArray`, and every member the suite uses is comfortably in range.
* **Every set is fresh and tiny.** The largest holds three members; the largest index used is 9.
* **Deletion is only ever from a one-element set.** `set.add(3); set.delete(3); set.delete(4);`
  is the entire deletion coverage.
* **Iteration is drained immediately.** `obliterator.take(set.values())` creates a cursor and
  exhausts it in one expression, so nothing about the cursor's *state* can be observed.

## What upstream does NOT test

This is the section that carries the weight. Everything below is reachable through the public API
and never exercised by the original suite.

**Return values and chaining**

1. **`delete`'s return value is never used.** It is upstream's only method with a meaningful
   boolean result, and the block that calls it twice asserts only `size`.
2. **`add`'s return value is never used.** Upstream returns `this` for chaining; nothing asserts it.

**The structure's one interesting algorithm**

3. **The swap-with-last in `delete` is never observed.** Deleting a non-last member moves the last
   member into the hole and rewrites its index. The suite only ever deletes from a one-element set,
   where the swap is a self-assignment, so the branch that makes a sparse set O(1) is untested.
4. **Iteration order after a delete is never checked.** It is no longer insertion order, and that
   is the only externally visible consequence of (3).
5. **Deleting the last member, then a member that was moved, is never done.**
6. **Re-adding after `clear` is never done.** `clear` is O(1) and leaves live-looking debris in
   `dense`; that the debris stays unreachable is asserted once (`has(1) === false`) but never
   re-tested after the set is used again.
7. **Adding a member back after deleting it** is never done.
8. **The set is never filled to capacity.**

**The entire typed-array width machinery**

9. **Only one length is ever constructed: 10.** `getPointerArray(10)` returns `Uint8Array`, so the
   16-bit and 32-bit branches are **never reached** through this module and the `length > 2³²`
   throw is never reached.
10. **The truncating store is never triggered.** Reaching it needs a member ≥ 256; upstream's
    largest is 9.

**Out-of-range members — the whole regime**

11. **No member outside `0..length` is ever passed to anything.** That single omission hides all of
    the following, each of which is real upstream behaviour:
    * `has(m)` past the end is `false` — because `sparse[m]` is `undefined` and `undefined < size`
      is false;
    * `delete(m)` past the end is also `false`, but *by a different clause*: `undefined >= size` is
      **also** false, so evaluation continues to `dense[undefined] !== m`, which is true;
    * `add(m)` past the end **corrupts the set** — see "Bugs this found";
    * and therefore **`size` can exceed `length`**, which nothing in the module defends against.

**Iteration — everything except one immediate drain**

12. **Mutation during iteration is never performed.** Upstream's cursor freezes `size` and reads
    `dense` lazily (DESIGN.md §3.4 hybrid capture), so an element write mid-walk *is* visible and a
    length change is *not*. Neither half is tested.
13. **A cursor is never re-drained.** `obliterator.take` exhausts it once, so the
    non-restartability of D-06 is unobserved.
14. **`[...set]` is never used.** The suite reaches the cursor only through `set.values()`, so the
    collection-level `Symbol.iterator` — the *factory* half of D-07, and the half napi does not
    provide for free — has **zero** upstream coverage despite being the last line of the upstream
    module.
15. **`values()` on an empty set, or after `clear`, is never called.**
16. **The `undefined` window is never reached**, which follows from (11) and (12) together.

**`forEach`**

17. **The second callback argument is never inspected.** Upstream calls `callback.call(scope, item,
    item)` — the member as both value *and* key, which is how a `SparseSet` mimics a `Set`. The
    test's callback declares one parameter.
18. **`scope` is never passed**, so the `arguments.length > 1 ? scope : this` branch is untested.
19. **`forEach` on an empty set is never called.**

**Never called at all**

20. `inspect()` and the `nodejs.util.inspect.custom` symbol. ~20 LOC of the module.

## What we test in addition

Rust native tests, mapped to the gaps above.

`crates/mnemonist-core/src/structures/sparse_set.rs` — 19 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all six upstream blocks, as a baseline |
| `a_duplicate_add_changes_nothing_at_all` | 2 — and asserts both arrays are byte-identical, not just `size` |
| `delete_swaps_the_last_member_into_the_hole` | 1, 3, 4, 5 — deletes a middle member, then the member that moved |
| `deleting_the_last_member_is_a_self_swap` | 3, 5 |
| `clear_leaves_stale_entries_that_stay_unreachable` | 6 — asserts the debris is *there* and unreachable, pinning the O(1) clear against a future "tidy-up" |
| `reads_out_of_range_report_absence` | 11 (the `has`/`delete` half) |
| `an_out_of_range_add_corrupts_the_set_exactly_as_upstream_does` | 10, 11 — the compound defect, pinned value by value |
| `negative_members_arrive_as_their_two_s_complement_and_truncate_alike` | 11 — ToUint32 and a narrowing store compose to the same answer in both languages |
| `size_can_exceed_length_and_then_iteration_hits_the_gap` | 11, 16 — the `undefined` window, reached through public calls only |
| `a_delete_past_capacity_writes_dense_but_not_sparse` | 11 — the expando case, see "Bugs this found" |
| `cursors_do_not_restart_but_the_set_can_be_walked_again` | 13, 14 — both levels of D-07 in one test |
| `a_delete_during_iteration_is_visible_to_the_cursor` | 12 |
| `a_delete_ahead_of_the_cursor_can_yield_a_member_twice` | 12 — the nastier half: the swap makes the last member appear twice and the deleted one never |
| `an_add_during_iteration_is_not_visible_to_the_cursor` | 12 — the frozen-length half |
| `picks_one_pointer_width_for_both_arrays` | 9 — five lengths across both width boundaries |
| `rejects_a_length_no_pointer_array_can_index` | 9 (the throw) |
| `a_zero_length_set_accepts_nothing_and_finds_nothing` | 11, 15, 16 — the degenerate end of the corruption path |
| `a_one_member_set_behaves` | 7 |
| `fills_to_capacity_without_running_off_the_end` | 8 |

`crates/mnemonist-core/src/cursor/mod.rs` — 13 tests, covering the machinery itself against a
synthetic source rather than against `SparseSet`, so the semantics are pinned once for all ~30
modules that will use it: non-restartability, partial consumption, element writes visible, growth
invisible, shrink opening gaps rather than terminating, a fully emptied source, a reversed walk
(the `Stack.values()` shape), and the detached `CursorState` being driven across a mutation that a
borrowing cursor could not permit.

`crates/mnemonist-core/src/utils/typed_arrays.rs` — 10 tests, two of them new for this module:
`try_get` reporting the out-of-range read instead of panicking, and `try_set` dropping the
out-of-range write while still truncating the in-range one.

The **differential fuzzer** then covers gaps 1–16 continuously rather than at hand-picked points.
Both backing arrays are in the observable-state set, so the swap in `delete` and every truncating
store are compared slot for slot after *every* operation of *every* generated program; roughly one
generated member in eight is out of range; and the grammar interleaves cursor creation and stepping
with mutation, which is what D-21 has asked for since Wave 0 and what no previous module had the
surface to provide.

**Still untested, stated rather than glossed:** gap 20 (`inspect`, not bridged — a Node display
convenience with no upstream assertion), gap 17/18 in their `arguments.length` form (see the
divergence table), and non-integer members, which napi's `u32` coercion rejects before any port
code runs.

## Bugs this found

**B-8 — `add(member)` with `member >= length` corrupts the set, three defects deep.**
`status: verified against Node 24.18.1`. Upstream never validates `member`, and neither guard in
`add` fires for an out-of-range one, because `sparse[member]` is `undefined` and every comparison
against `undefined` is false. What follows is three separate silent failures in three lines:

```js
this.dense[this.size] = member;   // (1) TRUNCATES: add(300) on a length-10 set stores 44
this.sparse[member]   = this.size; // (2) DROPPED: out-of-range typed-array store is a no-op
this.size++;                       // (3) happens anyway
```

Measured on real Node: `new SparseSet(10)` then `add(300)` gives `size === 1`,
`dense === [44, 0, …]`, `sparse` untouched, and `has(300) === has(44) === false`. The member is
stored, counted, iterable, and unfindable under either name.

**B-9 — and therefore `size` can exceed `length`, which makes upstream's own iterator yield
`undefined`.** `status: verified against Node 24.18.1`. This is the second-order consequence and
the more interesting half. `values()` freezes `size`, and `dense` is a fixed-length typed array, so
once (3) above has pushed `size` past `length` the cursor reads off the end:

```js
var u = new SparseSet(2);
u.add(100); u.add(101); u.add(102); u.add(103);   // size 4, length 2
[...u]  // → [100, 101, undefined, undefined]
```

That is DESIGN.md §3.7's shrink window, reached **through the public API in four calls**, on a
module whose test file never passes an out-of-range member. §3.7 chose Option A (reproduce the
`undefined` gap) over Option B (terminate cleanly) on the grounds that no upstream *test* reaches
the window — true, and it is why Option B was measured as costing zero on the 40% axis. This
module shows the window is not exotic: **two calls reach it**
(`new SparseSet(0); s.add(0); Array.from(s)` → `[undefined]`), and the differential fuzzer finds
it in 0.3 seconds when the port takes Option B. Option A was the right call for a stronger reason
than the one recorded.

**B-10 — `delete` past capacity writes a string-keyed expando onto a typed array.**
`status: verified against Node 24.18.1`. Upstream's swap is

```js
index = this.dense[this.size - 1];     // undefined once size > length
this.dense[this.sparse[member]] = index;
this.sparse[index]              = this.sparse[member];
```

and the two stores then behave *differently*: `dense[slot] = undefined` still lands, as `0`,
because a `NaN` element store is `0`; but `sparse[undefined]` is a **property**, not element 0. It
creates `sparse.undefined` — an expando no method ever reads — and leaves the array untouched.
`new SparseSet(3)`, add `0/1/2/99`, `delete(1)` leaves `dense = [0, 0, 2]`, `sparse = [0, 1, 2]`
and `sparse.undefined = 1`.

**This one caught the port.** The first cut wrote `sparse[0]`, which is what reading the three
lines as a unit produces rather than reading them statement by statement. It is fixed
(`5a38c3c`), pinned by `a_delete_past_capacity_writes_dense_but_not_sparse`, and it was
subsequently used as falsification sabotage A for the fuzzer, which caught it in 6.6 seconds.

**A defect in our own harness, found while porting this module.** `status: fixed (3120085)`.
Not an upstream bug, recorded here because it invalidated evidence this project had already
published. proptest's `TestRunner` counts successes for its whole lifetime and loops
`while successes < config.cases`, so the campaign driver's reuse of one runner across batches meant
**every batch after the first executed no new cases at all** — only the persisted regression
corpus, which proptest replays before the (now empty) main loop, and then spun at 100% CPU until
the deadline. The recorded `static-disjoint-set` campaigns of "16,666 cases" were 32 genuinely new
programs plus two saved seeds re-run ~8,300 times each. Measured decisively: with the corpus file
removed, a 120-second campaign dropped from 16,666 cases to 32.

It surfaced here only because `sparse-set` had no corpus yet, so instead of quietly repeating two
programs the driver spun visibly and `--duration 20` reported 32 cases in 20.0 seconds. Both
`static-disjoint-set` campaigns have been re-run and re-logged; the superseded lines are kept in
`fuzz/log.txt` under a correction block rather than deleted. Pinned by
`every_batch_generates_new_cases`, which runs with **no** corpus so that the only way past `batch`
cases is a batch that really generated.

**What the fuzzer found: nothing new.** Two campaigns, 2.94 M operations, zero divergences. As with
`static-disjoint-set`, that is the expected outcome — a faithful port reproduces upstream's bugs,
so differential fuzzing structurally cannot find them (D-33). B-8, B-9 and B-10 were all found by
reading the file statement by statement and confirming each step against Node. What the fuzzer is
for is the other direction, and it was proven to work in that direction twice (see Fuzz, below).

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **Out-of-range members are reproduced, not guarded.** | The opposite call to `static-disjoint-set`, whose bridge raises a `RangeError`. The difference is upstream's behaviour, not a change of mind: there, upstream propagates `NaN` through arithmetic and no honest Rust reproduction exists; here every step is a well-defined read, a truncating store or a dropped store, so the faithful port *is* expressible — and it is cheaper than a guard as well as more useful. |
| D-09 | **The shrink window is reproduced (Option A), not collapsed.** | `Step` has three states, not two: `Gap` is `{done: false, value: undefined}` and is distinct from `Done`. Reachable in two public calls on this module; falsified below. |
| — | **`Yield` is `Either<u32, Undefined>`, not `Option<u32>`.** | §3.7 step 3 left open whether napi can express `undefined` in a yield slot. It can, but **not** through `Option`: napi renders `None` as `null`, and `null` is not `undefined` to `deepStrictEqual`. `Either::B(())` is a real `undefined`, which frees `Option` to keep its own meaning — `None` is `{done: true}`. |
| D-06 | **No collection implements `IntoIterator`.** | It would hand out a fresh iterator per `for` loop and silently restart, where upstream's cursor continues from where it stopped. Collections expose `values()`; the `Cursor` is the stateful thing. |
| D-07 | **`Symbol.iterator` is installed from Rust, not from the shim.** | The factory half is the one napi does not provide. It runs from napi's module-export hook, driven by a table, so `require('@port/addon').SparseSet` is spreadable on its own. A shim that added semantics would mean the addon was incomplete without the test harness. |
| — | **The Rust-side `Iterator` impl skips gaps rather than stopping.** | Rust has no `undefined` to yield, and stopping would turn a shrink into an early end — the exact divergence Option A exists to avoid. Skipping gives a Rust caller the same sequence of *real* elements a JS caller filtering `undefined` would see. The faithful three-way primitive is `step()`; the `Iterator` impl is the convenience built on it. |
| — | **`add` returns `bool` in core; the bridge returns `this`.** | Core reports whether the member was newly inserted, which upstream exposes only through `size`. The bridge drops it so the JS surface matches exactly. |
| — | **`dense` and `sparse` are not exposed to JS.** | They are public typed arrays upstream and a JS caller can write *through* them. napi can only hand out a copy of a Rust `Vec`, so exposing them would silently break write-through — worse than their absence. They are exposed in Rust, and the differential fuzzer compares both slot for slot on every op, so the representation is verified rather than merely hidden. |
| — | **`forEach(cb, undefined)` binds `this` to the set.** | Upstream keys off `arguments.length > 1`, which napi's typed signature cannot see: "omitted" and "passed as `undefined`" are the same value. Upstream would bind `undefined`, and therefore `globalThis` for a sloppy-mode callback. The omitted-argument case — the only one the original suite uses — is exact, and passing a real scope object is exact. |
| — | **`new()` returns `Result`.** | Upstream throws for `length > 2³²`. Same treatment as `static-disjoint-set`. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion and no Rust equivalent. |

## Fuzz + bench

### Fuzz

```
module=sparse-set seed=42       cases=19006 ops=1961772 wall=120.0s divergences=0
module=sparse-set seed=20260801 cases=9425  ops=980895  wall=60.0s  divergences=0
```

Two campaigns, two seeds, **2.94 M operations, zero divergences**. Reproduce with
`target/release/difffuzz --module sparse-set --seed 42 --cases 19006`.

* **Op alphabet:** `add(m)` (weight 5) · `delete(m)` (2) · `has(m)` (2) · `clear()` (1) ·
  `$iter("values")` (2) · `$next()` (3) · `$spread()` (1).
* **Observable state, compared after every op:** `size`, `length`, **`dense` and `sparse`**. Both
  backing arrays are public upstream, and comparing them slot for slot is what makes the swap in
  `delete` and every truncating store checkable directly rather than only through their eventual
  effect on iteration order.
* **Lengths:** `0..=400`. Zero is included because `new SparseSet(0)` is legal and every member is
  then out of range; 400 straddles 256, where both arrays switch to 16-bit and where a truncating
  `dense` store starts folding distinct members onto the same value.
* **Members:** `0..length + 64`, so roughly **one in eight is out of range**.
* **Program length:** 1..200 ops.
* **Deliberately excluded: nothing.** This is the contrast with `static-disjoint-set`, which had to
  exclude out-of-range indices. Here they are the most interesting part of the grammar, because
  out-of-range `add` is the only route to `size > length` and therefore the only route to the
  `undefined` window.

The `$` ops are protocol rather than methods (`fuzz/oracle.js`): `$iter` opens the one cursor each
side holds, `$next` steps it against whatever the set has become since, and `$spread` is
`Array.from(set)` — going through the *collection's* `Symbol.iterator` and so constructing a fresh
cursor every time. `$spread` is separate from `$next` on purpose: the factory half of D-07 is only
observable by comparing an op that must restart against one that must not, and folding them
together would leave every non-interleaved program still passing. This is the first grammar in the
repo that satisfies D-21.

**The fuzzer was falsified twice, once per half of the grammar.** A fuzzer that has never been seen
to fail is just a second green light — the lesson gate 6 exists for, applied to the fuzzer itself.
Both sabotages were reverted and both seeds are committed with provenance in
`crates/difffuzz/proptest-regressions/sparse-set.txt`, where proptest replays them before any novel
case on every subsequent run.

**A — the out-of-range half.** Sabotage: `delete`'s two swap stores treated as if they behaved
alike past capacity, i.e. writing `sparse[0]` where upstream writes an expando. This is the mistake
the port actually made first (B-10). Caught in **1,416 cases (6.6 s)**, shrunk from 200 ops to
seven:

```js
var s = new SparseSet(5);
s.add(0); s.add(1); s.add(2); s.add(4);
s.add(5);      // out of range: dense takes it, sparse does not
s.add(5);      // size runs to 6 against a length of 5
s.delete(1);   // port sparse [1,1,2,0,3], upstream [0,1,2,0,3]
```

**B — the cursor half.** Sabotage: one line in `mnemonist-core/src/cursor`, returning `Step::Done`
where the faithful port returns `Step::Gap` — exactly DESIGN.md §3.7's rejected Option B. Caught in
**352 cases (0.3 s)**, shrunk to two operations:

```js
var s = new SparseSet(0);
s.add(0);        // out of range on a zero-length set: size becomes 1
Array.from(s);   // port [], upstream [undefined]
```

### Falsification of the port (gate 6)

Separate from the fuzzer falsifications above: gate 6 asks that sabotaging the core turns the
*original mocha suite* red, proving it exercises Rust rather than a JS fallback.

**The assertion the sabotage had to break was named first:**
`should be possible to create an iterator over the set's values` —
`assert.deepStrictEqual(obliterator.take(set.values()), [3, 6, 9])`, at `test/sparse-set.js:74`.
It was chosen because it is the only assertion in the file that reaches the new cursor machinery,
which is the code this unit exists to prove out.

**The sabotage:** `Sequence::freeze` for `SparseSet` returning `dense.len()` — the set's
*capacity* — instead of `self.size`, which is the single most plausible way to mis-port
`var size = this.size`.

**Confirmed red**, and red in precisely the named place: `5 passing, 1 failing`, the failure being
that assertion, with `actual` `[3, 6, 9, 0, 0, 0, 0, 0, 0, 0]` against `expected` `[3, 6, 9]`.
Reverted; **confirmed green again**: `6 passing`.

### Bench

`bench/results.json` → `modules["sparse-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, 32 MB L3, WSL2, Node 24.18.1, rustc 1.97.1.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, pinned to CPUs 2–3.

**`mixed-1e6`** — 1e6 `add`/`has`/`delete` (50/25/25) over length 1e6, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **8.9** | 11.5 | 1.3× faster |
| p99 ns/op | 24.9 | 25.6 | **a tie** |
| RSS delta MB | **11.4** | 39.0 | |
| structure-only RSS delta MB | **1.4** | 10.2 | |
| startup ms | **0.6** | 15.1 | reported separately; not throughput |

**`mixed-4e6`** — the same op mix at four times the length:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **13.8** | 21.2 | 1.5× faster |
| p99 ns/op | **240.4** | 324.3 | 1.3× |
| min ns/op | **9.0** | 13.8 | |
| structure-only RSS delta MB | **12.9** | 21.6 | |

**`drain-1e5`** — full iteration: a length-1e5 set prefilled by 1e5 random `add`s (leaving 63,061
distinct members), then 100 complete walks, one timed sample per walk:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/element | **0.94** | 3.18 | 3.4× faster |
| p99 ns/element | **1.66** | 4.76 | 2.9× |
| structure-only RSS delta MB | **0.4** | 6.6 | |

No regressions on any workload. `bench/drive.js` derives the `regressions` array mechanically from
the published metrics, so one cannot be quietly dropped from a run.

**Two honest readings, both of which cut against the port.**

*The `mixed-1e6` p99 is a tie, not a win.* 24.9 against 25.6 is 3%, and this host has already
demonstrated a noise band an order of magnitude wider than that: upstream's own p99 on
`static-disjoint-set` swung 32% between two clean runs. Anything under roughly 1.5× on this table
should be read as "no difference".

*The `drain` workload is the one place the port's advantage is structural rather than incidental,
and it is worth being precise about why it is not more.* Upstream's cursor is a closure
allocating a fresh `{value: …}` object per step, so 3.18 ns/element against 0.94 is largely V8's
allocation and property-write cost, not an algorithmic difference — both sides do one array read
and one increment per element. **This is an unconfirmed attribution**: distinguishing allocation
cost from megamorphic property access would need V8 allocation counters or `--trace-gc`, and
neither has been run. What is confirmed is the ratio and the checksum agreement
(`315169152400` on both sides, so both walked the same 6.3 M elements and summed them identically).

**Why `drain` batches by the walk rather than by 1000 elements.** A cursor costs something per
*walk* as well as per element — it freezes state at creation (§3.4) — and splitting a walk across
fixed-size samples would bury that fixed cost in whichever sample happened to contain the creation.
`batch_k` therefore carries members-per-walk (63,061) instead of a constant, so `ns / batch_k`
still means nanoseconds per element. Both sides derive it independently and the driver's checksum
gate would fail if they disagreed.

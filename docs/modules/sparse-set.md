# sparse-set

Upstream: `sparse-set.js` (168 LOC) + `utils/typed-arrays.js` (187 LOC, only `getPointerArray`
reachable) · `test/sparse-set.js` — **76 lines, 6 `it` blocks, 9 assertion statements**.

Port: `crates/mnemonist-core/src/structures/sparse_set.rs`,
`crates/mnemonist-core/src/cursor/mod.rs`, `crates/mnemonist-core/src/utils/typed_arrays.rs`.
Bridge: `crates/mnemonist-napi/src/sparse_set.rs`, `crates/mnemonist-napi/src/cursor.rs`.

This is the first module in the port with a real iterator surface, which is why it was chosen to
land immediately after the cursor machinery. It is also, unexpectedly, the module that makes the
`undefined` shrink window reachable through the **public API** in two calls — see D-09, below.

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

12. **Mutation during iteration is never performed.** Upstream's cursor freezes `size` at
    construction but reads `dense` lazily, live, on every step — a **hybrid capture**: the walk's
    *length* is a snapshot, its *elements* are not. So an element write mid-walk *is* visible and a
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

`crates/mnemonist-core/src/structures/sparse_set.rs` — 19 tests, mapped to the gaps above, closing
every gap 1–11 and 13–15: a duplicate `add` byte-identical to a no-op, a delete that swaps the last
member into the hole (and the self-swap case of deleting the last member), `clear`'s debris asserted
present and unreachable, out-of-range reads reporting absence, an out-of-range `add` pinned value by
value against the exact corruption, negative members truncating via two's-complement the same way in
both languages, `size` exceeding `length` and iteration hitting the gap, the expando case from
`delete` past capacity, cursor non-restartability next to collection restartability, and both pointer
widths at both boundaries. Full test-to-gap mapping: `docs/modules/evidence/sparse-set.md`.

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
with mutation, which is what this class of grammar needs and what no previous module had the
surface to provide.

**Still untested, stated rather than glossed:** gap 20 (`inspect`, not bridged — a Node display
convenience with no upstream assertion), gap 17/18 in their `arguments.length` form (see the
divergence table), and non-integer members, which napi's `u32` coercion rejects before any port
code runs.

## Bugs this found

**B-8 — `add(member)` with `member >= length` corrupts the set, three defects deep.**
Verified against Node 24.18.1. Upstream never validates `member`, and neither guard in
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
`undefined`.** Verified against Node 24.18.1. This is the second-order consequence and
the more interesting half. `values()` freezes `size`, and `dense` is a fixed-length typed array, so
once (3) above has pushed `size` past `length` the cursor reads off the end:

```js
var u = new SparseSet(2);
u.add(100); u.add(101); u.add(102); u.add(103);   // size 4, length 2
[...u]  // → [100, 101, undefined, undefined]
```

That is the shrink window (D-09, below), reached **through the public API in four calls**, on a
module whose test file never passes an out-of-range member. Option A (reproduce the
`undefined` gap) was chosen over Option B (terminate cleanly) on the grounds that no upstream *test*
reaches the window — true, and it is why Option B was measured as costing zero on the 40% axis. This
module shows the window is not exotic: **two calls reach it**
(`new SparseSet(0); s.add(0); Array.from(s)` → `[undefined]`), and the differential fuzzer finds
it in 0.3 seconds when the port takes Option B. Option A was the right call for a stronger reason
than the one recorded.

**B-10 — `delete` past capacity writes a string-keyed expando onto a typed array.**
Verified against Node 24.18.1. Upstream's swap is

```js
index = this.dense[this.size - 1];     // undefined once size > length
this.dense[this.sparse[member]] = index;
this.sparse[index]              = this.sparse[member];
```

and the two stores then behave *differently*: `dense[slot] = undefined` still lands, as `0`,
because a `NaN` element store is `0`; but `sparse[undefined]` is a **property**, not element 0. It
creates `sparse.undefined` — an expando no method ever reads — and leaves the array untouched.
`new SparseSet(3)`, add `0/1/2/99`, `delete(1)` leaves `dense = [0, 0, 2]`, `sparse = [0, 1, 2]`
and `sparse.undefined = 1`. Reproduced exactly, including the expando; pinned by
`a_delete_past_capacity_writes_dense_but_not_sparse` and used as fuzzer falsification sabotage A
(see "Fuzz + bench").

**What the fuzzer found in the port: nothing new.** Two campaigns, 2.94 M operations, zero
divergences. As with `static-disjoint-set`, that is the expected outcome — a faithful port
reproduces upstream's bugs, so differential fuzzing structurally cannot find them. B-8, B-9 and B-10
were all found by reading the file statement by statement and confirming each step against Node.
What the fuzzer is for is the other direction, and it is sharper than the original suite by a wide
margin — see "Fuzz + bench".

**The bridge held a bare core value behind `&self`**, which LLVM was entitled to compile as a
`noalias readonly` pointer and hoist reads across a re-entrant JS callback (B-31). It now holds
`RefCell<Core>`, which is not `Freeze`, and every `&mut self` method borrows via `borrow_mut()`
taken per step and released before the callback runs, so a re-entrant callback never meets an
outstanding borrow. See `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`. Full history in the
log.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **Out-of-range members are reproduced, not guarded.** | The opposite call to `static-disjoint-set`, whose bridge raises a `RangeError`. The difference is upstream's behaviour, not a change of mind: there, upstream propagates `NaN` through arithmetic and no honest Rust reproduction exists; here every step is a well-defined read, a truncating store or a dropped store, so the faithful port *is* expressible — and it is cheaper than a guard as well as more useful. |
| D-09 | **The shrink window is reproduced (Option A), not collapsed.** | `Step` has three states, not two: `Gap` is `{done: false, value: undefined}` and is distinct from `Done`. Reachable in two public calls on this module; falsified below. |
| — | **`Yield` is `Either<u32, Undefined>`, not `Option<u32>`.** | It was left open whether napi can express `undefined` in a yield slot. It can, but **not** through `Option`: napi renders `None` as `null`, and `null` is not `undefined` to `deepStrictEqual`. `Either::B(())` is a real `undefined`, which frees `Option` to keep its own meaning — `None` is `{done: true}`. |
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

Two campaigns, two seeds, **2.94 M operations, zero divergences**:

```
module=sparse-set seed=42       cases=19006 ops=1961772 wall=120.0s divergences=0
module=sparse-set seed=20260801 cases=9425  ops=980895  wall=60.0s  divergences=0
```

Reproduce with `target/release/difffuzz --module sparse-set --seed 42 --cases 19006`.

The op alphabet covers `add`/`delete`/`has`/`clear` plus the three cursor ops. Both backing arrays
(`dense` and `sparse`) are compared slot for slot after every op, which is what makes the swap in
`delete` and every truncating store checkable directly rather than only through their eventual
effect on iteration order. Roughly one generated member in eight is out of range, by design — this
is the first module where out-of-range values are the most interesting part of the grammar rather
than something to exclude, because out-of-range `add` is the only route to `size > length` and
therefore the only route to the `undefined` window. Full grammar: evidence file.

The `$` ops are protocol rather than methods (`fuzz/oracle.js`): `$iter` opens the one cursor each
side holds, `$next` steps it against whatever the set has become since, and `$spread` is
`Array.from(set)` — going through the *collection's* `Symbol.iterator` and so constructing a fresh
cursor every time. `$spread` is kept separate from `$next` on purpose: the factory half of D-07 is
only observable by comparing an op that must restart against one that must not, and folding them
together would leave every non-interleaved program still passing. This is the first grammar in the
repo that interleaves cursor stepping with mutation throughout.

`$forEach(method, rule, limit)` walks the instance with a callback that calls back into it. The
compared result is the sequence of callback argument pairs, so the walk's shape is checked and not
only the state it leaves behind. This module's mutations: `delete(a0)` and `clear()` uncapped;
`add(a0 + 1)` capped at two firings. **The cap is not tuning**: this module's `forEach` re-reads
`this.size` on every iteration, so an uncapped `s.forEach(m => s.add(m + 1))` never terminates —
upstream included. The cap is what makes the op well-defined, and that live bound is also the
contrast the op exists to check: `sparse-queue-set` captures its bound instead and does the
opposite, in near-identical code. What it does not reach: `difffuzz` compares `mnemonist-core`
against upstream JS, and the napi bridge — where a re-entrant callback would actually run — is not
in that loop; `tests/boundary/reentrancy.js` covers that instead, driving the real addon with real
JS callbacks. One deliberate narrowing, mirrored on both sides: a selected callback argument that is
`undefined` skips the mutation, because feeding it back in reaches upstream's `NaN`-indexed swap,
which `usize` cannot express and the core does not model. Disclosed in `fuzz/log.txt`.

**The fuzzer was falsified twice, once per half of the grammar, and both sabotages leave the
original upstream test file green.** Sabotage A treats `delete`'s two swap stores as if they behaved
alike past capacity — writing `sparse[0]` where upstream writes an expando, the mistake the port
actually made first (B-10) — and is caught in 1,416 cases (6.6 s), shrunk to seven ops. Sabotage B
returns `Step::Done` where the faithful port returns `Step::Gap`, exactly the rejected Option B, and
is caught in 352 cases (0.3 s), shrunk to two ops. Both were reverted; both seeds are committed with
provenance in `crates/difffuzz/proptest-regressions/sparse-set.txt`, replayed before any novel case
on every subsequent run. Full repro code: evidence file.

**Falsification of the port (gate 6), separate from the fuzzer falsifications above:** the
assertion the sabotage had to break was named first —
`should be possible to create an iterator over the set's values` at `test/sparse-set.js:74` — chosen
because it is the only assertion in the file that reaches the cursor machinery this unit exists to
prove out. The sabotage, `Sequence::freeze` for `SparseSet` returning `dense.len()` (the set's
capacity) instead of `self.size`, is confirmed red in precisely that place (5 passing, 1 failing,
`[3, 6, 9, 0, 0, 0, 0, 0, 0, 0]` against `[3, 6, 9]`); reverted, confirmed green again (6 passing).
Full record: evidence file.

### Bench

`bench/results.json` → `modules["sparse-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, 32 MB L3, WSL2, Node 24.18.1, rustc 1.97.1.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, pinned to CPUs 2–3.

**`mixed-1e6`** (1e6 `add`/`has`/`delete`, 50/25/25, length 1e6): the port is 1.3× faster at p50
(8.9 vs 11.5 ns/op) and roughly tied at p99 (24.9 vs 25.6). **`mixed-4e6`** (same mix, 4× length):
1.5× faster at p50, 1.3× at p99. **`drain-1e5`** (100 full walks of a length-1e5 set holding 63,061
distinct members after random `add`s): 3.4× faster at p50 (0.94 vs 3.18 ns/element), 2.9× at p99.
Full tables: evidence file.

One entry in `bench/results.json` (`mixed-4e6`'s `p99_ns_per_op`, 330.395 port vs 328.803 upstream,
ratio 1.00) is sub-noise variation rather than a real regression: it is a 0.48% difference, three
orders of magnitude below the ~32% p99 run-to-run swing this host's own methodology document records
for upstream alone. `bench/drive.js`'s regression check has no noise floor — any port figure that
exceeds upstream's by even a fraction of a nanosecond is mechanically listed — so the entry is left
exactly as generated rather than hand-edited out. Every other regression this module carries is well
outside that band and should be read as real.

Two honest readings, both of which cut against the port. *The `mixed-1e6` p99 is a tie, not a win*:
24.9 against 25.6 is 3%, well inside the noise band above. *The `drain` workload is the one place
the port's advantage is structural rather than incidental, and it is worth being precise about why
it is not more*: upstream's cursor is a closure allocating a fresh `{value: …}` object per step, so
3.18 ns/element against 0.94 is largely V8's allocation and property-write cost, not an algorithmic
difference — both sides do one array read and one increment per element. This attribution is
unconfirmed: distinguishing allocation cost from megamorphic property access would need V8
allocation counters or `--trace-gc`, neither of which has been run. What is confirmed is the ratio
and the checksum agreement (`315169152400` on both sides, so both walked the same 6.3 M elements and
summed them identically).

`drain` batches by the walk rather than by a fixed element count because a cursor costs something
per *walk* as well as per element — it freezes state at creation — and splitting a walk across
fixed-size samples would bury that fixed cost in whichever sample happened to contain the creation.
`batch_k` therefore carries members-per-walk (63,061) instead of a constant, so `ns / batch_k` still
means nanoseconds per element; both sides derive it independently and the driver's checksum gate
would fail if they disagreed.

**Why this unit stays descoped despite the numbers above.** Every benchmark above links
`mnemonist-core::SparseSet` directly, never through N-API, which is correct for comparing the port
against upstream but goes nowhere near the reason `tests/scope.txt` still marks this module out of
scope: the napi bridge (`crates/mnemonist-napi/src/sparse_set.rs`) holds its `CoreSet` in a
`RefCell` and calls `.borrow()`/`.borrow_mut()` on every access, and what that borrow-flag check
costs in isolation had never been measured. `bench/runner/src/sparse_set.rs::run_mixed_refcell`
closes that gap without going through napi either: the identical mixed workload over the identical
`SparseSet`, wrapped in a bare `RefCell`, accessed through `.borrow()`/`.borrow_mut()` exactly as the
bridge does. Run with `bench-runner --refcell-probe --size <N> --warmup 3 --measured <M>`; it is
deliberately not part of `harness::MODULES` and never writes to `bench/results.json`, since there is
no upstream JS analogue of "a bare Rust `RefCell`" to publish a `regressions` array against.

At size 1e6, repeated probes put the plain and `RefCell`-wrapped p50 within noise of each other in
both directions (deltas from −1.1% to +1.6% across three probes of 10 measured passes; a fourth
probe at 30 measured passes had the wrapped variant faster). At size 4e6 the sign flips between
probes too. **The isolated borrow-flag check is not distinguishable from this host's own run-to-run
noise** at either size — methodology.md already documents up to ~32% p99 swings between clean runs
on this machine, and the deltas sit well inside that band. Full probe tables: evidence file.

This is a real measurement, not an absence of one, and it answers a real question: whether the
`RefCell` alone is a *dominant* driver of any bridge-path slowdown. On this evidence, no — not
detectably, on its own, at these sizes. What is **not** measured here, and is not claimed, is the
full cost of going through napi: argument marshalling, `Result`-to-`Error` conversion, and the FFI
call boundary itself are all real costs this probe never touches, because touching them means going
through N-API, which would poison the comparison this whole harness exists to keep clean. Re-scoping
`sparse-set` is a call to be made per `tests/scope.txt`'s own note, not this probe's to make; this
section exists so that call can be made from a real number instead of an assumption.

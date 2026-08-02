# bit-vector

Upstream: `bit-vector.js` (550 LOC) + `utils/bitwise.js` (109 LOC, see
`docs/modules/utils-bitwise.md`) + `obliterator/iterator` · `test/bit-vector.js` — **320 lines,
21 `it` blocks, 96 assertion statements**.

Port: `crates/mnemonist-core/src/structures/bit_vector.rs` +
`crates/mnemonist-core/src/structures/bits.rs` + `crates/mnemonist-core/src/utils/bitwise.rs` +
`crates/mnemonist-core/src/cursor/mod.rs`.
Bridge: `crates/mnemonist-napi/src/bit_vector.rs`, `crates/mnemonist-napi/src/cursor.rs`.
Shim: `tests/bridge/bit-vector.js`.

The largest module in this batch, and the one with the best test:source ratio of the four. It shares
`bits.rs` with `bit-set` because **upstream copy-pastes seven methods between the two files** — so
B-17 and B-18 arrive here for free, exactly as they arrived upstream for free. Both were
re-verified against `BitVector` on Node rather than inferred from `BitSet`.

---

## What upstream tests

Twenty-one blocks. Eleven are `bit-set`'s tests with the class name changed; ten are this module's
own, and they are where the interesting coverage sits:

* **The growth policy is genuinely well tested**: a custom `capacity + 32`, a custom `capacity + 2`,
  a policy that returns the same capacity (asserted to throw), and the default. This is the best
  covered part of either bit module.
* **`reallocate` is tested in both directions**, including a shrink that moves `length`.
* **`push`/`pop` are tested**, and the `pop` block walks the exact sequence that exposes this
  module's central defect — then asserts the one index that hides it.
* **`resize` up and down**, with the capacity checked after each.
* **Out-of-bound `get`/`test`** are asserted, at index 17 on a vector of length 5.
* **`set` out of bounds is asserted to throw**, on a vector of length 0.

What it still never does: any index between `length` and the end of the allocated region, any
`reset` of a bit that is not set, any `select` that skips a word, any mutation during iteration, and
any assertion about `size` after a `pop`.

## What upstream does NOT test

**`push`/`pop`, one assertion short**

1. **`get(0)` after the pop/push(0) sequence.** The `pop` test does
   `push(1); push(1); pop(); pop(); push(0); push(1)` and then asserts `get(1)`. Asking about
   `get(0)` instead would have returned `1` and exposed B-21 on the spot.
2. **`size` is never read after a `pop`.** It is asserted eleven times elsewhere in the file and not
   once here.
3. **Pushing `1` onto a slot that already holds `1`** — i.e. re-pushing after a pop — is never
   checked against `size`.
4. **`rank(length)` is never compared with `size`**, which is the cheapest possible detector for all
   of the above.

**The bounds guard**

5. **`set(length, v)` is never called.** The guard is `this.length < index`, so one-past-the-end is
   admitted and writes into the capacity region without moving `length`. The out-of-bound test uses
   index 17 against length 5 — twelve past the interesting index.
6. **`get(length)`** is likewise never called; it reads the capacity region rather than answering
   `undefined`.

**Capacity and length coming apart**

7. **A vector with capacity but zero length is never iterated.** `new BitVector(); v.grow();` then
   `forEach` calls back **32 times** on a vector of length 0 — B-22.
8. **`reallocate` to `0`** is never done.
9. **A shrinking `reallocate` that discards a word holding set bits** is never done.
10. **`reallocate` where the rounded capacity is unchanged but `length` still gets clamped** — the
    early-return ordering — is never done.
11. **`grow(capacity)`'s policy *loop*** is never exercised past one iteration: every `grow` in the
    file needs a single application.

**The policy**

12. **A policy returning a non-number** is never tested, though `applyPolicy` explicitly checks for
    it.
13. **A policy returning a negative** is never tested, though `applyPolicy` explicitly checks for it.
14. **A non-integer policy result** is never tested; it is accepted and rounded up to a word.
15. **`applyPolicy(0)`** — where `override || this.capacity` falls back — is never called directly.

**Inherited from `bit-set`, and untested here for the same reasons**

16. `reset` on an already-clear bit (B-17). 17. `select` skipping an empty word (B-18).
18. `select` past the population, and on an empty vector. 19. Indices past `length` but inside the
last word (B-23). 20. Mutation during iteration, and re-draining a cursor. 21. `forEach`'s `scope`.

**Never called at all**

22. `inspect()` and the `nodejs.util.inspect.custom` symbol.

## What we test in addition

`crates/mnemonist-core/src/structures/bit_vector.rs` — 19 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all twenty-one upstream blocks, as a baseline |
| `pop_leaves_size_and_the_bits_behind` | 1, 2, 3, 4 — B-21, the whole sequence with the assertion upstream skipped |
| `pushing_true_onto_an_already_set_slot_counts_it_twice` | 3 |
| `set_at_length_writes_a_bit_that_length_does_not_cover` | 5, 6 |
| `get_is_undefined_only_strictly_past_the_length` | 6 |
| `a_zero_length_vector_with_capacity_still_iterates_a_whole_word` | 7 — B-22 |
| `a_length_that_exactly_fills_its_words_walks_all_of_them` | 7 — the same misfire where it is also correct |
| `reallocate_clamps_length_even_when_the_capacity_does_not_change` | 10 |
| `reallocate_to_zero_drops_the_array_and_the_length` | 8 |
| `a_shrinking_reallocate_discards_the_words_above_the_cut` | 9 |
| `a_zero_override_falls_back_to_the_current_capacity` | 15 |
| `a_policy_can_fail_three_ways` | 12, 13 — and our own refusal |
| `a_non_integer_policy_result_is_rounded_up_to_a_word` | 14 |
| `grow_loops_the_policy_until_it_covers_the_target` | 11 — seven applications |
| `to_json_takes_one_word_past_the_length_clamped_by_the_array` | — five lengths, including the clamped case |
| `reallocate_detaches_an_open_cursor` | 20 |
| `growth_during_iteration_is_invisible_to_an_open_cursor` | 20 |
| `cursors_do_not_restart_but_the_vector_can_be_walked_again` | 20 |
| `an_initial_length_of_thirty_derives_a_capacity_of_thirty_two` | — the `initialLength \|\| initialCapacity` quirk |
| `inherits_the_reset_and_select_defects_verbatim` | 16, 17 — re-verified against `BitVector`, not inferred |
| `indices_past_the_backing_array_are_inert` | 19 |

Plus the 13 tests on the shared `bits.rs`, listed in `docs/modules/bit-set.md`.

**Still untested, stated rather than glossed:** gap 22 (`inspect`, not ported), gap 21 in its
`arguments.length` form, and a JS caller writing *through* `vector.array` (both in the divergence
table).

## Bugs this found

**B-21 — `pop` maintains neither `size` nor the bit, and `push(0)` clears nothing.**
`status: VERIFIED against Node 24.18.1`. Three defects in six lines:

```js
BitVector.prototype.push = function (value) {
  if (this.capacity === this.length) this.grow();
  if (value === 0 || value === false) return ++this.length;   // (1) no store, no clear
  this.size++;                                                // (2) unconditional
  var index = this.length++, …
  this.array[byteIndex] |= (1 << pos);
};
BitVector.prototype.pop = function () {
  if (this.length === 0) return;
  var index = --this.length;                                  // (3) size and bit untouched
  return (this.array[byteIndex] >> pos) & 1;
};
```

So `size` stops being the population as soon as anything is popped:

```js
var v = new BitVector();
v.push(1); v.push(1);   // size 2
v.pop(); v.pop();       // length 0 -- size STILL 2, bits STILL set
v.push(0);              // length 1
v.get(0)                // 1, not 0     <-- upstream's test asserts get(1) instead
v.push(1);              // size 3, with two bits actually set
```

**Upstream's own `pop` test performs exactly this sequence** and asserts `v.get(1) === 1`, which is
true either way. One index to the left and it would have failed.

**B-22 — `length % 32 || 32` treats a length of 0 as a full final word.**
`status: VERIFIED against Node 24.18.1`. The `|| 32` exists for a length that fills its last word
exactly, and `0 % 32` is also falsy. `BitSet` cannot reach it — its array is empty when its length
is — but `BitVector` can, because capacity outlives length: `new BitVector(); v.grow();` then
`forEach` calls back 32 times on a vector of length 0.

**And the same `length < index` off-by-one as `HashedArrayTree`** (B-16's shape, in a different
file): `set(length, v)` writes into the capacity region without moving `length`. Measured:
`new BitVector(5); set(5, 1)` gives `size === 1`, `get(5) === 1`, `test(5) === true`, `rank(5) === 0`
and iteration over five bits. Two of this batch's four modules have the identical guard bug, written
independently.

**Inherited verbatim from the copy-paste:** B-17 (`reset`'s missing `>>> 0` driving `size` negative)
and B-18 (`select` losing 32 positions per skipped word). Both re-measured against `BitVector` on
Node. See `docs/modules/bit-set.md` for the analysis and `docs/modules/utils-bitwise.md` for B-19
and B-20, also in this unit's require-closure.


### B-31 — `&self` on a `Freeze` type was `noalias readonly` (fixed 2026-08-01)

This bridge held a bare core value, so `&self` compiled to a `noalias readonly` pointer and LLVM was
entitled to hoist reads across the JS callback — which it did. It now holds `RefCell<Core>`, which
is not `Freeze`, and every `&mut self` method became `&self` + `borrow_mut()`. The borrow is taken
per step and released before the callback runs, so a re-entrant callback never meets an outstanding
borrow. See `planning/NOTES.md` B-31 and `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`.

**And the one place the fix could not be applied cleanly — worth reading before copying this
pattern.** The rule the `RefCell` imposes is that no borrow may be alive across a call that can run
JavaScript. Everywhere else in the bridge that is achievable: `forEach` re-borrows per step,
`DefaultMap::get` runs its factory between the read and the write. Here it is not, because the
**growth policy is JavaScript that `mnemonist-core` calls from inside `grow`** — so `push`, `set`,
`grow`, `resize`, `reallocate` and `apply_policy` hold the vector while a JS function runs.

The failure mode is not graceful. A `RefCell` panic inside a `#[napi]` method **aborts the
process**: napi 3.12 does not `catch_unwind` a sync call, and a panic unwinding out of an
`extern "C"` frame is an abort. Measured on a policy that did nothing but read `vector.length`.
Every borrow in this bridge is therefore fallible and raises a named error instead — see
`REENTRANT_POLICY`, decision B31-b, and the two `BitVector` policy specs in
`tests/boundary/reentrancy.js`. Upstream would serve such a call from a half-grown vector; this port
refuses it. That is a stated narrowing, and it replaces an abort, which replaced undefined
behaviour.

## Deliberate divergences

Everything in `docs/modules/bit-set.md`'s table applies — the shared store, the word-caching cursor,
the signed `size`, the `i64` indices, `array` exposed as a copy, the strict `value === 0` test,
`select`'s `Either`, the `forEach` scope caveat, and `inspect` not being ported. Additionally:

| # | Divergence | Why |
|---|---|---|
| — | **The growth policy is `Box<dyn Fn(f64) -> Option<f64>>`.** | `None` is upstream's `typeof newCapacity !== 'number'`, which a JS policy really can produce and which `applyPolicy` explicitly checks for. `f64` in and out because a policy result of `40.5` is accepted upstream and rounded to a word. |
| — | **A throwing JS policy is re-raised by the bridge, not by the core.** | The core's `Option` has nowhere to put an exception, so `JsPolicy` parks it in a `RefCell` and the calling method prefers it over the core's classification. Without that, a throwing policy would surface as "policy returned an invalid value" — a different error from a different place. |
| — | **A policy returning `NaN` or `Infinity` is refused.** | Upstream's guard is `typeof !== 'number' \|\| < 0`, and `NaN` passes both because every `NaN` comparison is false. It then flows into `Math.ceil(NaN / 32) * 32` and `new Uint32Array(NaN)`. There is no honest Rust reproduction of an allocation of `NaN` elements, so it raises instead — the same call D-37 makes for `StaticDisjointSet`'s out-of-range reads. |
| — | **`BitVector` is not `Clone`-equivalent across policies.** | `Box<dyn Fn>` cannot be cloned, so `Clone` copies the bits and the capacity and resets the policy to the default. Nothing upstream clones a vector, and silently sharing a policy would be worse than a documented reset. |
| — | **The constructor's `initialLength \|\| initialCapacity` union is resolved in the bridge.** | Upstream reads `initialLength \|\| initialCapacity \|\| 0`, so `{initialCapacity: 30}` sets the **length**. The core takes a length and nothing else; the quirk is reproduced at the boundary where the JS object exists, and pinned by a test on the arithmetic that follows from it. |

## Fuzz + bench

### Fuzz

```
module=bit-vector seed=42       cases=21852 ops=2250634 wall=120.0s divergences=0
module=bit-vector seed=20260801 cases=9617  ops=981385  wall=60.0s  divergences=0
```

Two campaigns, two seeds, **3.23 M operations, zero divergences**. Reproduce with
`target/release/difffuzz --module bit-vector --seed 42 --cases 21852`.

Both campaigns were accidentally launched twice, concurrently. The duplicates are commented out in
`fuzz/log.txt` rather than deleted, and rather than summed: a pair shares a seed and therefore
largely the same programs. The withdrawn numbers are kept visible because the pairs are an
accidental measurement of the contention effect that has gate 10 deferred here -- same seed, same
wall budget, same machine, overlapping in time gave 21,852 vs 20,657 cases and 9,617 vs 10,603.
A wall-clock-bounded campaign is not reproducible in case count; only `--cases` is.

* **Op alphabet:** `set(i)` (3) · `set(i, 0)` (2) · `reset(i)` (3) · `flip(i)` (2) · `get(i)` (2) ·
  `test(i)` (1) · `rank(i)` (2) · `select(r)` (2) · **`push(1)` (3) · `push(0)` (3) · `pop()` (3)** ·
  `resize(l)` (2) · `reallocate(c)` (2) · `grow(c)` (1) · `grow()` (1) · `$iter("values")` (1) ·
  `$iter("entries")` (1) · `$next()` (3) · `$spread()` (1).
* **Observable state, compared after every op:** `size`, `length`, `capacity`, **`array`** and
  `toJSON()`.
* **Initial lengths:** `0..=200`. **Indices:** `0..length + 64`. **Extents:** `0..512`.
* **Program length:** 1..200 ops.
* `push(1)` and `push(0)` are separate ops on purpose: only the former touches `size` and only the
  latter leaves a stale bit, and B-21 needs both interleaved with `pop`.
* `set` is the only op in this grammar that throws, and its message is compared in full through the
  `{"$throw": …}` encoding added for `hashed-array-tree`.

**Deliberately excluded: custom growth policies.** Upstream's policy is a JS function and a
generated program is JSON. The default policy is therefore the only one fuzzed — and since the
default is strictly increasing, **both throws in `applyPolicy` are unreachable from this grammar**.
They are covered by native tests in `mnemonist-core` instead (`a_policy_can_fail_three_ways`,
`a_non_integer_policy_result_is_rounded_up_to_a_word`). Stated explicitly because a silently
narrowed grammar reads as "we covered everything" when it did not.

**The fuzzer was falsified before it was trusted** (D-32). Sabotage: `pop` made to clear the bit it
returns and to decrement `size` — which is what `pop` is supposed to do, and the single most
plausible repair anyone would make to this module. Caught in **1,075 cases (1.0 s)** and shrunk from
200 ops to **two**:

```js
var s = new BitVector(0);
s.push(1);
s.pop();
// port     array [0], size 0, toJSON [0]
// upstream array [1], size 1, toJSON [1]
```

Two operations, three of the five observed fields disagreeing — and upstream's own `pop` test
performs that exact pair, then asserts only the returned value and `length`. Reverted; the seed is
committed with provenance in `crates/difffuzz/proptest-regressions/bit-vector.txt`.

### Falsification of the port (gate 6)

**Named first:** `should throw if the policy returns an irrelevant size.` →
`assert.throws(function () { vector.push(1); }, /policy/)` at `test/bit-vector.js:291`. Chosen
because the policy machinery is the best-covered part of the upstream file, so a sabotage there has
a real assertion to break.

**The sabotage:** `applyPolicy`'s `newCapacity <= this.capacity` weakened to `<`, i.e. accepting a
policy that returns exactly the current capacity. A boundary flip, not a deletion.

**Confirmed red**, at exactly the named line: `20 passing, 1 failing`, "Missing expected exception"
at `test/bit-vector.js:291`. Reverted; **confirmed green again**: 21 passing.

**Recorded because it is the gate's own lesson, and this module has two examples of it.** Neither of
B-21's halves could have served as the sabotage:

* "Fixing" `push(0)` to clear its slot leaves the suite **green**, because every slot the push test
  writes over is already zero.
* "Fixing" `pop` to decrement `size` leaves it **green** too, because no assertion in the file reads
  `size` after a `pop`.

Both would have been sabotages incapable of failing — which is exactly the miss DESIGN.md §1.1 was
written about. Across this batch of four modules, **five** plausible-looking sabotages were rejected
on that ground before a usable one was found.

### Bench

**Not run.** Gate 10 is deliberately outstanding: benchmarks need an idle machine, and this unit was
ported alongside two others in parallel worktrees, where a contended run has already been measured on
this project to inflate both sides 2–3× (NOTES.md, H+5). It is batched into a separate quiet pass,
and `bit-vector` is therefore **not** in `tests/scope.txt` yet — by DESIGN.md §1.1 it is not done
until it is. Gates 1–9 are green.

One thing the eventual benchmark should watch for, recorded now so it is not rationalised later: the
shared store is an `Rc<RefCell<Vec<u32>>>`, and every `set`/`reset`/`flip` takes a `RefCell` borrow
that upstream does not pay for. On operations that are otherwise one load, one OR and one store,
that overhead is not obviously negligible. It bought exact reproduction of `clear`/`reallocate`
detaching an open cursor; whether it costs anything measurable is an open question, not an assumed
answer.

### `$forEach` — the op that was missing (added 2026-08-01, B-31)

`bit-vector`'s grammar had no `forEach` op at all. That omission is what let B-31 — a `forEach`
callback mutating the collection it is walking — through 3.23 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it.

`$forEach(method, rule, limit)` now walks the instance with a callback that calls back into it.
The compared result is the sequence of callback argument pairs, so the walk's **shape** is checked
and not only the state it leaves behind. This module's mutations:

* `set(a1)`, `reset(a1)`, `flip(a1)`, `pop()` uncapped; `push()` capped at four.

`push`'s cap **is** tuning and is stated as such: the outer bound is captured, so an uncapped push
still terminates, but a push per bit over a 400-bit vector is hundreds of reallocations per case and
the throughput buys more programs than the depth does. `set` and `push` can throw from the growth
policy; the throw is reported alongside the steps already taken rather than instead of them, so the
two sides never agree on less than they know.

**What it does not reach, stated so the campaign is not over-read.** `difffuzz` compares
`mnemonist-core` against upstream JS; the napi bridge, where B-31's hoisted read actually lived, is
not in that loop. No op alphabet can catch that class of bug here. The specs that do are
`tests/boundary/reentrancy.js`, which drive the real addon with real JS callbacks — red on the
pre-fix bridges, green after.

One deliberate narrowing, mirrored on both sides: a selected callback argument that is `undefined`
skips the mutation. Feeding it back in reaches upstream's `NaN`-indexed swap, which `usize` cannot
express and the core does not model. Fully disclosed in `fuzz/log.txt`.

### Bench

`bench/results.json` → `modules["bit-vector"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`get`/`pop` (50/25/25), `vector`/`hashed-array-tree`'s shape
(this module grows under `push`, unlike `bit-set`'s fixed domain, so there is no capacity parameter
to set), xorshift32 seed 42. `rank`/`select` excluded for the reason recorded in `bit-set`'s own
bench doc: neither has an index behind it, so a single call is O(i / 32) words, and a
uniform-weighted mix would put a domain-scaling cost next to three genuinely O(1) ops.

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 8.1 | **8.3** | tie |
| p99 ns/op | **12.9** | 14.5 | 1.1× faster |
| min ns/op | **7.5** | 7.8 | tie |
| RSS delta MB | **6.1** | 17.8 | |
| structure-only RSS delta MB | **1.3** | 9.8 | |
| startup ms | **0.6** | 16.4 | 27× (reported separately; not throughput) |

**No regressions, but the narrowest margin of the eleven mixed workloads in this batch** — p50 and
min are effectively ties (within 3%, well inside the noise band methodology.md documents: up to
~32% p99 swings between clean runs on this host). A probe at 4e6 domain (single measured pass, not
committed as a second workload row) confirmed the same picture rather than revealing a boundary:
port still ahead on p50/p99/min at that scale, by a similar small margin, with the *sign* of the
gap flipping between individual passes at both sizes — this is noise, not a trend. `push`/`pop`/
`get` here are all single-word bit operations once the vector is allocated, the same shape
`bit-set`'s zero-overhead bit ops have, which is plausibly why this is the one growable module in
the batch that comes closest to parity rather than winning decisively like `vector`/
`hashed-array-tree` do. Unconfirmed: not isolated by profiling, offered as the mechanism most
consistent with the numbers.

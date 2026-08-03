# bit-set

Upstream: `bit-set.js` (379 LOC) + `utils/bitwise.js` (109 LOC, see `docs/modules/utils-bitwise.md`)
+ `obliterator/iterator` · `test/bit-set.js` — **189 lines, 12 `it` blocks, 60 assertion
statements**.

Port: `crates/mnemonist-core/src/structures/bit_set.rs` + `crates/mnemonist-core/src/structures/bits.rs`
+ `crates/mnemonist-core/src/utils/bitwise.rs` + `crates/mnemonist-core/src/cursor/mod.rs`.
Bridge: `crates/mnemonist-napi/src/bit_set.rs`, `crates/mnemonist-napi/src/cursor.rs`.
Shim: `tests/bridge/bit-set.js`.

**`bits.rs` is shared with `bit-vector`**, because upstream ships the two files with seven methods
copy-pasted between them — `reset`, `flip`, `rank`, `select`, `forEach`, `values` and `entries` are
byte-identical, and `set` differs only by a bounds guard. Every defect below is therefore present
**twice** upstream, and is written once here.

---

## What upstream tests

Twelve `it` blocks, and the shape of the coverage matters more than the list:

* **Four of the twelve are about one thing**: that `size` tracks `set`/`flip`/`reset` correctly at
  bit 31. Three were clearly added in response to a real bug (`should count set bits when only last
  bit is set`, `… when flipping bits`, `… when only last bit is reset`), and a fourth,
  `length divisible by 32 iteration, issue #117.`, cites an upstream issue number. So the file's own
  history says the sign boundary and the last-word boundary have both bitten before.
* **`reset` is called four times in the whole file**, and every one clears a bit that is actually
  set.
* **`select` is tested on a `BitSet(11)`** — one word. No word is ever skipped.
* **`rank` is tested on a `BitSet(8010)`**, deliberately not a multiple of 32, with 80 evenly spaced
  bits. This is the only test in the file that reaches more than three words.
* **Every index passed to anything is inside `length`.**
* **Iteration is drained immediately** — `obliterator.take(set.values())` — except in the #117 test,
  which drives `next()` by hand but never mutates in between.
* **`clear()` is never called.**

## What upstream does NOT test

**The `size` counter, past the three cases that were already found the hard way**

1. **`reset` on a bit that is already clear.** Every `reset` in the file clears a set bit. This is
   the precondition for B-17, and it is one call away from the assertions that exist.
2. **That `size` and `rank(length)` agree.** They are computed by completely different means — a
   running counter versus a popcount — and no assertion ever compares them.
3. **`size` after `clear()`**, since `clear` is never called.
4. **Repeated `set` of the same bit**, i.e. that the counter is idempotent.

**`select`, beyond one word**

5. **No `select` call ever skips an empty word**, which is the precondition for B-18.
6. **`select(r)` for `r` past the population** — the branch where upstream falls out of its loop and
   returns `undefined`, a third return shape the tests never see.
7. **`select(0)`**, which matches before any bit is counted and answers the position of the first
   *zero* bit.
8. **`select` on an empty set**, i.e. the `-1` branch.

**Out-of-range indices — the entire regime**

9. **No index outside `0..length` is ever passed to anything.** That hides both the fact that
   out-of-range indices are inert (which is the answer to "does the `SparseSet` corruption family
   recur?" — see below) *and* B-23, the band between `length` and the end of the last allocated word
   where a bit is accepted and then invisible.
10. **Negative indices**, which upstream turns into a negative word index and drops.

**`clear`**

11. **`clear()` is never called at all** — not its effect on `size`, not that the set is reusable
    afterwards, and not that it **reallocates**, which is observable through an open cursor.

**Iteration**

12. **Mutation during iteration is never performed.** Upstream's cursor captures the array *object*,
    so a `clear` mid-walk is invisible to it while a write into a word it has not yet entered is
    visible. Neither half is tested.
13. **A cursor is never re-drained** (D-06), and **`[...set]` is never used** (the factory half of
    D-07, which is the last line of the upstream module).
14. **`values()`/`entries()` on an empty set** is never called.
15. **`forEach`'s `scope` argument** is never passed.

**Never called at all**

16. `inspect()` and the `nodejs.util.inspect.custom` symbol — ~18 LOC.

## What we test in addition

`crates/mnemonist-core/src/structures/bits.rs` — 13 tests, against the shared store, so the
semantics are pinned once for both modules:

| Test | Closes gap |
|---|---|
| `a_no_op_reset_decrements_size_when_the_top_bit_of_the_word_is_set` | 1 — B-17, including `size == -2` |
| `a_no_op_reset_is_harmless_while_the_top_bit_is_clear` | 1 — the control that explains why upstream never noticed |
| `rank_returns_zero_whenever_the_size_counter_is_zero` | 2 — the propagation into `rank` |
| `select_loses_thirty_two_positions_per_skipped_word` | 5 — B-18 |
| `select_answers_minus_one_a_position_or_undefined` | 6, 7, 8 — all three return shapes |
| `out_of_range_indices_are_inert_rather_than_corrupting` | 9 |
| `a_bit_past_length_but_inside_the_word_is_counted_yet_invisible` | 9 — B-23 |
| `a_cursor_keeps_the_array_it_was_opened_over` | 11, 12 |
| `writes_ahead_of_the_cursor_are_visible_but_not_within_the_current_word` | 12 — the word-granularity half |
| `a_walk_is_not_restartable` | 13 |
| `entries_pair_each_bit_with_its_ordinal` | — |
| `the_last_word_is_full_when_the_length_is_a_multiple_of_thirty_two` | — B-22, the `\|\| 32` misfire |
| `cloning_copies_the_backing_store` | — a port-side invariant, not an upstream one |

`crates/mnemonist-core/src/structures/bit_set.rs` — 15 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all twelve upstream blocks, as a baseline |
| `a_reset_that_clears_nothing_still_decrements_size` | 1 |
| `a_corrupted_size_makes_rank_lie` | 2 — and that `select` bails on the same counter |
| `the_same_reset_is_harmless_while_the_words_top_bit_is_clear` | 1 |
| `select_loses_a_word_of_positions_for_every_empty_word_it_skips` | 5 |
| `select_off_the_end_is_undefined_and_out_of_range_is_minus_one` | 6, 7, 8 |
| `indices_past_the_backing_array_are_inert` | 9, 10 |
| `a_bit_between_length_and_the_end_of_its_word_is_counted_but_unreachable` | 9 |
| `clear_detaches_an_open_cursor_from_the_words_it_zeroes` | 11, 12 |
| `clear_resets_size_and_the_set_is_reusable` | 3, 11 |
| `repeated_sets_and_resets_are_idempotent_in_size` | 4 |
| `a_zero_length_set_holds_and_yields_nothing` | 14 |
| `iteration_yields_exactly_length_bits` | 14 — nine lengths across both word boundaries |
| `cursors_do_not_restart_but_the_set_can_be_walked_again` | 13 — both levels of D-07 |
| `writes_during_iteration_are_visible_only_beyond_the_current_word` | 12 |
| `rank_saturates_past_the_end_rather_than_reading_off_it` | 9 |
| `allocates_one_word_per_thirty_two_bits_rounded_up` | — |

**Still untested, stated rather than glossed:** gap 16 (`inspect`, not ported), gap 15 in its
`arguments.length` form (see the divergence table), and a JS caller writing *through* `set.array`,
which the bridge cannot support (also in the table).

## Bugs this found

**B-17 — `reset` omits the `>>> 0` that `set` and `flip` apply, so `size` drifts and can go
negative.** `status: VERIFIED against Node 24.18.1`. `size` is never a popcount; it is maintained by
comparing the word before and after each write, which is only valid if both readings are unsigned.
`set` and `flip` say so:

```js
newBytes = this.array[byteIndex] |= (1 << pos);
newBytes = newBytes >>> 0;                        // <-- reset() does NOT do this
if (newBytes > oldBytes) this.size++;
```

`reset` compares the **signed** value of the compound assignment against the **unsigned**
`Uint32Array` read. On any word whose bit 31 is set the signed value is negative, so
`newBytes < oldBytes` holds whether or not the reset changed anything:

```js
var s = new BitSet(32);
s.set(31);      // size 1
s.reset(0);     // bit 0 was ALREADY clear
s.size          // 0   -- and bit 31 is still set
s.rank(32)      // 0   -- rank early-returns on size === 0, so it lies too
```

Three no-op resets give `size === -2`. This is the strongest find in the batch: a one-token omission,
three lines from two correct call sites, whose consequence propagates into both `rank` and `select`
(each bails on `size === 0`). It is why `Words::size` is an `i64` — a `usize` cannot hold the state
upstream reaches.

**B-18 — `select` does not advance its position across the words it skips.**
`status: VERIFIED against Node 24.18.1`.

```js
if (byte === 0) continue;              // <-- p is NOT advanced by 32 here
for (var j = 0; j < b; j++, p++) { … }
```

Every all-zero word before the answer costs the result 32. `new BitSet(64); s.set(40)` answers
`select(1) === 8`. With bits at 3 and 70 in a `BitSet(96)`, `select(1) === 3` (right — nothing
skipped) and `select(2) === 38` (wrong by exactly one word). Invisible upstream because both
`select` tests use a length of 11.

**B-23 — an index past `length` but inside the last allocated word is accepted, and then invisible.**
`status: VERIFIED against Node 24.18.1`. `new BitSet(10)` allocates one 32-bit word, and `set(20)`
lands in it: `size === 1`, `array === [1048576]`, while `rank(10) === 0`, `select(1) === undefined`
and iteration yields ten zeros. `size` disagrees with every other view of the same set.

**Asked and answered: the `SparseSet` out-of-range family does NOT recur here.** Both modules are
typed-array-backed and the question is the obvious one. Measured on Node:

| call, index past the array | upstream | why |
|---|---|---|
| `set` / `reset` / `flip` | **no-op**, `size` unchanged | the store is dropped, and every comparison against `undefined` is false |
| `get` | `0` | `undefined >> pos` is `0` |
| `test` | `false` | — |

`new BitSet(10).set(1000)` leaves `size === 0` and the array untouched. The structural difference is
that `SparseSet` **increments its counter unconditionally** after a dropped store (B-8), where
`BitSet` derives its counter from a before/after comparison that an `undefined` read makes inert.
B-23 is the narrow survivor of the family, and it is a *reachability* gap rather than corruption.

See also `docs/modules/utils-bitwise.md` for B-19 and B-20, found in this unit's require-closure.


### B-31 — `&self` on a `Freeze` type was `noalias readonly` (fixed 2026-08-01)

This bridge held a bare core value, so `&self` compiled to a `noalias readonly` pointer and LLVM was
entitled to hoist reads across the JS callback — which it did. It now holds `RefCell<Core>`, which
is not `Freeze`, and every `&mut self` method became `&self` + `borrow_mut()`. The borrow is taken
per step and released before the callback runs, so a re-entrant callback never meets an outstanding
borrow. See B-31, above, and `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **The word store is `Rc<RefCell<Vec<u32>>>`.** | `clear()` **replaces** `this.array`, and `values()` captures the array object, so a cursor opened beforehand keeps reading the pre-clear words — measured on Node. Reading through `&self` in `Sequence::slot` would have shown the new array. `Rc<Vec<_>>` with copy-on-write would have made element writes *invisible* to an open cursor, which is the opposite divergence. Every borrow is confined to one method call, so two can never overlap. |
| — | **The cursor caches the current word.** | Upstream reads `byte = array[i++]` once per **word**, not once per bit, so a write into the word being walked is invisible while a write into the next word is visible. A `Cell<Option<(usize, u32)>>` in `BitWindow` reproduces exactly that. |
| — | **`size` is `i64`, not `usize`.** | B-17 takes it negative. A `usize` could not represent the state upstream reaches, and clamping at zero would be a silent divergence in the one field the module is about. |
| — | **Indices are `i64` with a real ToInt32, not `usize`.** | Every upstream use is a bitwise expression, so a negative index gives a negative word index and is dropped. A `usize` coercion would turn `set(-1)` into `set(4294967295)` — the same outcome by accident for `set`, and a 134-million-iteration loop inside `rank`. |
| — | **`Step::Gap` is unreachable here.** | Unlike `SparseSet`, where the shrink window is reachable in two public calls. The cursor keeps its own array alive and no upstream method resizes a word vector in place, so every ordinal below the frozen length has a word behind it. The bridge's yield type is therefore a plain `Option<u32>` rather than `Either<u32, Undefined>` — stated because the *absence* of the gap here is a claim, not an oversight. |
| — | **`array` IS exposed to JS, as a copy.** | The original test reads `set.array.length`, so it has to exist — unlike `SparseSet`'s `dense`/`sparse`, which are hidden for exactly this reason. napi can only hand out a copy, so a JS caller writing *through* it is a silent divergence. The differential fuzzer compares the real backing store on the Rust side after every operation, so the representation is verified rather than merely exposed. |
| — | **`set(index, value)` matches on the JS value's type.** | Upstream's test is a strict `value === 0 \|\| value === false`, not truthiness: `set(i, null)` and `set(i, '')` both *set* the bit. Coercing to a `bool` at the boundary would have been wrong in both directions. |
| — | **`select` yields `Either<i64, Undefined>`.** | Three outcomes — `-1`, a position, and upstream's loop fall-through — and D-39 says `Option` renders the third as `null`. |
| — | **`forEach(cb, undefined)` binds `this` to the set.** | Upstream keys off `arguments.length > 1`, which napi's typed signature cannot see. The omitted case — the only one the original suite uses — is exact, as is passing a real scope object. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion. |

## Fuzz + bench

### Fuzz

```
module=bit-set seed=42       cases=26373 ops=2625894 wall=120.0s divergences=0
module=bit-set seed=20260801 cases=12871 ops=1293656 wall=60.0s divergences=0
```

Two campaigns, two seeds, **3.92 M operations, zero divergences**. Reproduce with
`target/release/difffuzz --module bit-set --seed 42 --cases 26373`.

* **Op alphabet:** `set(i)` (weight 4) · `set(i, 0)` (2) · **`reset(i)` (3)** · `flip(i)` (2) ·
  `get(i)` (2) · `test(i)` (1) · `rank(i)` (2) · `select(r)` (2) · `clear()` (1) ·
  `$iter("values")` (1) · `$iter("entries")` (1) · `$next()` (3) · `$spread()` (1).
* **Observable state, compared after every op:** `size`, `length`, **`array`** and `toJSON()`.
  `array` is the point — `size` alone would agree in plenty of programs where the words had already
  diverged.
* **Lengths:** `0..=400`, thirteen words. Zero is included because `new BitSet(0)` allocates nothing
  and is the degenerate end of every guard; 400 is sparse enough that empty words between set bits
  are routine, which is what B-18 needs.
* **Indices:** `0..length + 64`, so a steady fraction land in B-23's band and beyond it.
* **Program length:** 1..200 ops.
* **Deliberately excluded: nothing.** Out-of-range indices, negative-adjacent behaviour and cursor
  interleaving are all generated. `reset` is weighted **up** rather than down, because B-17 only
  misfires on a bit that is already clear, and a low weight would make that rare rather than routine.

`$iter` alternates between `values` and `entries`: they share an implementation in this port and are
separate closures upstream, so fuzzing only one would leave the other unchecked. `clear()` is in the
alphabet *because* it interacts with an open cursor.

**The fuzzer was falsified before it was trusted**. Sabotage: `reset` given the `>>> 0`
upstream forgot — B-17 *fixed*, which is the single most plausible thing a future cleanup does to
this file. Caught in **1,325 cases (2.0 s)** and shrunk from 200 ops to **two**:

```js
var s = new BitSet(1);
s.set(31);      // inside word 0, past length 1 -- accepted (B-23)
s.reset(0);     // clears nothing; upstream decrements anyway
// port size 1, upstream size 0
```

Worth noting what that two-op program shows: **B-23 and B-17 compound.** The `set(31)` is only
possible because an index past `length` but inside the last word is accepted, and it is what puts
bit 31 into the word, which is the precondition for `reset`'s signed comparison to misfire. Neither
defect alone reaches the state, and no upstream test passes an index past `length` at all. Reverted;
the seed is committed with provenance in `crates/difffuzz/proptest-regressions/bit-set.txt`.

### Falsification of the port (gate 6)

**Named first:** `length divisible by 32 iteration, issue #117.` →
`assert.strictEqual(counter, set.length)` at `test/bit-set.js:178`. Chosen because it is the only
assertion in the file that depends on the last-word width calculation, and because upstream issue
#117 exists precisely because that calculation was once wrong.

**The sabotage:** `length % 32 || 32` reduced to `length % 32` — "fixing" a guard that genuinely
looks like a bug, and which B-22 shows *is* one at length 0.

**Confirmed red**, at exactly the named line: `11 passing, 1 failing`, `32 !== 64` at
`test/bit-set.js:178`. Reverted; **confirmed green again**: 12 passing.

**Recorded because it is the gate's own lesson: neither of this module's headline defects could have
served as the sabotage.** "Fixing" `reset`'s missing `>>> 0` leaves the suite green, because every
`reset` in the file clears a bit that is actually set. "Fixing" `select` leaves it green too, because
its `select` test uses a length of 11 and never skips a word. Both would have been sabotages
incapable of failing — the exact failure mode gate 6 exists to catch, and here there were two of
them waiting.

### `$forEach` — the op that was missing (added 2026-08-01, B-31)

`bit-set`'s grammar had no `forEach` op at all. That omission is what let B-31 — a `forEach`
callback mutating the collection it is walking — through 3.92 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it.

`$forEach(method, rule, limit)` now walks the instance with a callback that calls back into it.
The compared result is the sequence of callback argument pairs, so the walk's **shape** is checked
and not only the state it leaves behind. This module's mutations:

* `set(a1)`, `reset(a1)`, `flip(a1)` and `clear()`, all uncapped.

Uncapped is safe because a `BitSet` cannot grow and the outer bound is `this.array.length`, read
once. What the op checks is the **word snapshot**: upstream lifts `byte = this.array[i]` out of the
inner loop, so a callback that writes to the word currently being walked is invisible for the rest
of that word and visible in the next one.

**What it does not reach, stated so the campaign is not over-read.** `difffuzz` compares
`mnemonist-core` against upstream JS; the napi bridge, where B-31's hoisted read actually lived, is
not in that loop. No op alphabet can catch that class of bug here. The specs that do are
`tests/boundary/reentrancy.js`, which drive the real addon with real JS callbacks — red on the
pre-fix bridges, green after.

One deliberate narrowing, mirrored on both sides: a selected callback argument that is `undefined`
skips the mutation. Feeding it back in reaches upstream's `NaN`-indexed swap, which `usize` cannot
express and the core does not model. Fully disclosed in `fuzz/log.txt`.

### Bench

`bench/results.json` → `modules["bit-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`reset`/`get`/`test` (50/25/25) over capacity 1e6, xorshift32
seed 42. `rank` is deliberately excluded: it has no rank/select index behind it on either side, so
a single call is O(i / 32) words, and a 25%-weighted mix at this domain made the harness spend ten
minutes on six of ten reps before it was killed — see `bench/runner/src/bit_set.rs` for the full
account.

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 8.87 | **7.94** | 1.12× slower |

**Fixed 2026-08-03 — the index conversion was the whole margin.** `split` in
`crates/mnemonist-core/src/structures/bits.rs`, reached by every one of `set`, `reset`, `flip` and
`get`, converted its `i64` index to `f64` and ran JavaScript's full `ToInt32`: `trunc`, then
`rem_euclid(2^32)`, then a sign fixup. That path exists for indices outside `i32`'s range — which is
exactly what upstream's out-of-range reads reach, and those are reproduced bug-for-bug here — but
`ToInt32` is the *identity* for any value already inside that range: `trunc` is a no-op on an
integral value, and `rem_euclid` of a value already in `[-2^31, 2^31)` returns it unchanged once the
sign fixup is applied.

`split` now tries `i32::try_from` first and falls back to the float path only when the index really
does not fit. The equivalence was checked over 2.6 million values including every boundary
(`i32::MIN`, `i32::MAX`, ±2^31, ±2^32) rather than argued from the definition alone.

Six runs alternating the old and new code: the port's p50 is **8.66–8.70 ns before and 5.86–5.93 ns
after**, with upstream steady at 7.85–7.91 ns throughout, so the port-side change is unambiguous —
about 32%. This module now reads **1.31× faster** than upstream where it read 1.10× slower.
`bit-vector` shares `split` and moved with it, from a tie to 1.30× faster.
| p99 ns/op | 16.30 | **14.87** | 1.10× slower |
| RSS delta MB | **6.1** | 17.6 | |
| structure-only RSS delta MB | **1.3** | 9.8 | |
| startup ms | **0.6** | 17.9 | 30× (reported separately; not throughput) |

**This is the module the port was predicted to win largest on, and on raw ns/op it
does not — a real, disclosed regression, not a rounding artefact.** Both p50 and p99 are ~10–12%
slower than V8's own typed-array path over three independent metrics (p50, p99, min), which rules
out a single unlucky batch. The original explanation was unconfirmed: `BitSet::set`/`reset`/`get`/
`test` each go through `Words::set_bit`/`get_bit` (`crates/mnemonist-core/src/structures/bits.rs`),
an extra call frame LLVM may not always inline as aggressively as V8 inlines a monomorphic
`Uint32Array` element access at this op's simplicity.

**Confirmed 2026-08-02, and refined** — `bench/runner/src/bit_set.rs`, reachable via
`bench-runner --bit-set-probe`, runs three variants of the identical op stream: the real `BitSet`
(`Rc<RefCell<Vec<u32>>>` behind `Words`, `i64` indices through a real `ToInt32`), a bare
`Vec<u32>` with plain `usize` indices and no `RefCell` at all, and a third variant *between* the two
— the same bare `Vec<u32>`, still no `RefCell`, but indices still pushed through the exact
`f64`-based `to_int32`/`rem_euclid` conversion `Words::split` uses:

| variant | p50 ns/op |
|---|---|
| wrapped `BitSet` (`RefCell` + `Words` + `to_int32`) | 8.456 |
| bare `Vec<u32>` + `to_int32`, no `RefCell` | 4.489 |
| bare `Vec<u32>`, plain `usize`, no `RefCell`, no `to_int32` | **3.026** |

**Verdict: confirmed, but the named mechanism was incomplete.** The isolated gap (5.43 ns/op)
splits roughly 73%/27% between the `RefCell`/`Words` wrapper layer (3.97 ns/op) and the `to_int32`
conversion alone (1.46 ns/op) — the wrapper is the larger piece, consistent with what was named, but
`to_int32`'s call to `f64::rem_euclid` (a real floating-point division, not a missed-inlining
artefact) is a second, previously undocumented contributor nearly a third the size of the first, and
the doc's "extra call frame LLVM may not inline" framing does not cover it — that framing describes
a compiler decision; `rem_euclid` is real arithmetic work that would cost the same fully inlined.
Both bare variants beat upstream's own published p50 (7.935 ns) outright, same pattern as `heap`'s
own confirmation above: the overhead here is larger than the entire measured regression.

What is confirmed independently of any of this is the memory and startup side: the port uses roughly
a third of the RSS and starts thirty times faster, exactly where a `Vec<u32>` versus a full V8
process should differ.

**Fix not attempted.** Both layers are load-bearing, not incidental: the `Rc<RefCell<Vec<u32>>>` is
`Words`'s own re-entrancy story (shared with `BitVector`, whose `length` is mutable and whose cursor
must keep reading a `clear()`'d array — see `bits.rs`'s own module docs), and the `f64`-based
`to_int32` is what makes a negative index drop cleanly rather than wrap through `usize` the way a
naive Rust port would (`bits.rs`'s own docs on `set(-1)`). A `usize` fast path guarded by "index is
non-negative and small" is conceivable, but it is a `crates/mnemonist-core` behaviour-preserving
optimisation, not a local tweak, and would need bit-set's fuzz campaign and bench figures re-run
before it could stand — out of scope here. Recorded as a proposal for later.

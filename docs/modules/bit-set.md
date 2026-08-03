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
semantics are pinned once for both modules, closing gaps 1, 2, 5–9 and 11–13. Full test-to-gap
mapping: evidence file.

`crates/mnemonist-core/src/structures/bit_set.rs` — 15 tests, closing every remaining gap except 15
and 16: a 1:1 reproduction of all twelve upstream blocks as a baseline, the same reset defect
attributed to the port, `select`'s three return shapes, out-of-range and negative indices inert,
`clear` detaching an open cursor and resetting size, idempotent repeated sets, and iteration across
both word boundaries. Full test-to-gap mapping: evidence file.

**Still untested, stated rather than glossed:** gap 16 (`inspect`, not ported), gap 15 in its
`arguments.length` form (see the divergence table), and a JS caller writing *through* `set.array`,
which the bridge cannot support (also in the table).

## Bugs this found

**B-17 — `reset` omits the `>>> 0` that `set` and `flip` apply, so `size` drifts and can go
negative.** Verified against Node 24.18.1. `size` is never a popcount; it is maintained by
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
Verified against Node 24.18.1.

```js
if (byte === 0) continue;              // <-- p is NOT advanced by 32 here
for (var j = 0; j < b; j++, p++) { … }
```

Every all-zero word before the answer costs the result 32. `new BitSet(64); s.set(40)` answers
`select(1) === 8`. With bits at 3 and 70 in a `BitSet(96)`, `select(1) === 3` (right — nothing
skipped) and `select(2) === 38` (wrong by exactly one word). Invisible upstream because both
`select` tests use a length of 11.

**B-23 — an index past `length` but inside the last allocated word is accepted, and then invisible.**
Verified against Node 24.18.1. `new BitSet(10)` allocates one 32-bit word, and `set(20)`
lands in it: `size === 1`, `array === [1048576]`, while `rank(10) === 0`, `select(1) === undefined`
and iteration yields ten zeros. `size` disagrees with every other view of the same set.

**The `SparseSet` out-of-range family does NOT recur here.** Both modules are typed-array-backed and
the question is the obvious one. Measured on Node:

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

**The bridge held a bare core value behind `&self`**, which LLVM was entitled to compile as a
`noalias readonly` pointer and hoist reads across a re-entrant JS callback (B-31). It now holds
`RefCell<Core>`, which is not `Freeze`, and every `&mut self` method borrows via `borrow_mut()`
taken per step and released before the callback runs, so a re-entrant callback never meets an
outstanding borrow. See `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`. Full history in the
log.

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

Two campaigns, two seeds, **3.92 M operations, zero divergences**:

```
module=bit-set seed=42       cases=26373 ops=2625894 wall=120.0s divergences=0
module=bit-set seed=20260801 cases=12871 ops=1293656 wall=60.0s divergences=0
```

Reproduce with `target/release/difffuzz --module bit-set --seed 42 --cases 26373`.

The op alphabet covers `set`/`reset`/`flip`/`get`/`test`/`rank`/`select`/`clear` plus the cursor ops,
with `reset` weighted **up** rather than down, because B-17 only misfires on a bit that is already
clear, and a low weight would make that rare rather than routine. Observable state is `size`,
`length`, **`array`** and `toJSON()` — `array` is the point, since `size` alone would agree in
plenty of programs where the words had already diverged. `$iter` alternates between `values` and
`entries`, which share an implementation in this port and are separate closures upstream, so
fuzzing only one would leave the other unchecked. `clear()` is in the alphabet *because* it
interacts with an open cursor. Deliberately excluded: nothing — out-of-range indices,
negative-adjacent behaviour and cursor interleaving are all generated. Full grammar: evidence file.

**The fuzzer was falsified before it was trusted.** Sabotage: `reset` given the `>>> 0`
upstream forgot — B-17 *fixed*, which is the single most plausible thing a future cleanup does to
this file. Caught in 1,325 cases (2.0 s), shrunk from 200 ops to two, and the two-op repro shows that
**B-23 and B-17 compound**: the accepted-but-invisible `set(31)` past `length` is what puts bit 31
into the word, which is the precondition for `reset`'s signed comparison to misfire — neither defect
alone reaches the state, and no upstream test passes an index past `length` at all. Reverted; the
seed is committed with provenance in `crates/difffuzz/proptest-regressions/bit-set.txt`. Full repro:
evidence file.

**Falsification of the port (gate 6):** the assertion named first was
`length divisible by 32 iteration, issue #117.` — chosen because it is the only assertion in the
file that depends on the last-word width calculation, and because upstream issue #117 exists
precisely because that calculation was once wrong. The sabotage, `length % 32 || 32` reduced to
`length % 32`, is confirmed red at exactly that line (11 passing, 1 failing, `32 !== 64`); reverted,
confirmed green again (12 passing). Neither of this module's headline defects (B-17, B-18) could
have served as this sabotage — "fixing" either leaves the original suite green, since every `reset`
in the file clears a bit that is actually set and `select`'s own test never skips a word — which is
exactly the failure mode gate 6 exists to catch, and here there were two of them waiting.

`$forEach(method, rule, limit)` walks the instance with a callback that calls back into it. This
module's mutations are `set(a1)`, `reset(a1)`, `flip(a1)` and `clear()`, all uncapped — safe uncapped
because a `BitSet` cannot grow and the outer bound is `this.array.length`, read once. What the op
checks is the word snapshot: upstream lifts `byte = this.array[i]` out of the inner loop, so a
callback that writes to the word currently being walked is invisible for the rest of that word and
visible in the next one. What it does not reach: the napi bridge, where a re-entrant callback would
actually run, is outside the loop `difffuzz` compares; `tests/boundary/reentrancy.js` covers that
instead. One deliberate narrowing, mirrored on both sides: a selected callback argument that is
`undefined` skips the mutation, because feeding it back in reaches upstream's `NaN`-indexed swap,
which `usize` cannot express and the core does not model. Disclosed in `fuzz/log.txt`.

### Bench

`bench/results.json` → `modules["bit-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`reset`/`get`/`test` (50/25/25) over capacity 1e6, xorshift32
seed 42. `rank` is deliberately excluded: it has no rank/select index behind it on either side, so
a single call is O(i / 32) words, and a 25%-weighted mix at this domain made the harness spend ten
minutes on six of ten reps before it was killed — see `bench/runner/src/bit_set.rs` for the full
account. Full table: evidence file.

**The index-conversion path on the hot methods (`set`/`reset`/`flip`/`get`) was found and fixed, and
it was the whole of an earlier disclosed regression.** `split` in
`crates/mnemonist-core/src/structures/bits.rs` ran JavaScript's full `ToInt32` (`trunc`, then
`rem_euclid(2^32)`, then a sign fixup) on every call — needed only for indices outside `i32`'s range,
which upstream's out-of-range reads reach and which are reproduced bug-for-bug, but the identity for
any index already inside that range. `split` now tries `i32::try_from` first and falls back to the
float path only when the index really does not fit; the equivalence was checked over 2.6 million
values including every boundary (`i32::MIN`, `i32::MAX`, ±2^31, ±2^32). The port's p50 moved from
roughly 8.7 ns/op to roughly 5.9 ns/op against upstream's steady ~7.9 ns/op — **this module now reads
about 1.31× faster than upstream** where it previously read about 1.10–1.12× slower. `bit-vector`
shares `split` and moved with it, from a tie to about 1.30× faster. Full before/after figures and
the fix history: log.

**The wrapper layer's remaining cost was isolated by a bare counterfactual**
(`bench-runner --bit-set-probe`), comparing the real `BitSet` against a bare `Vec<u32>` with plain
`usize` indices and no `RefCell`, and a third variant in between that keeps the `f64`-based
`to_int32` conversion but drops the `RefCell`. The isolated gap between the wrapped and fully bare
variant splits roughly 73%/27% between the `RefCell`/`Words` wrapper layer and the `to_int32`
conversion alone — `to_int32`'s `f64::rem_euclid` is real floating-point division, not merely a
missed-inlining artefact, and is a meaningfully sized contributor on its own. Both bare variants beat
upstream's own published p50 outright. Full probe table: evidence file.

What is confirmed independently of any of this is the memory and startup side: the port uses roughly
a third of the RSS and starts thirty times faster, exactly where a `Vec<u32>` versus a full V8
process should differ.

A `usize` fast path guarded by "index is non-negative and small" — removing the remaining `RefCell`
and `to_int32` overhead entirely — has not been attempted: both layers are load-bearing rather than
incidental (the `Rc<RefCell<Vec<u32>>>` is `Words`'s own re-entrancy story, shared with `BitVector`;
the `f64`-based `to_int32` is what makes a negative index drop cleanly rather than wrap through
`usize`), and such a change would be a `crates/mnemonist-core` behaviour-preserving optimisation
needing bit-set's fuzz campaign and bench figures re-run before it could stand.

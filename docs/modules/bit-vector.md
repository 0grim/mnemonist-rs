# bit-vector

Upstream: `bit-vector.js` (550 LOC) + `utils/bitwise.js` (109 LOC, see
`docs/modules/utils-bitwise.md`) + `obliterator/iterator` · `test/bit-vector.js` — **320 lines,
21 `it` blocks, 96 assertion statements**.

Port: `crates/mnemonist-core/src/structures/bit_vector.rs` +
`crates/mnemonist-core/src/structures/bits.rs` + `crates/mnemonist-core/src/utils/bitwise.rs` +
`crates/mnemonist-core/src/cursor/mod.rs`.
Bridge: `crates/mnemonist-napi/src/bit_vector.rs`, `crates/mnemonist-napi/src/cursor.rs`.
Shim: `tests/bridge/bit-vector.js`.

The largest of this group of four modules, and the one with the best test:source ratio. It shares
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

`crates/mnemonist-core/src/structures/bit_vector.rs` — 19 tests, closing every gap above except 21
and 22: a 1:1 reproduction of all twenty-one upstream blocks as a baseline, the full pop/size
sequence upstream skipped the assertion on, both bounds-guard gaps, capacity-with-zero-length
iteration, both directions of `reallocate` at their edge cases, all three policy failure shapes,
the policy loop across seven applications, cursor detachment on `reallocate` and invisibility of
growth during iteration, and both inherited `bit-set` defects re-verified directly against
`BitVector` rather than inferred. Full test-to-gap mapping: evidence file.

Plus the 13 tests on the shared `bits.rs`, listed in `docs/modules/bit-set.md`.

**Still untested, stated rather than glossed:** gap 22 (`inspect`, not ported), gap 21 in its
`arguments.length` form, and a JS caller writing *through* `vector.array` (both in the divergence
table).

## Bugs this found

**B-21 — `pop` maintains neither `size` nor the bit, and `push(0)` clears nothing.**
Verified against Node 24.18.1. Three defects in six lines:

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
Verified against Node 24.18.1. The `|| 32` exists for a length that fills its last word
exactly, and `0 % 32` is also falsy. `BitSet` cannot reach it — its array is empty when its length
is — but `BitVector` can, because capacity outlives length: `new BitVector(); v.grow();` then
`forEach` calls back 32 times on a vector of length 0.

**And the same `length < index` off-by-one as `HashedArrayTree`** (B-16's shape, in a different
file): `set(length, v)` writes into the capacity region without moving `length`. Measured:
`new BitVector(5); set(5, 1)` gives `size === 1`, `get(5) === 1`, `test(5) === true`, `rank(5) === 0`
and iteration over five bits. Two of this group's four modules have the identical guard bug, written
independently.

**Inherited verbatim from the copy-paste:** B-17 (`reset`'s missing `>>> 0` driving `size` negative)
and B-18 (`select` losing 32 positions per skipped word). Both re-measured against `BitVector` on
Node. See `docs/modules/bit-set.md` for the analysis and `docs/modules/utils-bitwise.md` for B-19
and B-20, also in this unit's require-closure.

**The bridge held a bare core value behind `&self`**, which LLVM was entitled to compile as a
`noalias readonly` pointer and hoist reads across a re-entrant JS callback (B-31). It now holds
`RefCell<Core>`, which is not `Freeze`, and every `&mut self` method borrows via `borrow_mut()`
taken per step and released before the callback runs. Full history in the log.

**One place the fix cannot be applied cleanly.** The rule the `RefCell` imposes is that no borrow
may be alive across a call that can run JavaScript. Everywhere else in the bridge that is
achievable: `forEach` re-borrows per step, `DefaultMap::get` runs its factory between the read and
the write. Here it is not, because **the growth policy is JavaScript that `mnemonist-core` calls
from inside `grow`** — so `push`, `set`, `grow`, `resize`, `reallocate` and `apply_policy` hold the
vector while a JS function runs. A `RefCell` panic inside a `#[napi]` method aborts the process:
napi 3.12 does not `catch_unwind` a sync call, and a panic unwinding out of an `extern "C"` frame is
an abort. Every borrow in this bridge is therefore fallible and raises a named error instead of
risking that abort — see `REENTRANT_POLICY`, `docs/DECISIONS.md`'s "Re-entrancy" section, and the
two `BitVector` policy specs
in `tests/boundary/reentrancy.js`. Upstream would serve a re-entrant call from a half-grown vector;
this port refuses it instead. That is a stated narrowing, and it replaces an abort, which replaced
undefined behaviour.

## Deliberate divergences

Everything in `docs/modules/bit-set.md`'s table applies — the shared store, the word-caching cursor,
the signed `size`, the `i64` indices, `array` exposed as a copy, the strict `value === 0` test,
`select`'s `Either`, the `forEach` scope caveat, and `inspect` not being ported. Additionally:

| # | Divergence | Why |
|---|---|---|
| — | **The growth policy is `Box<dyn Fn(f64) -> Option<f64>>`.** | `None` is upstream's `typeof newCapacity !== 'number'`, which a JS policy really can produce and which `applyPolicy` explicitly checks for. `f64` in and out because a policy result of `40.5` is accepted upstream and rounded to a word. |
| — | **A throwing JS policy is re-raised by the bridge, not by the core.** | The core's `Option` has nowhere to put an exception, so `JsPolicy` parks it in a `RefCell` and the calling method prefers it over the core's classification. Without that, a throwing policy would surface as "policy returned an invalid value" — a different error from a different place. |
| — | **A policy returning `NaN` or `Infinity` is refused.** | Upstream's guard is `typeof !== 'number' \|\| < 0`, and `NaN` passes both because every `NaN` comparison is false. It then flows into `Math.ceil(NaN / 32) * 32` and `new Uint32Array(NaN)`. There is no honest Rust reproduction of an allocation of `NaN` elements, so it raises instead — the same call made for `StaticDisjointSet`'s out-of-range reads. |
| — | **`BitVector` is not `Clone`-equivalent across policies.** | `Box<dyn Fn>` cannot be cloned, so `Clone` copies the bits and the capacity and resets the policy to the default. Nothing upstream clones a vector, and silently sharing a policy would be worse than a documented reset. |
| — | **The constructor's `initialLength \|\| initialCapacity` union is resolved in the bridge.** | Upstream reads `initialLength \|\| initialCapacity \|\| 0`, so `{initialCapacity: 30}` sets the **length**. The core takes a length and nothing else; the quirk is reproduced at the boundary where the JS object exists, and pinned by a test on the arithmetic that follows from it. |

## Fuzz + bench

### Fuzz

Two campaigns, two seeds, **3.23 M operations, zero divergences**:

```
module=bit-vector seed=42       cases=21852 ops=2250634 wall=120.0s divergences=0
module=bit-vector seed=20260801 cases=9617  ops=981385  wall=60.0s  divergences=0
```

Reproduce with `target/release/difffuzz --module bit-vector --seed 42 --cases 21852`.

The op alphabet covers `set`/`reset`/`flip`/`get`/`test`/`rank`/`select`/`clear` plus `push(1)`,
`push(0)`, `pop()` (kept as separate ops, since only the former touches `size` and only the latter
leaves a stale bit, and B-21 needs both interleaved), `resize`/`reallocate`/`grow` and the cursor
ops. Observable state is `size`, `length`, `capacity`, **`array`** and `toJSON()`. `set` is the only
op in this grammar that throws, and its message is compared in full through the `{"$throw": …}`
encoding added for `hashed-array-tree`. **Deliberately excluded: custom growth policies** — upstream's
policy is a JS function and a generated program is JSON, so the default (strictly increasing) policy
is the only one fuzzed, and both throws in `applyPolicy` are structurally unreachable from this
grammar; they are covered by native tests instead. Full grammar: evidence file.

**The fuzzer was falsified before it was trusted.** Sabotage: `pop` made to clear the bit it
returns and to decrement `size` — which is what `pop` is supposed to do, and the single most
plausible repair anyone would make to this module. Caught in 1,075 cases (1.0 s), shrunk from 200
ops to two — and upstream's own `pop` test performs that exact pair, then asserts only the returned
value and `length`. Reverted; the seed is committed with provenance in
`crates/difffuzz/proptest-regressions/bit-vector.txt`. Full repro: evidence file.

**Falsification of the port (gate 6):** the assertion named first was
`should throw if the policy returns an irrelevant size.` — chosen because the policy machinery is
the best-covered part of the upstream file, so a sabotage there has a real assertion to break. The
sabotage, `applyPolicy`'s `newCapacity <= this.capacity` weakened to `<` (accepting a policy that
returns exactly the current capacity), is confirmed red at exactly the named line (20 passing, 1
failing, "Missing expected exception"); reverted, confirmed green again (21 passing). Neither half
of B-21 could have served as this sabotage: "fixing" `push(0)` to clear its slot leaves the suite
green, because every slot the push test writes over is already zero, and "fixing" `pop` to decrement
`size` leaves it green too, because no assertion in the file reads `size` after a `pop`. Across this
group of four modules, five plausible-looking sabotages were rejected on that ground before a usable
one was found. Full record: evidence file.

`$forEach(method, rule, limit)` walks the instance with a callback that calls back into it. This
module's mutations are `set(a1)`, `reset(a1)`, `flip(a1)`, `pop()` (uncapped) and `push()` (capped
at four). `push`'s cap **is** tuning, stated as such: the outer bound is captured so an uncapped
push still terminates, but a push per bit over a 400-bit vector is hundreds of reallocations per
case and the throughput buys more programs than the depth does. `set` and `push` can throw from the
growth policy; the throw is reported alongside the steps already taken rather than instead of them,
so the two sides never agree on less than they know. What it does not reach: the napi bridge, where
a re-entrant callback would actually run, is outside the loop `difffuzz` compares;
`tests/boundary/reentrancy.js` covers that instead. One deliberate narrowing, mirrored on both sides:
a selected callback argument that is `undefined` skips the mutation, because feeding it back in
reaches upstream's `NaN`-indexed swap, which `usize` cannot express and the core does not model.
Disclosed in `fuzz/log.txt`.

### Bench

`bench/results.json` → `modules["bit-vector"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`get`/`pop` (50/25/25), `vector`/`hashed-array-tree`'s shape
(this module grows under `push`, unlike `bit-set`'s fixed domain, so there is no capacity parameter
to set): the port is about 1.30× faster at p50 (6.42 vs 8.3 ns/op), 1.1× faster at p99, roughly tied
at min. No regressions. Full table: evidence file. `rank`/`select` are excluded for the reason
recorded in `bit-set`'s own bench doc: neither has an index behind it, so a single call is O(i / 32)
words, and a uniform-weighted mix would put a domain-scaling cost next to three genuinely O(1) ops.

This module shares `split` with `bit-set`, whose `ToInt32` fast path is described in that unit's
document, and moved with it from a tie (8.20 ns) to 6.42 ns — see the log for that fix's history.
That also answers a question this document had previously left open: the shared store is an
`Rc<RefCell<Vec<u32>>>`, and every `set`/`reset`/`flip` takes a borrow upstream does not pay for; on
operations that are otherwise a load, an OR and a store, that was not obviously negligible. The
borrow is still there and the module is now faster than upstream, so whatever the borrow costs, it
was not what stood between this port and a win — the index conversion was. The `RefCell` bought
exact reproduction of `clear`/`reallocate` detaching an open cursor, and it is still paying for
itself.

**No regressions, but the narrowest margin of the eleven mixed workloads in this group** — p50 and
min are effectively ties (within 3%, well inside the noise band methodology.md documents: up to
~32% p99 swings between clean runs on this host). A probe at 4e6 domain confirmed the same picture
rather than revealing a boundary: port still ahead on p50/p99/min at that scale, by a similar small
margin, with the *sign* of the gap flipping between individual passes at both sizes — this is noise,
not a trend. `push`/`pop`/`get` here are all single-word bit operations once the vector is
allocated, the same shape `bit-set`'s zero-overhead bit ops have, which is plausibly why this is the
one growable module in the batch that comes closest to parity rather than winning decisively like
`vector`/`hashed-array-tree` do. This attribution is unconfirmed: not isolated by profiling, offered
as the mechanism most consistent with the numbers.

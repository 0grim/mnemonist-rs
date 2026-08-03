# fixed-reverse-heap

Upstream: `fixed-reverse-heap.js` (209 LOC) + `heap.js` (576 LOC) + `utils/comparators.js`
(79 LOC) · **864 LOC in the closure** · `test/fixed-reverse-heap.js` — **123 lines, 7 `it` blocks,
17 assertion statements**.

Port: `crates/mnemonist-core/src/structures/fixed_reverse_heap.rs` (plus
`crates/mnemonist-core/src/structures/heap.rs` and `.../utils/comparators.rs`).
Bridge: `crates/mnemonist-napi/src/fixed_reverse_heap.rs`, `.../js_array.rs`, `.../comparators.rs`.
Shim: `tests/bridge/fixed-reverse-heap.js`.

**Is this the same unit as `heap`? No — it is a separate unit whose closure is a superset.**
`test/fixed-reverse-heap.js` requires only `../fixed-reverse-heap.js`, which requires `./heap.js`
and `./utils/comparators.js`. Two test files means two units; what they share
is that this one cannot land until `heap` has. The practical consequence is straightforward:
`multi-set` needs `fixed-reverse-heap`, and `fixed-reverse-heap` needs `heap`, so the
655 LOC of the `heap` unit unblocks 209 here and 361 test lines later.

A bounded "keep the best *k*" heap. It is *reverse* because it stores its elements under
`reverseComparator(comparator)`, which puts the **worst** survivor at the root, so evicting it when
a better element arrives is one `replace` rather than a scan.

---

## What upstream tests

Seven `it` blocks, all with total, side-effect-free comparators:

* **Two `ArrayClass`es**: `Array` in three blocks, `Uint8Array` in four. Five of the seven
  assertions comparing contents compare against a `Uint8Array`, and two assert `instanceof`.
* **One capacity: 3.** Every block.
* **One custom comparator**, the reversing one in the last block.
* **`consume` and `toArray` are both covered**, including that `toArray` leaves `size` alone.
* **`clear()` is called once**, followed only by more `push`es.
* **`push` is called at most nine times** in a block, so the eviction path runs — that is the best
  covered part of the file.

## What upstream does NOT test

**The constructor**

1. **Only capacity 3 is ever constructed.** Never `0` — which is *accepted*, because the guard is
   `&&` where `||` was meant, and then discards every push in silence (B-73). Never `1`, where the
   heap is a single slot and `consume`'s `i !== 0` branch never runs.
2. **A negative or non-numeric capacity is never passed.** `new FixedReverseHeap(Array, -1)` dies
   in `new Array(-1)` with `Array`'s own `RangeError`, because `this.items = new ArrayClass(capacity)`
   runs **before** either guard. The only input that reaches mnemonist's own message is a
   non-number that coerces to `<= 0`.
3. **A non-function comparator is never passed**, so the second guard is unexercised.
4. **`this.ArrayClass` and `this.items` are never read.** Both are public properties.

**`clear()`**

5. **`peek()` after `clear()` is never called.** It answers with an item that is no longer in the
   heap, because `clear` resets `size` and does not touch `items` (B-74). `consume` and `toArray`
   both slice to `size`, so they agree the item is gone — which is precisely why the bug is latent
   and why an observation of `items` is what makes it visible.
6. **`clear()` on an empty heap** is never called.

**`peek()`**

7. **`peek()` is never called at all**, in any of the seven blocks. Its whole semantics — that it
   returns the **worst** kept item rather than the best, which is the opposite of `Heap#peek` — is
   unasserted.

**The comparator as a callback**

8. **A comparator that mutates the heap** is never used. Its `push` writes `this.items[this.size]`
   and then sifts, so a comparator that appends to `items` grows the array past `capacity` and
   nothing notices.
9. **A comparator that throws** is never used.
10. **The two-argument constructor form and the three-argument form are both used**, but never with
    a comparator that would distinguish them from the default — the reversing comparator in the last
    block is the only custom one, and it is not compared against the default on the same input.

**Typed-array store semantics**

11. **No value outside `0..255` is ever pushed into the `Uint8Array` heaps.** `push(300)` stores
    `44` and `push(-1)` stores `255`, and the eviction comparison then runs against the *narrowed*
    value, not the one that was passed.
12. **A non-integral value** is never pushed.

**`consume` / `toArray`**

13. **`consume()` on an empty heap** is never called.
14. **`toArray()` twice in a row** is never done — it is non-destructive, and nothing pins that
    beyond one call.
15. **A heap below capacity** is consumed once (block 2); a heap that has never been full is never
    `toArray`ed.

**Never called at all**

16. `inspect()` and the `nodejs.util.inspect.custom` symbol.

## What we test in addition

`crates/mnemonist-core/src/structures/fixed_reverse_heap.rs` — 8 tests, closing gaps 1, 5, 7, 8 and
15 in addition to reproducing the upstream blocks as a baseline: a below-capacity `consume` returns
only the live prefix, `peek` after `clear` returns the stale value (pinned so a future "tidy-up" of
`clear` fails here), `consume` after `clear` ignores the stale contents, a zero capacity silently
accepting nothing, and a re-entrant comparator that pushes, asserting the array grows past
`capacity` while `size` does not. Full test-to-gap mapping: evidence file.

**JavaScript boundary spec** — `tests/boundary/heap.js`, seven `FixedReverseHeap` assertions,
closing gaps 1, 2, 3, 5, 7, 8 and 11. Every expectation in it was run against the pinned upstream
source first and is what upstream printed, so it measures divergence in either direction.

Gap 11 in particular can only be checked here: the narrowing is a JavaScript typed-array store
semantic, and the port gets it by writing through a real `Uint8Array` (D-73) rather than by
implementing it.

**Differential fuzzer** — see below.

**Still untested, stated rather than glossed:** gap 16 (`inspect`, not ported), gap 12 (a
non-integral push, which narrows in a typed array and is stored verbatim in an `Array` — covered by
neither), and gap 11 *in the fuzzer*, for the reason in the Fuzz section.

## Bugs this found

Two of the ten in the `B-70`–`B-79` block are this module's; both are verified against
Node 24.18.1 and pinned by `tests/boundary/heap.js`.

**B-73 — the capacity guard is `&&` where `||` was meant.**

```js
if (typeof capacity !== 'number' && capacity <= 0)
  throw new Error('mnemonist/FixedReverseHeap.constructor: capacity should be a number > 0.');
```

For any number the first half is false, so the `&&` short-circuits and the guard **cannot fire for
the very inputs it names**. `new FixedReverseHeap(Array, 0)` is accepted; `push` then returns `0`,
`size` stays `0` and `consume()` is `[]` — every element silently discarded. The only way to reach
the throw is a non-number that coerces to `<= 0`, e.g. `null`. And a negative capacity never gets
there either, because `new ArrayClass(capacity)` runs first and raises `Array`'s own `RangeError`.

**B-74 — `clear()` leaves `items`, so `peek()` answers a discarded item.**

```js
FixedReverseHeap.prototype.clear = function () { this.size = 0; };
FixedReverseHeap.prototype.peek  = function () { return this.items[0]; };
```

`peek` reads the array directly and never consults `size`:

```js
var heap = new FixedReverseHeap(Array, 3);
heap.push(45); heap.push(12); heap.push(46);
heap.clear();
heap.size      // 0
heap.peek()    // 46   ← no longer in the heap
heap.consume() // []   ← and consume agrees it is gone
```

Upstream's own test calls `clear()` and then only ever `push`es again, so it never looks. This is
also the module's fuzzer falsification, below.

**The port's own defects on this module: none.** The two found while landing this unit were both in
`heap`'s bridge (napi's one-name-table conflict, and a detached `#[napi(factory)]`); see
`docs/modules/heap.md`.

## Deliberate divergences

Everything in `docs/modules/heap.md`'s table applies, since this module is built on those
algorithms. Specific to this one:

| # | Divergence | Why |
|---|---|---|
| — | **`ArrayClass` stays a real JavaScript constructor.** | Unlike `hashed-array-tree`, which maps the class onto a `PointerWidth`, this bridge allocates through the constructor it was handed. It has to: the original suite asserts `consume() instanceof Uint8Array` *and* uses plain `Array`, which has no width. Allocating through it also gives the `ToUint32`-then-narrow store semantics for free and exactly (D-73). |
| — | **The constructor's statement order is reproduced.** | `new ArrayClass(capacity)` runs before both guards, so a capacity the class refuses raises the class's error rather than mnemonist's. Reordering would have been tidier and would have changed which error a caller sees. |
| — | **The dead capacity guard is reproduced, `typeof` half included.** | B-73. A port that "fixed" the `&&` would reject a capacity upstream accepts, which is a behaviour change in the direction of correctness — and therefore wrong. |
| — | **`clear()` does not touch `items`.** | B-74, same reasoning; and it is the module's fuzzer sabotage precisely because it is the most plausible unrequested improvement. |
| — | **`arguments.length === 2` is read as "a third argument was supplied".** | napi's typed signature cannot see arity. The two forms upstream's suite uses are exact; the difference is an explicit `new FixedReverseHeap(Array, cmp, undefined)`. |
| — | **`this.comparator` and `this.ArrayClass` are not exposed.** | Same call as `heap`'s D-77 for the comparator; `ArrayClass` follows it, since it is only meaningful alongside one. `items` **is** exposed, as the live array. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion. |

## Fuzz + bench

### Fuzz

Four campaigns, three seeds, **6.30 M operations, zero divergences**:

```
module=fixed-reverse-heap seed=42       cases=25906 ops=2551774 wall=120.0s divergences=0
module=fixed-reverse-heap seed=20260801 cases=11913 ops=1158167 wall=60.0s divergences=0
module=fixed-reverse-heap seed=31337    cases=13442 ops=1320693 wall=60.0s divergences=0
module=fixed-reverse-heap seed=42       cases=12879 ops=1273022 wall=60.0s divergences=0  (post-fix)
```

The first three ran against the pre-review build; see `docs/modules/heap.md` for why they stand and
were re-run anyway. Reproduce with `target/release/difffuzz --module fixed-reverse-heap --seed 42
--cases 25906`.

The op alphabet covers `push`/`peek`/`clear`/`toArray`/`consume`. The constructor alphabet draws
`ArrayClass` (see the exclusion below), one of five comparator factories (reusing `heap`'s set minus
`clearer`, since this structure's `clear` sets `size` without rebinding `items` — the rebinding case
`clearer` exists for does not apply here; `clear` is an ordinary op in the alphabet instead, which
reaches B-74 directly), and a generated capacity in `0..5` with zero included, because a capacity of
`0` is accepted upstream (B-73) and a grammar that only generated sensible capacities would never
have visited that branch. Both constructor arities are generated — 30% of programs omit the
comparator, upstream's `arguments.length === 2` form. Observable state is `size`, `capacity` and
`items`; `items` is `capacity` slots long from construction and keeps its contents through a
`clear()`, which is what makes B-74 visible in the state rather than only through a `peek`. Full
grammar: evidence file.

**Deliberately excluded: typed-array `ArrayClass`es** (gap 11). Upstream's `ArrayClass` may be any
typed array, and the element narrowing that comes with one is a JavaScript store semantic that
`mnemonist-core`'s `VecStore` does not have and is not supposed to have — the port gets it by
writing through a real typed array in the *bridge* (D-73). Fuzzing it would compare core against a
behaviour core deliberately delegates, so every program would diverge for a reason that is not a
defect. It is covered instead by `test/fixed-reverse-heap.js`, which uses `Uint8Array` in four of
its seven blocks, and by `tests/boundary/heap.js`, which asserts `push(300) → 44` through the real
bridge. **This is a real gap in the fuzz coverage and is stated as one.**

**The fuzzer was falsified.** Sabotage: `FixedReverseHeap::clear` blanking the backing array as
well as resetting `size` — that is, *fixing* B-74, the most plausible "obvious improvement" anyone
would make to this file, and one that makes the port strictly more correct than upstream and
therefore wrong. All 7 assertions of `test/fixed-reverse-heap.js` still passed under it; the fuzzer
caught it in 84 operations, shrinking it to a `clear()` on a capacity-1 heap, and
`tests/boundary/heap.js` caught it too, by name. Reverted; the seed is committed with a provenance
header in `crates/difffuzz/proptest-regressions/fixed-reverse-heap.txt`. Full repro: evidence file.

**Falsification of the port (gate 6):** the assertion named first was `should be possible to
consume the heap.` — `assert.deepStrictEqual(heap.consume(), [1, 4, 8])` at
`test/fixed-reverse-heap.js:31` — chosen because `consume` is the only algorithm this module owns
outright, since `push` delegates to `Heap.siftDown`/`Heap.replace`. The sabotage, `consume`'s
backwards fill written with one index flipped, is confirmed red in the named place (2 passing, 5
failing, `[8, 4, 1]` against `[1, 4, 8]`); reverted, confirmed green again (7 passing). Full record:
evidence file.

### Bench

`bench/results.json` → `modules["fixed-reverse-heap"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`peek` (75/25), default numeric comparator, capacity `size / 2`
rather than a tiny slice of the op count (`fixed-stack`/`fixed-deque`'s own convention): a capacity
that is a tiny fraction of the value domain fills once and then rarely displaces anything again
(a fresh draw only beats the current worst with probability `capacity / domain`), while half the
domain keeps a fresh uniform draw's odds of beating the worst near 50% throughout the run. Measured
directly with a standalone probe (comparing `peek()` before/after every push once the heap was
already full): **30,074 displacements over 49,843 full-heap pushes — a 60.3% rate** — confirming
the sift-down `replace` path is the common case here, not a rare one: the port is 1.04× faster at
p50 (14.50 vs 15.07 ns/op), essentially tied at min, and about 1.20× slower at p99 (115.94 vs
96.99). Full table: evidence file.

**A real, reproducible p99 loss: ~1.2× across two independent runs**, re-run specifically because a
loss this small invites checking it is not a one-off — it held both times, with `min_ns_per_op`
essentially tied (the two differ by under 0.2%, which the driver's regression check still flags
mechanically since the rule is `port > original`, not a tolerance band). **Cause: unconfirmed.**
Both sides do the identical sift-down comparison count per displacement at this workload's 60%
displacement rate; no profiling was done to isolate why the port's tail is worse specifically at
p99 rather than uniformly, so no mechanism is asserted. p50 and every RSS/startup figure favour the
port. Checksum `234148030045`, identical on both sides.

The natural workload for this structure is "keep the *k* smallest of *n*", and a representative
benchmark needs to be run both at a *k* small relative to *n* (the eviction path) **and** at a *k*
comparable to *n* (the fill path), since they are different code — a run at `k === n` measures
`Heap::siftDown` and nothing this module owns.

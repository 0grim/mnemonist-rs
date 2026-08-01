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
and `./utils/comparators.js`. Two test files means two units under DESIGN.md §1.1; what they share
is that this one cannot land until `heap` has. The practical consequence is the one the roadmap
cares about: `multi-set` needs `fixed-reverse-heap`, and `fixed-reverse-heap` needs `heap`, so the
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

**Rust native tests** — `crates/mnemonist-core/src/structures/fixed_reverse_heap.rs`, 8:

| Test | Closes gap |
|---|---|
| `keeps_only_the_smallest_items`, `to_array_leaves_the_heap_intact`, `a_reverse_comparator_keeps_the_largest_items` | the upstream blocks, as a baseline |
| `consume_below_capacity_returns_only_the_live_prefix` | 15 |
| `peek_after_clear_answers_a_discarded_item` | 5, 7 — asserts the stale value *is* returned, so a future "tidy-up" of `clear` fails here |
| `consume_after_clear_ignores_the_stale_contents` | 5 — the other half, and why the bug is latent |
| `a_capacity_of_zero_silently_accepts_nothing` | 1 |
| `a_comparator_may_re_enter_and_push` | 8 — and the assertion is that the array grows past `capacity` while `size` does not |

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
Node 24.18.1 and pinned by `tests/boundary/heap.js`. Full write-ups in `planning/NOTES.md`.

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

```
module=fixed-reverse-heap seed=42       cases=25906 ops=2551774 wall=120.0s divergences=0
module=fixed-reverse-heap seed=20260801 cases=11913 ops=1158167 wall=60.0s divergences=0
module=fixed-reverse-heap seed=31337    cases=13442 ops=1320693 wall=60.0s divergences=0
```

Three campaigns, three seeds, **5.03 M operations, zero divergences**. Reproduce with
`target/release/difffuzz --module fixed-reverse-heap --seed 42 --cases 25906`.

* **Op alphabet:** `push(v)` (weight 7) · `peek()` (2) · `clear()` (2) · `toArray()` (2) ·
  `consume()` (1).
* **Constructor alphabet:** `ArrayClass` (see the exclusion below), one of five comparator
  factories, and a **generated capacity in `0..5`, zero included** — because a capacity of `0` is
  accepted upstream (B-73) and a grammar that only generated sensible capacities would never have
  visited that branch.
* **The comparator factories are `heap`'s**, minus `clearer`: this structure's `clear` sets `size`
  and does not rebind `items`, so it is not the rebinding case `heap` uses that factory for.
  `clear` is an ordinary op in the alphabet instead, which reaches B-74 directly.
* **Both constructor arities are generated** — 30% of programs omit the comparator, which is
  upstream's `arguments.length === 2` form.
* **Observable state, compared after every op:** `size`, `capacity` and `items`. `items` is
  `capacity` slots long from construction and keeps its contents through a `clear()`, which is what
  makes B-74 visible in the state rather than only through a `peek`.

**Deliberately excluded: typed-array `ArrayClass`es** (gap 11). Upstream's `ArrayClass` may be any
typed array, and the element narrowing that comes with one is a JavaScript store semantic that
`mnemonist-core`'s `VecStore` does not have and is not supposed to have — the port gets it by
writing through a real typed array in the *bridge* (D-73). Fuzzing it would compare core against a
behaviour core deliberately delegates, so every program would diverge for a reason that is not a
defect. It is covered instead by `test/fixed-reverse-heap.js`, which uses `Uint8Array` in four of
its seven blocks, and by `tests/boundary/heap.js`, which asserts `push(300) → 44` through the real
bridge. **This is a real gap in the fuzz coverage and is stated as one.**

**Falsification of the fuzzer.** Sabotage: `FixedReverseHeap::clear` blanking the backing array as
well as resetting `size` — that is, *fixing* B-74. Chosen because it is the most plausible
"obvious improvement" anyone would make to this file, and because it makes the port strictly more
correct than upstream and therefore wrong.

* **All 7 assertions of `test/fixed-reverse-heap.js` still passed under it.**
* The fuzzer found it in **84 operations** and shrank it to a `clear()` on a capacity-1 heap:

  ```js
  var s = new FixedReverseHeap(Array, ascending, 1);
  s.toArray(); s.toArray();
  s.clear();          // port items [], upstream items [undefined]
  ```

* `tests/boundary/heap.js` caught it too, by name.

Reverted; the seed is committed with a provenance header in
`crates/difffuzz/proptest-regressions/fixed-reverse-heap.txt`.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:** `should be possible to consume the
heap.` — `assert.deepStrictEqual(heap.consume(), [1, 4, 8])` at
`test/fixed-reverse-heap.js:31`. Chosen because `consume` is the only algorithm this module owns
outright: `push` delegates to `Heap.siftDown`/`Heap.replace`, so a sabotage there would have proved
`heap`'s code was running rather than this module's.

**The sabotage:** `consume`'s backwards fill, `array.set(i, last_item)` → `array.set(l - 1 - i, …)`.
One index, in the loop that is the entire reason this structure stores its elements reversed.

**Confirmed red**, and red in the named place: `2 passing, 5 failing`, the named assertion failing
with `[8, 4, 1]` against `[1, 4, 8]`. Reverted; **confirmed green again**: `7 passing`.

### Bench

**Not run.** Gate 10 is deferred to a serial pass on an idle machine (DESIGN.md §7.3); three other
agents were working while this unit landed. `fixed-reverse-heap` is therefore **complete except
gate 10** and is deliberately *not* in `tests/scope.txt`, which `tests/verify.sh` will report — the
intended state, not an oversight.

A note for whoever runs it: the natural workload is "keep the *k* smallest of *n*", which is what
the structure exists for, and it should be run at a *k* small relative to *n* (the eviction path)
**and** at a *k* comparable to *n* (the fill path), because they are different code. A run at
`k === n` measures `Heap.siftDown` and nothing this module owns.

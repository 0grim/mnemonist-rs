# queue

Upstream: `queue.js` (215 LOC) + `obliterator/foreach.js` (70 LOC) +
`obliterator/iterator.js` (95 LOC) · `test/queue.js` — **126 lines, 11 `it` blocks, 22 assertion
statements**.

Port: `crates/mnemonist-core/src/structures/queue.rs`, `crates/mnemonist-core/src/cursor/mod.rs`.
Bridge: `crates/mnemonist-napi/src/queue.rs`, `foreach.rs`, `js_slot.rs`, `statics.rs`, `cursor.rs`.

`queue.js` is `stack.js`'s structural twin — same shape, same `forEach` boundary, same cursor
machinery — and was ported straight after it for that reason. It has exactly one interesting
difference and it is a single line. **`Queue.prototype.values` re-reads `items.length` on every
step where `Stack.prototype.values` freezes it**, four files away, in otherwise identical code. A
port that modelled "one cursor shape for all modules" gets this wrong and no upstream test notices.

The `forEach` dispatch is shared with `stack` and documented there
(`docs/modules/stack.md` → "The `forEach` dispatch"); it is not repeated here.

---

## What upstream tests

Eleven `it` blocks, mirroring `test/stack.js` almost line for line:

```js
queue.enqueue('test');                     assert.strictEqual(queue.size, 1);
queue.clear();                             assert.deepStrictEqual(queue.toArray(), []);
assert.strictEqual(queue.peek(), undefined);
assert.strictEqual(queue.dequeue(), 1);    // …down to undefined
queue.forEach(function (item, i, l) { assert.strictEqual(item, i + 1); assert.strictEqual(queue, l); });
assert.deepStrictEqual(Queue.from([1, 2, 3]).toArray(), [1, 2, 3]);
assert.deepStrictEqual(Queue.of(1, 2, 3).toArray(), [1, 2, 3]);
var iterator = queue.values();             assert.strictEqual(iterator.next().value, 1);
var iterator = queue.entries();            assert.deepStrictEqual(iterator.next().value, [0, 1]);
for (var item of queue) assert.strictEqual(item, ++i);
```

Characterising the shape of that coverage:

* **Every queue holds at most three elements**, all small integers or the string `'test'`.
* **`offset` is never read.** Neither is `items`. Both are public properties upstream, and
  together they are the *entire* mechanism that makes a `Queue` different from an array with
  `shift()`.
* **The compaction is never observed.** The dequeue block drains a three-element queue, which
  compacts twice on the way — invisibly, because `toArray()` reports the same thing either way.
* **`from` is called with an array literal and with `arguments`.** Both reach branch 1.
* **Iteration is always immediate**, with no mutation between creating a cursor and draining it.

## What upstream does NOT test

**The data structure's only algorithm**

1. **The compaction schedule is entirely untested.** `++this.offset * 2 >= this.items.length`
   rebuilds the array; nothing reads `offset` or `items.length`, so the whole schedule — and the
   `O(1)` amortisation it buys — is unobserved. A port that compacted on every dequeue, or never,
   would pass.
2. **The compaction **rebinds** `this.items`.** A cursor opened beforehand keeps the old array and
   goes on yielding elements the queue has already dequeued. Never done.
3. **`dequeue` does not remove anything.** It reads `items[offset]` and advances; the element stays
   until a compaction drops it, and a cursor with an older frozen offset can still reach it. Never
   observed.
4. **`clear()` while a cursor is open** — same rebinding, same invisibility to the cursor. Never
   done.

**The live cursor end — the one line that differs from `Stack`**

5. **An `enqueue` during iteration is never performed.** The length is re-read every step, so it
   **is** visible — the opposite of `Stack`, where the frozen `l` hides it.
6. **A cursor that has reported `{done: true}` is never stepped again.** obliterator's `Iterator`
   has no done flag; it just re-runs its closure. So a queue that grows afterwards **resumes** the
   walk. Two lines of JS reach this and nothing upstream does.

**`forEach`**

7. **A `forEach` callback never mutates.** Upstream freezes the starting index and the bound but
   re-reads `this.items` each iteration, so a callback that dequeues far enough to compact sends the
   remaining reads into the *new* array under the old absolute index.
8. **`forEach` after a dequeue is never called**, so the `i = this.offset, j = 0` split — value
   index versus callback index — is untested.
9. **`scope` is never passed.**

**Return values and conveniences**

10. **`enqueue`'s return value is never used.**
11. **`toString`, `toJSON` and `inspect` are never called.**

**`from`, i.e. the dispatch**

12. **Four of the five branches are never reached.** Same as `stack`; see that document.

**Values**

13. **Only numbers and one string are ever stored.** Object identity, `undefined`, `NaN`, `-0`,
    lone surrogates and BigInts are untested.

## What we test in addition

`crates/mnemonist-core/src/structures/queue.rs` — 15 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all eleven upstream blocks, as a baseline |
| `enqueue_returns_the_new_size` | 10 |
| `the_compaction_fires_when_the_dead_prefix_reaches_half_the_array` | 1 — pinned index by index, `offset` and `items.length` after every dequeue |
| `a_one_element_queue_compacts_immediately` | 1 — the degenerate end (`1 * 2 >= 1`) |
| `interleaved_enqueue_and_dequeue_stay_in_order` | 1 — FIFO across compactions, which is the way the offset arithmetic actually breaks |
| `dequeueing_an_empty_queue_moves_nothing` | — |
| `cursors_do_not_restart_but_the_queue_can_be_walked_again` | D-06/D-07 |
| `an_enqueue_during_iteration_is_visible_to_the_cursor` | 5 — the live end |
| `a_finished_cursor_resumes_when_the_queue_grows` | 6 — nothing latches |
| `a_compaction_detaches_an_open_cursor_onto_the_old_array` | 2, 3 |
| `a_cursor_freezes_the_offset_it_was_opened_with` | 2, 3 |
| `clear_leaves_an_open_cursor_walking_the_old_array` | 4 |
| `for_each_reads_the_live_array_where_the_cursor_reads_the_capture` | 7 |
| `an_empty_queue_iterates_zero_times` | — |
| `from_iter_accepts_any_iterator` | D-03 |

`crates/mnemonist-core/src/cursor/mod.rs` — the three `Sequence::limit` tests added for this
module, against a synthetic source rather than a `Queue`, so the semantics are pinned once for
every future module with a live end.

`tests/boundary/stack-queue.js` — 37 specs shared with `stack`, each asserted **both**
differentially against `bench/upstream/queue.js` **and** explicitly. Gaps 7, 8, 11, 12 and 13 are
closed there, because they need JavaScript: mutating from inside a callback is a compile error in
Rust.

**Still untested, stated rather than glossed:** `inspect()` (gap 11, not ported), and `forEach`'s
`scope` in its `arguments.length` form (see the divergence table).

## Bugs this found

**None in `queue.js` itself.** Its arithmetic is correct, including the compaction, and reading it
statement by statement against Node turned up nothing.

**What it did surface is the divergence between the two `values()` implementations**, which is
recorded as B-6's mirror image rather than as a bug: `Stack` freezes `items.length` and `Queue`
does not, in code that is otherwise identical. Neither is wrong; they are simply different, and the
difference is observable in three lines:

```js
var q = new Queue(); var it = q.values(); q.enqueue(0); it.next();   // {value: 0, done: false}
var s = new Stack(); var jt = s.values(); s.push(0);    jt.next();   // {done: true}
```

Not filed upstream: there is no defect, only an inconsistency, and "these two files disagree about
whether to freeze a length" is a code-review note rather than an issue. It is exactly the kind of
thing a port normalises by accident, which is why `Sequence::limit` exists (D-42).

The three **port** defects found while bridging this module — the `noalias` hoist, napi's latching
`#.return`, and `napi_create_reference` rejecting primitives — are documented in
`docs/modules/stack.md`, "Bugs this found". The first of them was *measured on this module's
`forEach`*, and it is also present in the already-scoped `sparse-set` bridge, which this pass does
not fix.

**What the fuzzer found: nothing new.** Two campaigns, 4.13 M operations, zero divergences — the
expected outcome for a faithful port (D-33).

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-42 | **`Sequence` gained a `limit` method, and `Queue` overrides it.** | `Queue.prototype.values` re-reads `items.length` every step; `Stack.prototype.values` freezes it. Defaulting to the frozen length leaves every other source unchanged. Normalising the two would have silently terminated a walk that upstream resumes. |
| D-41 | **The backing store is `Rc<RefCell<Vec<T>>>`, not `Vec<T>`.** | The compaction and `clear()` both **rebind** `this.items`, and a cursor holds the array it captured. A `Vec` cannot express the difference between rebinding and mutating in place. |
| — | **`dequeue` returns a clone.** | Upstream leaves the element in the array; moving it out would leave a hole upstream does not have, and a cursor with an older frozen offset can still legitimately yield it. |
| D-43 | **The bridge holds `RefCell<CoreQueue<JsSlot>>`.** | `&self` is `noalias readonly` for a `Freeze` type, and JS mutates through the same pointer from inside a `forEach` callback. Measured on this module: the port answered `1, 2, 3, 4` where upstream answers `1, 4, undefined, undefined`. |
| D-44 | **Values are a `JsSlot` enum, not one `napi_ref` each.** | `napi_create_reference` rejects primitives for a version-8 module. Exact, because primitives are immutable and compared by value. |
| D-45 | **`Queue.of` is installed as evaluated JavaScript.** | napi-rs has no variadic parameter and `arguments` has no Rust representation. |
| D-46 | **napi's generator `#.return` is deleted from every cursor.** | Upstream cursors have none, so `break` leaves the walk resumable. |
| D-06 / D-07 | **No `IntoIterator`; `Symbol.iterator` installed from Rust.** | As `stack`, and for the same reasons. |
| D-03 | **`forEach` lives in `mnemonist-napi`, not core.** | As `stack`. |
| — | **`offset` is exposed to JS.** | It is a public property upstream, and unlike `SparseSet`'s typed arrays it is a plain number, so handing it over loses nothing — and it is what makes the compaction observable at all. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion. |
| — | **`forEach(cb, undefined)` binds `this` to the queue.** | Upstream keys off `arguments.length > 1`, which napi's typed signature cannot see. The omitted-argument case is exact. |

## Fuzz + bench

### Fuzz

```
module=queue seed=42       cases=25731 ops=2559824 wall=120.0s divergences=0
module=queue seed=20260801 cases=15805 ops=1573126 wall=60.0s  divergences=0
```

Two campaigns, two seeds, **4.13 M operations, zero divergences**.

Reproduce with `target/release/difffuzz --module queue --seed 42 --cases 25731`.

* **Op alphabet:** `enqueue(v)` (weight 6) · `dequeue()` (4) · `peek()` (2) · `clear()` (1) ·
  `$iter("values")` (2) · `$next()` (4) · `$spread()` (1).
* **Observable state, compared after every op:** `size`, **`offset`**, **`items`** and `toArray()`.
  The middle two are the point: `toArray()` alone cannot tell a compacted queue from an
  uncompacted one holding the same elements, so a port could get the entire schedule wrong for a
  whole program with nothing noticing.
* **Values:** `0..48`.
* **Program length:** 1..200 ops.
* **Deliberately excluded: nothing.**

`dequeue` is weighted heavily because the compaction only fires on a dequeue and the schedule needs
several in a row to be interesting; `$next` outweighs `$iter` four to one so that cursors are
stepped repeatedly, with enqueues and compactions landing between the steps. The
step-after-exhaustion case is reached constantly, which matters here in a way it does not for
`stack`: for a queue it is a *behaviour*, not a corner.

**The fuzzer was falsified before it was trusted.** Sabotage: `Sequence::limit` returning the
frozen length — that is, giving the queue the stack's cursor, which is exactly what a port that
generalised one cursor shape across modules would produce. Caught in **99 cases (0.1 s)**, shrunk
from 200 ops to **three**, which is the smallest program that can express it:

```js
var s = new Queue();
var it = s.values();     // opened on an empty queue
s.enqueue(0);
it.next();               // port {done: true}, upstream {value: 0}
```

Note what that repro also demonstrates: the cursor had already run off the end of an empty queue,
and obliterator's `Iterator` has no done flag, so it resumes. Reverted; the seed is committed with
a provenance header in `crates/difffuzz/proptest-regressions/queue.txt`.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:**
`should be possible to dequeue` — `assert.strictEqual(queue.dequeue(), 2)`, at `test/queue.js:52`.
Chosen because it is the only assertion in the file whose value depends on the offset having
advanced; everything else in the suite is satisfied by a queue that ignores `offset` entirely.

**The sabotage:** `Queue::dequeue` reading `items[0]` instead of `items[offset]` — forgetting the
offset, which is the most plausible single mistake in this file.

**Confirmed red**, and red in exactly the named place: `10 passing, 1 failing`, with
`actual 1, expected 2` at `test/queue.js:52`. Reverted; **confirmed green again**: `11 passing`.

Worth recording what this sabotage does *not* break, because it bounds what gate 6 proves here:
the compaction schedule itself is invisible to the original suite. A sabotage of
`++offset * 2 >= items.length` — say, `offset >= items.length` — leaves all 11 blocks green. That
is not a weakness in the choice of sabotage; it is the measurement of how thin the upstream
coverage is, and it is why `offset` and `items` are both in the fuzzer's observation set.

### Bench

**Not run.** Benchmarks need an idle machine and three agents were running concurrently; gate 10 is
batched into a quiet pass (DESIGN.md §7.3). `queue` is ready for it, and its interesting workload
is not the same as `stack`'s: the compaction makes enqueue/dequeue churn amortised rather than
flat, so a workload that dequeues in bursts is worth having alongside a balanced one. Until that
pass lands, this unit is **not** in `tests/scope.txt` and does not claim to be done.

### `$forEach` — the op that was missing (added 2026-08-01, B-31)

`queue`'s grammar had no `forEach` op at all. That omission is what let B-31 — a `forEach`
callback mutating the collection it is walking — through 4.13 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it.

`$forEach(method, rule, limit)` now walks the instance with a callback that calls back into it.
The compared result is the sequence of callback argument pairs, so the walk's **shape** is checked
and not only the state it leaves behind. This module's mutations:

* `dequeue()`, `enqueue(a0)` and `clear()`, all uncapped.

The `dequeue` row is the one that matters: enough dequeues **compact**, and the compaction rebinds
`this.items` to a shorter array while `i` and `l` still refer to the old one, so the remaining reads
run off the end and yield `undefined`. That is the exact program in `CellCursor`'s doc comment,
which was written from a hand-built repro; it is now generated.

**What it does not reach, stated so the campaign is not over-read.** `difffuzz` compares
`mnemonist-core` against upstream JS; the napi bridge, where B-31's hoisted read actually lived, is
not in that loop. No op alphabet can catch that class of bug here. The specs that do are
`tests/boundary/reentrancy.js`, which drive the real addon with real JS callbacks — red on the
pre-fix bridges, green after.

One deliberate narrowing, mirrored on both sides: a selected callback argument that is `undefined`
skips the mutation. Feeding it back in reaches upstream's `NaN`-indexed swap, which `usize` cannot
express and the core does not model. Fully disclosed in `fuzz/log.txt`.

# sparse-queue-set

Upstream: `sparse-queue-set.js` (218 LOC) + `utils/typed-arrays.js` (187 LOC, only
`getPointerArray` reachable) · `test/sparse-queue-set.js` — **134 lines, 7 `it` blocks, 24
assertion statements** (though the wrap-around block runs 13 of them 13 times, so ~250 assertions
execute).

Port: `crates/mnemonist-core/src/structures/sparse_queue_set.rs`,
`crates/mnemonist-core/src/cursor/mod.rs`, `crates/mnemonist-core/src/utils/typed_arrays.rs`.
Bridge: `crates/mnemonist-napi/src/sparse_queue_set.rs`, `crates/mnemonist-napi/src/cursor.rs`.

`SparseSet` with `dense` as a **ring**. `enqueue` appends at `(start + size) % capacity`, `dequeue`
takes from `start`, and membership is no longer "is the slot below `size`" but "is the slot inside
the live window, and does `dense[slot]` still hold this member" — a window that wraps. That single
change turns three of `sparse-set`'s well-behaved edges into new defects, all three verified
against Node 24.18.1 and all three reproduced.

---

## What upstream tests

Seven `it` blocks over capacities 4 and 10:

```js
var queue = new SparseQueueSet(10);
queue.enqueue(3); queue.enqueue(4); queue.enqueue(3);
assert.strictEqual(queue.size, 2);
assert.strictEqual(queue.capacity, 10);
// …has(3)/has(1), clear() then size+has, four enqueues then four dequeues…

// and the one block that does real work:
var queue = new SparseQueueSet(4), values = [2, 3, 1];
for (var i = 0; i < 13; i++) run();   // enqueue 3, take(values()), forEach, dequeue 3
```

This is the strongest of the three sibling suites, and it is worth saying why before listing what
it misses:

* **The wrap-around block is genuine.** Thirteen cycles of three enqueues and three dequeues on a
  capacity-4 ring rotates `start` through every value repeatedly, and checks `has`, `values()`,
  `forEach` and `dequeue` order on each pass. Iteration across a wrapped window is properly
  covered — the one thing this structure has that its siblings do not.
* **Two capacities are used, 4 and 10**, rather than one.
* **`dequeue`'s return value is asserted**, on every call, in two blocks.
* **`forEach`'s output is compared against `values()`'s** on each cycle, so the two walks are
  cross-checked against each other.

And then:

* **The ring is only ever three-quarters full.** `values` has three members and `capacity` is 4, so
  the block never enqueues a fourth while three are held.
* **Every member used is in range**, on both capacities. The largest is 3, against a capacity of 4.
* **Every dequeue is matched by an enqueue.** The queue is drained exactly, never over-drained.
* **Iteration is drained immediately.** `obliterator.take(queue.values())` creates a cursor and
  exhausts it in one expression.

## What upstream does NOT test

Everything below is reachable through the public API and never exercised by the original suite.

**The capacity boundary — where the structure's own sentinel stops working**

1. **No capacity near a pointer-width boundary is ever constructed.** Only 4 and 10, both
   comfortably inside `Uint8Array`. This hides B-12 entirely: `dequeue` marks a member absent by
   writing `capacity` into `sparse`, and `sparse` is sized for indices `0..capacity-1`, so at
   `capacity === 256` the sentinel truncates to `0` — an ordinary slot.
2. **The consequences of (1) are therefore all unreached:** a dequeued member reading as present
   again, and `enqueue` refusing to re-admit it.
3. **The 16- and 32-bit branches of `getPointerArray` are never reached**, and neither is the
   `capacity > 2³²` throw.
4. **No member ≥ 256 is ever enqueued**, so the truncating `dense` store never fires.

**The full ring**

5. **The queue is never filled to capacity.** The wrap block holds three of four slots.
6. **An `enqueue` into a full ring is never performed**, which hides B-13: nothing bounds `size` by
   `capacity`, and an out-of-range member evicts a **live** one.
7. **`size > capacity` is never reached**, and neither is any of what follows from it — a walk
   yielding more members than the ring has slots, with a duplicate in it.

**Degenerate capacities**

8. **`new SparseQueueSet(0)` is never constructed**, which hides B-14 entirely: `(start + size) %
   capacity` is `NaN`, both stores vanish, `size` still climbs, `start` grows without bound, and
   iteration yields `undefined` per phantom member.
9. **`new SparseQueueSet(1)` is never constructed** — the ring that wraps on every single dequeue.

**Out-of-range members — the whole regime**

10. **No member outside `0..capacity` is ever passed to anything.** That hides the truncating
    store (4), the eviction (6), and the fact that `has` past the end is safe while `enqueue` past
    the end is not.

**Emptiness**

11. **`dequeue()` on an empty queue is never called.** Every dequeue in the file is matched by an
    enqueue, so the `if (this.size === 0) return;` branch is untested and the `undefined` it
    returns is never observed.
12. **`values()` on an empty queue, or after `clear`, is never called.**
13. **`clear` is only ever called on an *unrotated* queue.** The block that clears enqueues six
    members into a capacity-10 queue and never dequeues, so `start` is still `0` and the fact that
    `clear` resets it as well as `size` — the one structural difference from `SparseSet.clear` — is
    not observed.

**Return values**

14. **`enqueue`'s return value is never used.** Upstream returns `this` for chaining.

**Iteration — everything except immediate drains**

15. **Mutation during iteration is never performed.** The cursor freezes `capacity`, `size` **and**
    `start`, and reads `dense` live, so a `dequeue` mid-walk does *not* move the walk while an
    `enqueue` that overwrites an unread slot *is* visible. Neither half is tested.
16. **A cursor is never re-drained**, so D-06 non-restartability is unobserved.
17. **`[...queue]` is never used.** The suite reaches the cursor only through `values()`, so the
    collection-level `Symbol.iterator` — the *factory* half of D-07 — has **zero** coverage despite
    being the last line of the module.
18. **The `undefined` window is never reached**, which follows from 8.

**`forEach`**

19. **The callback's second and third arguments are never inspected.** Upstream calls
    `callback.call(scope, this.dense[i], j, this)` — the ordinal and the queue itself. The test's
    callbacks declare one parameter.
20. **`scope` is never passed**, so the `arguments.length > 1 ? scope : this` branch is untested.
21. **`forEach`'s frozen loop is never distinguished from a live one.** This `forEach` captures
    `c`, `l` and `i` before its loop, where `SparseSet`'s and `SparseMap`'s re-read `this.size`
    every iteration — so a callback that dequeues shortens *their* loops and not this one. No
    callback in the file mutates.

**Never called at all**

22. `inspect()` and the `nodejs.util.inspect.custom` symbol.

## What we test in addition

Rust native tests, mapped to the gaps above.

`crates/mnemonist-core/src/structures/sparse_queue_set.rs` — 17 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all seven upstream blocks, including all 13 wrap cycles, as a baseline |
| `the_dequeue_sentinel_truncates_at_the_pointer_width_boundary` | **1, 2, 3** — B-12 at `capacity` 256, pinned on `sparse`, on the false-positive `has`, *and* on the enqueue that consequently does nothing |
| `one_below_the_boundary_the_sentinel_fits` | 1, 3 — the control at 255, so the defect is attributed to the boundary and not to the port |
| `the_sentinel_truncates_at_the_second_boundary_too` | 1, 3 — and at 65536, one width up. This is also where the fuzzer's disclosed exclusion is covered |
| `an_out_of_range_enqueue_evicts_a_live_member` | **5, 6, 7, 10** — B-13, pinned on `dense` slot by slot and on a walk that yields five members from a four-slot ring |
| `an_out_of_range_member_truncates_into_the_ring` | 4, 10 — the truncating store, and that the member dequeues under its truncated name |
| `a_zero_capacity_queue_counts_phantoms` | **8, 12, 18** — B-14: the `NaN` index, the dropped stores, `start` climbing past its own capacity, and every step a gap |
| `dequeuing_an_empty_queue_changes_nothing` | 11 |
| `a_one_slot_ring_wraps_on_every_dequeue` | 9, 14 |
| `clear_resets_the_rotation_as_well_as_the_size` | 13 — clears a *rotated* queue, and asserts the debris stays unreachable afterwards |
| `membership_holds_across_a_wrapped_window` | the `\|\|` in the window test, on the wrapped case, with the non-member checked too |
| `cursors_do_not_restart_but_the_queue_can_be_walked_again` | 16, 17 — both levels of D-07 |
| `a_dequeue_during_iteration_does_not_move_the_walk` | **15** — the frozen-`start` half, which is the opposite of what a live cursor would do |
| `an_enqueue_that_overwrites_an_unread_slot_is_visible` | **15** — the live-`dense` half, driven through an out-of-range enqueue so no duplicate check fires |
| `picks_one_pointer_width_for_both_arrays` | 3 — five capacities across both width boundaries |
| `rejects_a_capacity_no_pointer_array_can_index` | 3 (the throw) |
| `fills_and_drains_a_full_ring` | 5 — 300 members in, 300 out in order, and the queue back at `start == 0` |

**Every expectation in these tests was run against real Node first**, including the 255/256/65536
boundary trio and the two mutation-during-iteration cases, rather than reasoned about and hoped
for. That is the method settled on early in the project and it paid again here: B-12's
behaviour at 256 is not something reading the file makes obvious.

The **differential fuzzer** then covers gaps 1–18 continuously, with `start` in the observed state
so rotation is compared after every operation of every generated program.

**Still untested, stated rather than glossed:** gap 22 (`inspect`, not bridged), gaps 19–21 for
`forEach` (which lives at the bridge, and is exercised by the original suite
through the harness), and non-integer members, which napi's `u32` coercion rejects before any port
code runs.

## Bugs this found

**B-12 — `dequeue`'s absence sentinel does not fit the array it is written into.**
`status: verified against Node 24.18.1`. `dequeue` marks a member absent by writing a value no live
slot can hold:

```js
this.sparse[member] = this.capacity;
```

But `sparse` is `getPointerArray(capacity)` wide, and that function sizes for the largest *index*,
`capacity - 1`. At `capacity === 256` the array is a `Uint8Array`, and `256` truncates to **`0`** —
a perfectly ordinary slot. Measured:

```js
var q = new SparseQueueSet(256);
q.enqueue(5); q.dequeue();   // sparse[5] is 0, not 256
q.enqueue(7);
q.has(5)          // true   ← 5 was dequeued
q.enqueue(5);
[...q]            // [7]    ← and it can never be re-admitted
```

The control at `capacity === 255` gives `sparse[5] === 255`, `has(5) === false`, and a re-enqueue
that works. The same defect recurs one width up at `capacity === 65536` (`Uint16Array`), confirmed;
2³² is unreachable, since `getPointerArray` throws first. **So the bug is at exactly the two
capacities where `getPointerArray` switches width, and nowhere else** — which is why the fuzzer
draws 256 as a point rather than hoping a uniform range lands on it.

Two symptoms, and the second is worse than the first: a false-positive `has`, and an `enqueue` that
believes the false positive and silently refuses. Reproduced rather than fixed — the port gets it
for free, because `PointerVec::try_set` narrows exactly as a typed-array store does.

**B-13 — `enqueue` never checks whether the ring is full.**
`status: verified against Node 24.18.1`. Nothing bounds `size` by `capacity`. In range that is
unreachable: a queue holding every member of `0..capacity` rejects any further `enqueue` as a
duplicate. But one out-of-range member is enough, because `sparse[member]` is then `undefined` and
the duplicate check cannot fire:

```js
var q = new SparseQueueSet(4);
q.enqueue(0); q.enqueue(1); q.enqueue(2); q.enqueue(3);
q.enqueue(100);        // out of range
q.dense                // [100, 1, 2, 3]      ← member 0 silently evicted
q.size                 // 5, against capacity 4
q.has(0)               // false
[...q]                 // [100, 1, 2, 3, 100] ← five members from a four-slot ring
```

This is a strictly nastier corruption than B-8, `SparseSet`'s equivalent: there the out-of-range
write lands past the end of the array and destroys nothing, because the slot belonged to nobody.
Here `(start + size) % capacity` keeps the index **inside** the ring by construction, so it always
lands on a live slot once the ring is full.

**B-14 — `capacity === 0` divides by zero.**
`status: verified against Node 24.18.1`. `(this.start + this.size) % this.capacity` is `NaN`, and
`dense[NaN] = member` is a string-keyed expando rather than an element store, so both writes vanish
while `size` still increments. Three consequences:

```js
var q = new SparseQueueSet(0);
q.enqueue(0);
Array.from(q)      // [undefined]        ← the shrink window, in two calls
q.dequeue()        // undefined, and sets sparse.undefined = 0 (the B-10 expando)
q.start            // 1, and it keeps climbing: the wrap check is
                   // `start === capacity`, i.e. `1 === 0`, which is never true
```

The unbounded `start` is the part worth dwelling on. Every other structure in this family bounds
its indices by construction; here a field that is documented as an index into a zero-length array
grows without limit, and nothing ever reads it back into range. It is also the only observable
difference produced by falsification B below — which is why `start` is in the fuzzer's observed
state.

**B-8, B-9 and B-10 have analogues here**, since `has`'s guard structure and the out-of-range store
behaviour are `SparseSet`'s. The `sparse[undefined]` expando of B-10 reappears in `dequeue` at
`capacity === 0`, where `member` is `undefined`.

**What the fuzzer found: nothing new.** Two campaigns, 3.36 M operations, zero divergences — the
expected outcome for a faithful port (D-33), and the third module in a row to produce it. B-12,
B-13 and B-14 were all found by reading the file statement by statement and confirming each step
against Node. What the fuzzer is for is the other direction, and it was proven to work in that
direction twice (see Fuzz, below).


### B-31 — `&self` on a `Freeze` type was `noalias readonly` (fixed 2026-08-01)

This bridge held a bare core value, so `&self` compiled to a `noalias readonly` pointer and LLVM was
entitled to hoist reads across the JS callback — which it did. It now holds `RefCell<Core>`, which
is not `Freeze`, and every `&mut self` method became `&self` + `borrow_mut()`. The borrow is taken
per step and released before the callback runs, so a re-entrant callback never meets an outstanding
borrow. See B-31, above, and `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **The window test reproduces upstream's precedence, not a simplification of it.** | `index < capacity && (index >= start && index < start + size) \|\| (index < (start + size) % capacity)` is one three-term conjunction **or** one comparison, because `&&` binds tighter than `\|\|` — not the capacity guard distributed over both arms. The guard is provably redundant, since `(start + size) % capacity < capacity` always holds; reproducing the shape anyway matters because at `size > capacity` the two arms overlap instead of partitioning, and a "simplified" version would have to be re-derived for that case. |
| — | **The three defects are reproduced, not repaired.** | B-12, B-13 and B-14 are all observable through the public API, so fixing any of them would be a silent behavioural divergence. Each is pinned by native tests and, for B-13 and B-14, by a committed fuzz seed. |
| — | **`x % 0` is `None`, not a panic.** | JS gives `NaN` and every comparison against it is false; Rust's `%` panics on a zero divisor. The two places that compute it return `Option` and the `None` arm reproduces what JS's `NaN` does — a dropped store in `enqueue`, a false membership test in `has`, an unwrapped index in the walk. |
| D-09 | **The shrink window is reproduced (Option A), not collapsed.** | Reachable here at `capacity === 0`, in two calls, on a *different* module and by a *different* route from `sparse-set`'s. |
| D-06 | **No collection implements `IntoIterator`.** | It would hand out a fresh iterator per `for` loop and silently restart. |
| D-07 | **`Symbol.iterator` is installed from Rust**, aliased to `values`. | The factory half is the one napi does not provide. |
| — | **The Rust-side `Iterator` impl skips gaps rather than stopping.** | As `sparse-set`. The faithful three-way primitive is `step()`. |
| — | **`enqueue` returns `bool` in core; the bridge returns `this`.** | Core reports whether the member was newly enqueued, which upstream exposes only through `size`. |
| — | **`dequeue` returns `Option<u32>` in core and `Either<u32, Undefined>` at the bridge.** | Upstream's empty-queue return is a bare `return;`. napi renders `Option::None` as `null`, which `assert.strictEqual` distinguishes from `undefined`. |
| — | **`size`, `capacity` and `start` are read-only getters** where upstream's are writable data properties. | Reproducing the writability would mean accepting arbitrary values into fields that every method's arithmetic trusts. The original suite writes none of the three. |
| — | **`dense` and `sparse` are not exposed to JS.** | They are public typed arrays upstream and a JS caller can write *through* them; napi can only hand out a copy. Both are exposed in Rust and compared slot for slot by the fuzzer. |
| — | **`forEach` lives at the bridge, and drives the same `CursorState` the cursor does.** | `forEach` is kept out of the core. Worth recording that **this** `forEach` freezes `c`, `l` and `i` before its loop, where `SparseSet`'s and `SparseMap`'s re-read `this.size` each iteration — so a callback that dequeues does not shorten this loop and would shorten theirs. Driving the shared `CursorState` reproduces that without a second loop to drift from the first. |
| — | **`forEach(cb, undefined)` binds `this` to the queue.** | Upstream keys off `arguments.length > 1`, which napi's typed signature cannot see. The omitted-argument case — the only one the original suite uses — is exact. |
| — | **`new()` returns `Result`.** | Upstream throws for `capacity > 2³²`. Same treatment as its siblings. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion. |

## Fuzz + bench

### Fuzz

```
module=sparse-queue-set seed=42       cases=21564 ops=2254268 wall=120.0s divergences=0
module=sparse-queue-set seed=20260801 cases=10535 ops=1103405 wall=60.0s  divergences=0
```

Two campaigns, two seeds, **3.36 M operations, zero divergences**. Reproduce with
`target/release/difffuzz --module sparse-queue-set --seed 42 --cases 21564`.

* **Op alphabet:** `enqueue(m)` (weight 4) · `dequeue()` (3) · `has(m)` (3) · `clear()` (1) ·
  `$iter("values")` (2) · `$next()` (3) · `$spread()` (1). `dequeue` is weighted heavily because it
  is the **only** op that moves `start`, and the ring is what this module has that its siblings do
  not; a read-heavy mix would fill the ring once and never rotate it.
* **Observable state, compared after every op:** `size`, `capacity`, **`start`**, `dense`,
  `sparse`. `start` earns its place empirically — falsification B below differs in nothing else.
* **Capacities:** a mixture rather than one range, because the interesting capacities are not
  uniformly distributed. `0..=400` (weight 4, with 0 for B-14); `1..=8` (weight 3, so a 200-op
  program wraps many times rather than filling once); and `{255, 256}` (weight 2) — the B-12
  boundary drawn as a **point** with its control, since a uniform draw over `0..=400` reaches 256
  about once in 400 programs.
* **Members:** `0..capacity + 64`, so roughly one in eight is out of range.
* **Program length:** 1..200 ops.
* **Deliberately excluded: capacity 65536**, the second B-12 boundary. It is the same defect one
  width up and is covered by `the_sentinel_truncates_at_the_second_boundary_too` in the core's
  tests instead. Including it cost about **95% of throughput — measured, 880 op/s against 15,000**
  — because the observable state is two backing arrays, serialised, sent and compared after every
  single operation. A 60-second campaign that executes 5% of its programs is a worse check than a
  native test plus a fast campaign. Nothing else is excluded; every out-of-range member is
  generated.

The B-12 sentinel needs no special op to observe: `sparse` is in the observed state, so every
`dequeue` compares the value written into it.

**The fuzzer was falsified twice, once per half of this module's new grammar.** Both sabotages were
reverted and both seeds are committed with provenance in
`crates/difffuzz/proptest-regressions/sparse-queue-set.txt`.

**A — the ring.** Sabotage: `enqueue` "fixed" to refuse a full ring, i.e. B-13 repaired. Caught in
**811 cases (0.8 s)**, shrunk to two operations on the smallest possible ring:

```js
var s = new SparseQueueSet(1);
s.enqueue(0);
s.enqueue(1);   // out of range on a capacity-1 queue
                // port dense [0] size 1, upstream dense [1] size 2
```

The repro doubles as the smallest possible demonstration of B-13 itself: at capacity 1, the second
enqueue evicts the first.

**B — the degenerate capacity.** Sabotage: `dequeue`'s wrap check tidied from `===` to `>=`.
Reading `if (this.start === this.capacity) this.start = 0;` as a bounds check and "hardening" it is
the most invisible change available in this module — the two are identical for every capacity *but
zero*. Caught in **854 cases (3.9 s)**, shrunk to two operations:

```js
var s = new SparseQueueSet(0);
s.enqueue(0);
s.dequeue();   // port start 0, upstream start 1
```

Nothing else differs: not `size`, not the members, not either backing array. That seed is
simultaneously the justification for observing `start` and the proof that the constructor strategy
really does generate capacity 0.

### Falsification of the port (gate 6)

Separate from the fuzzer falsifications: gate 6 asks that sabotaging the core turns the *original
mocha suite* red, proving it exercises Rust rather than a JS fallback.

**The assertion the sabotage had to break was named first:** the wrap-around block's
`assert.deepStrictEqual(obliterator.take(queue.values()), values)`, at
`test/sparse-queue-set.js:77`. Chosen because it is the only assertion in the file that runs
against a **rotated** ring — it passes trivially on the first of the block's 13 cycles, when
`start` is still `0`, and only bites from the second.

**The sabotage:** `Sequence::slot` computing `start + ordinal` without the modulo, i.e. reading the
ring as linear. That is the plausible mis-port of upstream's `i++; if (i === c) i = 0;`, which does
not look like a modulo when you read it.

**Confirmed red**, and red in precisely the named place: `6 passing, 1 failing`, the failure at
`test/sparse-queue-set.js:77` on the second cycle. Reverted; **confirmed green again**:
`7 passing`.

**And a second falsification that was expected to stay green, and did.** Following `sparse-map`'s
lead, it is worth knowing *which* sabotages this suite cannot catch. The natural repair for B-13 —
`if (size >= capacity) return this;` in `enqueue` — leaves the suite at **7 passing, 0 failing**
while turning **three** native tests red (`an_out_of_range_enqueue_evicts_a_live_member`,
`an_enqueue_that_overwrites_an_unread_slot_is_visible`, `a_zero_capacity_queue_counts_phantoms`)
and being caught by the differential fuzzer in 0.8 seconds. The suite cannot see it because the
only member that could enqueue into a full ring is one already present, and the wrap block never
fills the ring anyway.

### Bench

**Not yet run.** Gate 10 is deliberately batched into a separate quiet serial pass — a benchmark
taken under load is not a slow benchmark, it is a wrong one, and this host has already demonstrated
a contended run inflating both sides 2–3×. This module is therefore
**not** in `tests/scope.txt`: gates 1–9 are green and gate 10 is outstanding, which is exactly what
`tests/verify.sh` will report.

### `$forEach` — the op that was missing (added 2026-08-01, B-31)

`sparse-queue-set`'s grammar had no `forEach` op at all. That omission is what let B-31 — a `forEach`
callback mutating the collection it is walking — through 3.36 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it.

`$forEach(method, rule, limit)` now walks the instance with a callback that calls back into it.
The compared result is the sequence of callback argument pairs, so the walk's **shape** is checked
and not only the state it leaves behind. This module's mutations:

* `dequeue()`, `enqueue(a0 + 1)` and `clear()`, all uncapped.

All three are safe uncapped, and that is the finding this op pins: unlike `sparse-set` and
`sparse-map`, this module's `forEach` captures `c`, `l` and `i` **before** the loop, so nothing the
callback does can lengthen or shorten the walk. The inconsistency is upstream's, and no program the
old alphabet could generate distinguished the two behaviours.

**What it does not reach, stated so the campaign is not over-read.** `difffuzz` compares
`mnemonist-core` against upstream JS; the napi bridge, where B-31's hoisted read actually lived, is
not in that loop. No op alphabet can catch that class of bug here. The specs that do are
`tests/boundary/reentrancy.js`, which drive the real addon with real JS callbacks — red on the
pre-fix bridges, green after.

One deliberate narrowing, mirrored on both sides: a selected callback argument that is `undefined`
skips the mutation. Feeding it back in reaches upstream's `NaN`-indexed swap, which `usize` cannot
express and the core does not model. Fully disclosed in `fuzz/log.txt`.

### Bench

`bench/results.json` → `modules["sparse-queue-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `enqueue`/`has`/`dequeue` (50/25/25) over capacity 1e6, xorshift32 seed
42, `sparse-set`'s add/has/delete shape with FIFO names — `dequeue` takes no operand, so the
workload's second operand goes unused on that op exactly as `has`'s does on `sparse-set`'s own
workload. Members drawn in range, so this never reaches B-13's out-of-range eviction path; in range
the ring's own ceiling does the interesting thing on its own once every member has cycled through.

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **8.4** | 12.9 | 1.5× faster |
| p99 ns/op | **23.4** | 61.2 | 2.6× faster |
| min ns/op | **5.8** | 10.2 | 1.8× faster |
| RSS delta MB | **11.6** | 41.3 | |
| structure-only RSS delta MB | **1.3** | 9.8 | |
| startup ms | **0.6** | 16.5 | 27× (reported separately; not throughput) |

No regressions. B-12 (the dequeue sentinel truncating at 256/65536) and B-13/B-14 are all reachable
only through out-of-range members, which this in-range workload never draws — consistent with
`sparse-set`'s own bench doc, which makes the same call for the same reason: benchmarking the
corruption path measures a bug, not a data structure.

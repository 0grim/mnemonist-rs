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
   comfortably inside `Uint8Array`. This hides BUG-SPARSE-QUEUE-SET-1 entirely: `dequeue` marks a member absent by
   writing `capacity` into `sparse`, and `sparse` is sized for indices `0..capacity-1`, so at
   `capacity === 256` the sentinel truncates to `0` — an ordinary slot.
2. **The consequences of (1) are therefore all unreached:** a dequeued member reading as present
   again, and `enqueue` refusing to re-admit it.
3. **The 16- and 32-bit branches of `getPointerArray` are never reached**, and neither is the
   `capacity > 2³²` throw.
4. **No member ≥ 256 is ever enqueued**, so the truncating `dense` store never fires.

**The full ring**

5. **The queue is never filled to capacity.** The wrap block holds three of four slots.
6. **An `enqueue` into a full ring is never performed**, which hides BUG-SPARSE-QUEUE-SET-2: nothing bounds `size` by
   `capacity`, and an out-of-range member evicts a **live** one.
7. **`size > capacity` is never reached**, and neither is any of what follows from it — a walk
   yielding more members than the ring has slots, with a duplicate in it.

**Degenerate capacities**

8. **`new SparseQueueSet(0)` is never constructed**, which hides BUG-SPARSE-QUEUE-SET-3 entirely: `(start + size) %
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
16. **A cursor is never re-drained**, so DIV-STACK-1 non-restartability is unobserved.
17. **`[...queue]` is never used.** The suite reaches the cursor only through `values()`, so the
    collection-level `Symbol.iterator` — the *factory* half of DIV-STACK-2 — has **zero** coverage despite
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

`crates/mnemonist-core/src/structures/sparse_queue_set.rs` — 17 tests, closing every gap above
except 19–22: a 1:1 reproduction of all seven upstream blocks including all 13 wrap cycles as a
baseline, the dequeue sentinel truncating at both pointer-width boundaries (with a control one
below each boundary), an out-of-range enqueue evicting a live member pinned slot by slot, a
zero-capacity queue's phantom members, a one-slot ring wrapping on every dequeue, `clear` resetting
rotation as well as size, and both mutation-during-iteration cases (frozen `start`, live `dense`).
Full test-to-gap mapping: evidence file.

**Every expectation in these tests was run against real Node first**, including the 255/256/65536
boundary trio and the two mutation-during-iteration cases, rather than reasoned about and hoped
for. BUG-SPARSE-QUEUE-SET-1's behaviour at 256 is not something reading the file makes obvious.

The **differential fuzzer** then covers gaps 1–18 continuously, with `start` in the observed state
so rotation is compared after every operation of every generated program.

**Still untested, stated rather than glossed:** gap 22 (`inspect`, not bridged), gaps 19–21 for
`forEach` (which lives at the bridge, and is exercised by the original suite
through the harness), and non-integer members, which napi's `u32` coercion rejects before any port
code runs.

## Bugs this found

**BUG-SPARSE-QUEUE-SET-1 — `dequeue`'s absence sentinel does not fit the array it is written into.**
Verified against Node 24.18.1. `dequeue` marks a member absent by writing a value no live
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

**BUG-SPARSE-QUEUE-SET-2 — `enqueue` never checks whether the ring is full.**
Verified against Node 24.18.1. Nothing bounds `size` by `capacity`. In range that is
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

This is a strictly nastier corruption than BUG-SPARSE-SET-1, `SparseSet`'s equivalent: there the out-of-range
write lands past the end of the array and destroys nothing, because the slot belonged to nobody.
Here `(start + size) % capacity` keeps the index **inside** the ring by construction, so it always
lands on a live slot once the ring is full.

**BUG-SPARSE-QUEUE-SET-3 — `capacity === 0` divides by zero.**
Verified against Node 24.18.1. `(this.start + this.size) % this.capacity` is `NaN`, and
`dense[NaN] = member` is a string-keyed expando rather than an element store, so both writes vanish
while `size` still increments. Three consequences:

```js
var q = new SparseQueueSet(0);
q.enqueue(0);
Array.from(q)      // [undefined]        ← the shrink window, in two calls
q.dequeue()        // undefined, and sets sparse.undefined = 0 (the BUG-SPARSE-SET-3 expando)
q.start            // 1, and it keeps climbing: the wrap check is
                   // `start === capacity`, i.e. `1 === 0`, which is never true
```

The unbounded `start` is the part worth dwelling on. Every other structure in this family bounds
its indices by construction; here a field that is documented as an index into a zero-length array
grows without limit, and nothing ever reads it back into range. It is also the only observable
difference produced by fuzzer falsification B — which is why `start` is in the fuzzer's observed
state.

**BUG-SPARSE-SET-1, BUG-SPARSE-SET-2 and BUG-SPARSE-SET-3 have analogues here**, since `has`'s guard structure and the out-of-range store
behaviour are `SparseSet`'s. The `sparse[undefined]` expando of BUG-SPARSE-SET-3 reappears in `dequeue` at
`capacity === 0`, where `member` is `undefined`.

**What the fuzzer found: nothing new.** Two campaigns, 3.36 M operations, zero divergences — the
expected outcome for a faithful port, and the third module in a row to produce it. BUG-SPARSE-QUEUE-SET-1,
BUG-SPARSE-QUEUE-SET-2 and BUG-SPARSE-QUEUE-SET-3 were all found by reading the file statement by statement and confirming each step
against Node. What the fuzzer is for is the other direction, and it is sharper than the original
suite by a wide margin — see "Fuzz + bench".

**The bridge held a bare core value behind `&self`**, which LLVM was entitled to compile as a
`noalias readonly` pointer and hoist reads across a re-entrant JS callback (PORTBUG-1). It now holds
`RefCell<Core>`, which is not `Freeze`, and every `&mut self` method borrows via `borrow_mut()`
taken per step and released before the callback runs, so a re-entrant callback never meets an
outstanding borrow. See `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`. Full history in the
log.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **The window test reproduces upstream's precedence, not a simplification of it.** | `index < capacity && (index >= start && index < start + size) \|\| (index < (start + size) % capacity)` is one three-term conjunction **or** one comparison, because `&&` binds tighter than `\|\|` — not the capacity guard distributed over both arms. The guard is provably redundant, since `(start + size) % capacity < capacity` always holds; reproducing the shape anyway matters because at `size > capacity` the two arms overlap instead of partitioning, and a "simplified" version would have to be re-derived for that case. |
| — | **The three defects are reproduced, not repaired.** | BUG-SPARSE-QUEUE-SET-1, BUG-SPARSE-QUEUE-SET-2 and BUG-SPARSE-QUEUE-SET-3 are all observable through the public API, so fixing any of them would be a silent behavioural divergence. Each is pinned by native tests and, for BUG-SPARSE-QUEUE-SET-2 and BUG-SPARSE-QUEUE-SET-3, by a committed fuzz seed. |
| — | **`x % 0` is `None`, not a panic.** | JS gives `NaN` and every comparison against it is false; Rust's `%` panics on a zero divisor. The two places that compute it return `Option` and the `None` arm reproduces what JS's `NaN` does — a dropped store in `enqueue`, a false membership test in `has`, an unwrapped index in the walk. |
| DIV-SPARSE-SET-1 | **The shrink window is reproduced (Option A), not collapsed.** | Reachable here at `capacity === 0`, in two calls, on a *different* module and by a *different* route from `sparse-set`'s. |
| DIV-STACK-1 | **No collection implements `IntoIterator`.** | It would hand out a fresh iterator per `for` loop and silently restart. |
| DIV-STACK-2 | **`Symbol.iterator` is installed from Rust**, aliased to `values`. | The factory half is the one napi does not provide. |
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

Two campaigns, two seeds, **3.36 M operations, zero divergences**:

```
module=sparse-queue-set seed=42       cases=21564 ops=2254268 wall=120.0s divergences=0
module=sparse-queue-set seed=20260801 cases=10535 ops=1103405 wall=60.0s  divergences=0
```

Reproduce with `target/release/difffuzz --module sparse-queue-set --seed 42 --cases 21564`.

The op alphabet covers `enqueue`/`dequeue`/`has`/`clear` plus the cursor ops; `dequeue` is weighted
heavily because it is the **only** op that moves `start`, and the ring is what this module has that
its siblings do not — a read-heavy mix would fill the ring once and never rotate it. Observable
state includes `size`, `capacity`, **`start`**, `dense` and `sparse` — `start` earns its place
empirically, since fuzzer falsification B (below) differs in nothing else. Capacities are drawn from
a mixture rather than one range, because the interesting capacities are not uniformly distributed:
most of the mass sits on `0..=400` and `1..=8` (so a 200-op program wraps many times rather than
filling once), with `{255, 256}` drawn as a point with its control, since a uniform draw over
`0..=400` reaches 256 about once in 400 programs. Members are drawn `0..capacity + 64`, so roughly
one in eight is out of range. **Deliberately excluded: capacity 65536**, the second BUG-SPARSE-QUEUE-SET-1 boundary —
the same defect one width up, covered instead by a dedicated core test. Including it in the grammar
cost about 95% of throughput (measured, 880 op/s against 15,000), because the observable state is
two backing arrays, serialised, sent and compared after every operation; a campaign that executes 5%
of its programs is a worse check than a native test plus a fast campaign. Nothing else is excluded;
every out-of-range member is generated. The BUG-SPARSE-QUEUE-SET-1 sentinel needs no special op to observe: `sparse`
is in the observed state, so every `dequeue` compares the value written into it. Full grammar:
evidence file.

**The fuzzer was falsified twice, once per half of this module's new grammar, and both leave the
original upstream suite green.** Sabotage A "fixes" `enqueue` to refuse a full ring (BUG-SPARSE-QUEUE-SET-2 repaired)
and is caught in 811 cases (0.8 s), shrunk to two operations on the smallest possible ring — the
repro doubles as the smallest demonstration of BUG-SPARSE-QUEUE-SET-2 itself, since at capacity 1 the second enqueue
evicts the first. Sabotage B tidies `dequeue`'s wrap check from `===` to `>=`, identical for every
capacity but zero, and is caught in 854 cases (3.9 s), shrunk to two operations that differ from
upstream in `start` alone — the seed that justifies observing `start` at all. Both reverted; both
seeds committed in `crates/difffuzz/proptest-regressions/sparse-queue-set.txt`. Full repro code:
evidence file.

**Falsification of the port (gate 6), separate from the fuzzer falsifications above:** the assertion
named first was the wrap-around block's
`assert.deepStrictEqual(obliterator.take(queue.values()), values)` at
`test/sparse-queue-set.js:77` — chosen because it is the only assertion in the file that runs
against a **rotated** ring, passing trivially on the first of the block's 13 cycles and only biting
from the second. The sabotage, `Sequence::slot` computing `start + ordinal` without the modulo (i.e.
reading the ring as linear), is confirmed red in precisely that place (6 passing, 1 failing, on the
second cycle); reverted, confirmed green again (7 passing).

A second falsification was expected to stay green, and did: the natural repair for BUG-SPARSE-QUEUE-SET-2 —
`if (size >= capacity) return this;` in `enqueue` — leaves the suite at 7 passing, 0 failing while
turning three native tests red and being caught by the differential fuzzer in 0.8 seconds. The suite
cannot see it because the only member that could enqueue into a full ring is one already present,
and the wrap block never fills the ring anyway. Full record: evidence file.

`$forEach(method, rule, limit)` walks the instance with a callback that calls back into it. This
module's mutations are `dequeue()`, `enqueue(a0 + 1)` and `clear()`, all uncapped. All three are safe
uncapped, and that is the finding this op pins: unlike `sparse-set` and `sparse-map`, this module's
`forEach` captures `c`, `l` and `i` **before** the loop, so nothing the callback does can lengthen or
shorten the walk. The inconsistency is upstream's, and no program the op alphabet could generate
without it would have distinguished the two behaviours. What it does not reach: the napi bridge,
where a re-entrant callback would actually run, is outside the loop `difffuzz` compares;
`tests/boundary/reentrancy.js` covers that instead. One deliberate narrowing, mirrored on both sides:
a selected callback argument that is `undefined` skips the mutation, because feeding it back in
reaches upstream's `NaN`-indexed swap, which `usize` cannot express and the core does not model.
Disclosed in `fuzz/log.txt`.

### Bench

`bench/results.json` → `modules["sparse-queue-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `enqueue`/`has`/`dequeue` (50/25/25) over capacity 1e6, members drawn in
range: the port is 1.5× faster at p50 (8.4 vs 12.9 ns/op), 2.6× faster at p99 (23.4 vs 61.2), 1.8×
faster at min. No regressions. Full table: evidence file.

BUG-SPARSE-QUEUE-SET-1 (the dequeue sentinel truncating at 256/65536) and BUG-SPARSE-QUEUE-SET-2/BUG-SPARSE-QUEUE-SET-3 are all reachable only through
out-of-range members, which this in-range workload never draws — consistent with `sparse-set`'s own
bench, which makes the same call for the same reason: benchmarking the corruption path measures a
bug, not a data structure.

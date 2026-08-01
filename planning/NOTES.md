# NOTES.md — running capture log

**Purpose:** raw material for the post-event write-up (**Write-Up Side Quest: $100 × 3, deadline
Aug 10 — a full week after code freeze**, judged on insight not followers). Not prose. Append
freely, fix nothing, delete nothing.

**The leverage:** the write-up costs zero hackathon hours *only if* the material is captured while
it happens. Nobody reconstructs a surprise at hour 70.

---

## Capture checklist — grab these DURING the event

- [ ] Terminal output of the **first** green upstream test run against the Rust port (screenshot + text)
- [ ] `SHA256SUMS` verification output
- [ ] **Every fuzz divergence**: the raw failure, the proptest-minimised repro, and the fix
- [ ] `proptest-regressions/` contents as they accumulate
- [ ] Wall-clock when each module lands (feeds a "what 72h actually looks like" chart)
- [ ] Benchmark numbers, including any **regressions** (honest ones are more interesting)
- [ ] Every moment of genuine surprise — those *are* the article
- [ ] Dead ends and what they cost in hours
- [ ] Anything an LLM got confidently wrong about the port (relevant to the event's AI framing)

---

## Bug candidates (upstream)

Must be filed upstream **during the event** to count for **Bug Catcher (+3, $100)**.
Status: `unverified` → `verified` → `filed #NNN` / `intentional`.

### B-1 — `iter`/`forEach` asymmetry on plain objects
`status: unverified` · `obliterator v2.0.5`
`take({a: 1})` **throws** (`iter.js` has no plain-object branch) while `forEach({a: 1}, cb)`
**iterates the values** (branch 5, `for…in`). Two helpers in the same library disagree about
whether a plain object is iterable.
**Likely intentional?** Possibly — `iter` must return an *iterator*, `forEach` only needs to visit.
Worth asking upstream regardless; even "intentional" is a documentation gap.

### B-2 — `toArray` produces sparse arrays when `guessLength` lies
`status: unverified` · `mnemonist utils/iterables.js`
`toArray` preallocates `new Array(guessLength(target))` then fills with `array[i++] = value`.
`guessLength` trusts `.length` then `.size` without validating against actual yield count.
Result on mismatch: **a sparse array with holes**, distinguishable from `undefined` in JS.
Sharpest case: `toArray({length: 5})` → `forEach` plain-object branch enumerates own properties
**including `length` itself** → `[5, <4 empty items>]`.
**This is the strongest candidate.** Concrete, reproducible in isolation, clearly unintended.

### B-3 — `take` with omitted `n`
`status: unverified` · `obliterator take.js`
`l = arguments.length > 1 ? n : Infinity`, then on early exhaustion `if (i !== n) array.length = i`.
With `n` omitted, `n === undefined`, so `i !== n` is **always** true. Benign today (no-op on a
growing array) but the guard doesn't express what it appears to intend.
**Low severity** — code-smell tier, not a behaviour bug. File only if others land.

### B-4 — `forEach` falsy guard rejects empty string and zero
`status: unverified` · `obliterator foreach.js`
`if (!iterable) throw`. So `forEach('', cb)` **throws** while `forEach('a', cb)` iterates.
An empty string is a legitimately iterable value that should yield zero times. Same for `0`
and `false` reaching a numeric path.
**Arguably intentional** as an input guard, but the empty-string case looks like a genuine miss.

### B-5 — `toString()` called on arbitrary input during dispatch
`status: unverified` · `obliterator foreach.js`
Branch 1 tests `iterable.toString() === '[object Arguments]'`. This **invokes `toString` on an
arbitrary user value** during type dispatch — a custom `toString` can throw, or return that exact
string and hijack the branch.
**Adversarially interesting**; low real-world impact.

### B-7 — `StaticDisjointSet.union` compares ranks of the ITEMS, not the ROOTS
`status: unverified — strong candidate` · `mnemonist static-disjoint-set.js`
Union-by-rank requires comparing the ranks of the two **roots**. Upstream reads the ranks of the
original arguments and then writes to the root:
```js
var xRoot = this.find(x), yRoot = this.find(y);
var xRank = this.ranks[x],       // <-- x, not xRoot
    yRank = this.ranks[y];       // <-- y, not yRoot
if (xRank < yRank)      this.parents[xRoot] = yRoot;
else if (xRank > yRank) this.parents[yRoot] = xRoot;
else { this.parents[yRoot] = xRoot; this.ranks[xRoot]++; }   // <-- writes xRoot
```
Reading at `x`/`y` but writing at `xRoot` is internally inconsistent, and non-root ranks are never
updated — so they stay 0 forever and the `else` branch fires almost always. The rank heuristic is
effectively disabled, degrading `find()` toward O(n) in the worst case.

**Results stay correct** — union-find is correct regardless of which tree is attached to which — so
this is a *performance* bug, not a correctness one. Note the consequence for us: **differential
fuzzing will never catch it**, because a faithful port reproduces it exactly. Found by reading.

**We reproduce it, we do not fix it.** `find(x)` returns a root, and which element becomes root is
observable; "fixing" it would be a silent behavioural divergence. Goes in `DECISIONS.md` as a
deliberate bug-for-bug reproduction, and upstream as an issue.
*Also a good write-up beat: the class of bug that differential testing structurally cannot find.*

### B-8 — `SparseSet.add(m)` with `m >= length` corrupts the set, three defects deep
`status: VERIFIED against Node 24.18.1` · `mnemonist sparse-set.js`
Neither guard in `add` fires for an out-of-range member, because `sparse[m]` is `undefined` and
every comparison against `undefined` is false. Three separate silent failures then follow in three
consecutive lines:
```js
this.dense[this.size] = member;    // (1) TRUNCATES — add(300) on a length-10 set stores 44
this.sparse[member]   = this.size; // (2) DROPPED — out-of-range typed-array store is a no-op
this.size++;                       // (3) happens anyway
```
Measured: `new SparseSet(10); add(300)` → `size === 1`, `dense === [44, 0, …]`, `sparse` untouched,
`has(300) === has(44) === false`. The member is stored, counted, iterable and unfindable under
either name.
**We reproduce it.** Unlike `StaticDisjointSet`'s out-of-range read, every step here is a
well-defined read, truncating store or dropped store, so the faithful port is expressible — and it
is cheaper than a guard as well as more useful. See `docs/modules/sparse-set.md`.

### B-9 — and therefore `size` can exceed `length`, so upstream's own iterator yields `undefined`
`status: VERIFIED against Node 24.18.1` · `mnemonist sparse-set.js`
The second-order consequence of B-8(3), and the more interesting half. `values()` freezes `size`
and `dense` is a fixed-length typed array, so:
```js
var u = new SparseSet(2);
u.add(100); u.add(101); u.add(102); u.add(103);   // size 4, length 2
[...u]  // → [100, 101, undefined, undefined]
```
**This is DESIGN.md §3.7's shrink window, reached through the public API in four calls** — two, on
a zero-length set. §3.7 chose Option A on the grounds that no upstream *test* reaches the window,
which measured Option B as costing zero on the 40% axis. That remains true and the conclusion was
right, but for a stronger reason than recorded: the window is not exotic, and the differential
fuzzer finds it in 0.3s when the port takes Option B.

### B-10 — `SparseSet.delete` past capacity writes a string-keyed expando onto a typed array
`status: VERIFIED against Node 24.18.1` · `mnemonist sparse-set.js`
```js
index = this.dense[this.size - 1];        // undefined once size > length
this.dense[this.sparse[member]] = index;  // LANDS, as 0 — a NaN element store is 0
this.sparse[index]              = ...;    // does NOT land — sparse[undefined] is a PROPERTY
```
`new SparseSet(3)`, add `0/1/2/99`, `delete(1)` → `dense = [0, 0, 2]`, `sparse = [0, 1, 2]`,
`sparse.undefined = 1`.
**This one caught the port.** The first cut wrote `sparse[0]`, which is what reading the three
lines as a unit produces rather than statement by statement. Fixed, pinned by a test, and then used
as fuzzer falsification sabotage A — caught in 6.6s and shrunk to seven ops.
*Lesson worth keeping: for a bug-for-bug port, read JS one statement at a time and confirm each
against the runtime. Reading for intent is how you write the correct version by accident.*

### B-11 — `SparseMap.delete` moves the key and leaves the value behind
`status: VERIFIED against Node 24.18.1` · `mnemonist sparse-map.js`
**The best find of the port so far, and the only one that needs no out-of-range input.**
`delete` is `SparseSet`'s swap-with-last copied verbatim into a structure that has a third
parallel array, and the third array is never touched:
```js
index = this.dense[this.size - 1];
this.dense[this.sparse[member]] = index;   // the last MEMBER moves into the hole
this.sparse[index]              = this.sparse[member];
this.size--;                               // and `vals` is never touched
```
Measured: `set(3,'a') set(4,'b') set(5,'c')` then `delete(3)` gives `get(5) === 'a'` and
`[...m] === [[5,'a'],[4,'b']]`. Member 5's value is member 3's. Holds for a typed value store too,
so it is the swap and not the `Array`.

**Why it survived, measured rather than argued.** The upstream file deletes exactly twice, both
times from a map holding ONE entry, where the swap is a self-assignment. Sabotaging our port to
*fix* the bug leaves `test/sparse-map.js` at **9 passing, 0 failing** — while turning **four** of
our native tests red and being caught by the differential fuzzer in **3.0 seconds**, shrunk to
three ops. That pair of numbers is the cleanest statement of the rigor gap this project has
produced: the suite is not weak in an obvious way (it covers both constructor signatures and all
three iterators), it just never builds a map big enough for its own `delete` to do anything.

*Write-up beat: "the test suite covered every method and still could not see the bug, because
every deletion it performs is a no-op by construction." Also the sharpest possible answer to
"why differential-fuzz a library with 525 passing tests".*

### B-12 — `SparseQueueSet.dequeue`'s absence sentinel does not fit its own array
`status: VERIFIED against Node 24.18.1` · `mnemonist sparse-queue-set.js`
`dequeue` marks a member absent by writing the capacity, as a value no live slot can hold:
```js
this.sparse[member] = this.capacity;
```
But `sparse` is `getPointerArray(capacity)` wide, and that function sizes for the largest *index*,
`capacity - 1`. At **capacity exactly 256** the array is a `Uint8Array` and the sentinel truncates
to **0** — an ordinary slot. Measured:
```js
var q = new SparseQueueSet(256);
q.enqueue(5); q.dequeue();     // sparse[5] is 0, not 256
q.enqueue(7);
q.has(5)        // true   <- 5 was dequeued
q.enqueue(5); [...q]           // [7]  <- and it can never be re-admitted
```
Control at capacity 255: `sparse[5] === 255`, `has(5) === false`, re-enqueue works. Same defect one
width up at **capacity 65536** (`Uint16Array`), confirmed. So the bug is at exactly the two powers
of two where `getPointerArray` switches, and 2³² is unreachable.
**Two symptoms, and the second is worse:** a false-positive `has`, and an `enqueue` that believes
it and refuses to re-admit the member. Reproduced, not fixed — `try_set` narrows, so the port gets
it for free.

### B-13 — `SparseQueueSet.enqueue` never checks whether the ring is full
`status: VERIFIED against Node 24.18.1` · `mnemonist sparse-queue-set.js`
Nothing bounds `size` by `capacity`. In range that is unreachable — a queue holding every member of
`0..capacity` rejects any further enqueue as a duplicate — but ONE out-of-range member is enough,
because `sparse[member]` is then `undefined` and the duplicate check cannot fire:
```js
var q = new SparseQueueSet(4);
q.enqueue(0); q.enqueue(1); q.enqueue(2); q.enqueue(3);
q.enqueue(100);          // out of range
q.dense                  // [100, 1, 2, 3]  <- member 0 silently evicted
q.size                   // 5, against a capacity of 4
q.has(0)                 // false
[...q]                   // [100, 1, 2, 3, 100]  — five members from a four-slot ring
```
The out-of-range write lands on a **live slot** rather than off the end, which is what makes this
different from B-8: `SparseSet.add(300)` corrupts a slot nobody was using, `enqueue(100)` evicts a
member that was legitimately queued.

### B-14 — `SparseQueueSet` with `capacity === 0` divides by zero
`status: VERIFIED against Node 24.18.1` · `mnemonist sparse-queue-set.js`
`(this.start + this.size) % this.capacity` is `NaN`, and `dense[NaN] = member` is a string-keyed
expando rather than an element store, so both writes vanish while `size` still increments. Then:
* `Array.from(q)` is `[undefined]` after one `enqueue` — DESIGN.md §3.7's shrink window, reached in
  **two calls**, on a different module from `sparse-set` and by a different route;
* `dequeue()` returns `undefined` and sets `sparse.undefined = 0`, the B-10 expando again;
* and `start` climbs **without bound**, because the wrap check is `start === capacity`, i.e.
  `1 === 0`, which is never true. Every other structure in this family bounds its indices.
### B-30 — `forEach` on a truthy primitive dies in the `in` operator, not in its own guard
`status: VERIFIED against Node 24.18.1` · `obliterator v2.0.4/2.0.5 foreach.js`
A truthy primitive — a number, a boolean, a symbol, a bigint — survives `if (!iterable) throw`,
is not an indexed sequence, and has no `.forEach`. It then reaches

```js
if (SYMBOL_SUPPORT && Symbol.iterator in iterable && ...)
```

and `in` **requires an object**. So:

```js
forEach(5, cb)  // TypeError: Cannot use 'in' operator to search for 'Symbol(Symbol.iterator)' in 5
```

Measured on Node 24.18.1 for `5`, `true`, `10n` (which stringifies as `10`, not `10n`) and
`Symbol(x)`. The error a caller sees names V8's operator rather than the library that called it,
and the library's *own* guard for "this is not iterable" never fires — a caller who passes a
number gets a message that reads like a bug in obliterator's internals.

**Related, same file, same dispatch:** `forEach(Object.create(null), cb)` throws
`TypeError: iterable.toString is not a function`, because branch 1 calls `toString()` unguarded
on an arbitrary value (that is B-5, with a second symptom). Both are reproduced verbatim by the
port and pinned in `tests/boundary/foreach.js`.

**Severity: low, but it is a genuine gap in a guard that exists.** Two lines would close it
(`typeof iterable !== 'object' && typeof iterable !== 'function'` before the `in`), and a
one-line doc note would close it as documentation. Worth filing alongside B-4, which is the same
guard's other blind spot.
### B-40 — `DefaultMap.get` tests the VALUE, not the key, and `size` then drifts without bound
`status: VERIFIED against Node 24.18.1` · `mnemonist default-map.js`
```js
DefaultMap.prototype.get = function(key) {
  var value = this.items.get(key);
  if (typeof value === 'undefined') {   // (1) asks about the VALUE, not `items.has(key)`
    value = this.factory(key, this.size);
    this.items.set(key, value);
    this.size++;                        // (2) a counter, where set/delete read items.size
  }
  return value;
};
```
Measured:
```text
m.set('a', undefined);   size 1   items.size 1
m.get('a');              size 2   items.size 1   factory called AGAIN
m.get('a');              size 3   items.size 1   factory called AGAIN
m.delete('a');           size 0   items.size 0   resynchronised
```
Three consequences: the factory re-runs on every *read* of a key whose value is `undefined` (so a
stateful factory such as `DefaultMap.autoIncrement()` advances on a read); `size` is unbounded in
the number of reads rather than the number of entries; and the drift is **silent and self-healing**,
because any `set` or `delete` snaps `size` back, so an interleaved program shows a `size` that is
sometimes right with nothing to say which.

Reproduced, not corrected. **The correction is what a careful porter writes by accident** — making
`size` return the entry count is tidier, is what the name suggests, and leaves all seven upstream
assertions green. Confirmed as gate-9 falsification sabotage A: the original suite stayed at
7 passing while the differential fuzzer caught it in 136 cases (0.1 s) and shrank it to two
operations. Full write-up in `docs/modules/default-map.md`.

*Lesson: the same one B-10 taught, from the other side. B-10 was found by reading statement by
statement; B-40 is a case where reading for intent gives you a **cleaner** program than upstream's,
and the original tests cannot tell the difference.*

### B-6 — `Stack.values()` captures `items.length`, not `this.size`
`status: unverified` · `mnemonist stack.js`
Other structures capture `this.size`. These coincide for `Stack` today; the inconsistency is latent
rather than active.
**Probably not a bug** — log it, don't file it.

### B-15 — `HashedArrayTree.pop` reads the last BLOCK, not the popped index's block
`status: VERIFIED against Node 24.18.1` · `mnemonist hashed-array-tree.js`
```js
var lastBlock = this.blocks[this.blocks.length - 1];   // the LAST block
var i = (--this.length) & this.offsetMask;             // offset of the POPPED index
return lastBlock[i];
```
The offset comes from the popped index; the block is taken unconditionally from the end of
`blocks`. They agree only while the tree fits in one block — which is the whole of upstream's
coverage, since its `pop` test uses the 1024-element default and pushes twice. Measured with
`blockSize: 2` after pushing `1, 2, 3`: `pop()` gives `3`, then `0`, then `3`. The `2` is
unreachable and the `3` comes back twice. `length` is decremented correctly, so only the returned
value is wrong and nothing downstream notices. A shrinking `resize` reaches it without any growth at
all, because `resize` never deallocates: push `7,8,9,10`, `resize(1)`, `pop()` gives `9`.
**Strong candidate** — a data structure returning the wrong element from `pop`.

### B-16 — `HashedArrayTree`'s bounds guard is `length < index`, admitting `index === length`
`status: VERIFIED against Node 24.18.1` · `mnemonist hashed-array-tree.js`
The same `if (this.length < index)` guards `set` and `get`, and the strict `<` lets one-past-the-end
through. Three different outcomes depending on where `length` sits:
* `get(length)` returns the raw block slot, so **a brand-new tree answers `get(0)` with `0`, not
  `undefined`** — which is exactly what upstream's own "should return undefined on out-of-bound
  values" test asserts, one index away (it asks for index 2 on a length-0 tree).
* `set(length, v)` **writes**, and `length` does not move. Invisible to `pop`, visible to `get`.
* when the admitted index is also `capacity`, `blocks[capacity >> blockMask]` is `undefined` and
  upstream raises `TypeError: Cannot set properties of undefined (setting '0')`.

### B-17 — `BitSet`/`BitVector.reset` omits the `>>> 0` that `set` and `flip` apply, so `size` drifts and can go NEGATIVE
`status: VERIFIED against Node 24.18.1` · `mnemonist bit-set.js` + `bit-vector.js` (copy-pasted)
`size` is never a popcount; it is maintained by comparing the word before and after each write,
which only works if both readings are unsigned. `set` and `flip` say so explicitly:
```js
newBytes = this.array[byteIndex] |= (1 << pos);
newBytes = newBytes >>> 0;                        // <-- reset() does NOT do this
if (newBytes > oldBytes) this.size++;
```
`reset` compares the **signed** value of the compound assignment against the **unsigned**
`Uint32Array` read. On any word whose bit 31 is set, the signed value is negative, so
`newBytes < oldBytes` is true whether or not the reset changed anything:
```js
var s = new BitSet(32);
s.set(31);      // size 1
s.reset(0);     // bit 0 was ALREADY clear
s.size          // 0   -- and bit 31 is still set
s.rank(32)      // 0   -- rank early-returns on size === 0, so it lies too
```
Three no-op resets give `size === -2`. **Strongest of this batch**: it is a one-token omission, the
two correct call sites are three lines away, and the consequence propagates into `rank` and
`select`, both of which bail on `size === 0`.

### B-18 — `BitSet`/`BitVector.select` does not advance its position across skipped words
`status: VERIFIED against Node 24.18.1` · `mnemonist bit-set.js` + `bit-vector.js` (copy-pasted)
```js
for (var i = 0; i < l; i++) {
  byte = this.array[i];
  if (byte === 0) continue;              // <-- p is NOT advanced by 32 here
  for (var j = 0; j < b; j++, p++) { … }
}
```
`p` only moves inside the inner loop, so every all-zero word before the answer costs the result 32.
`new BitSet(64); s.set(40)` answers `select(1) === 8`, where 40 is correct. With bits at 3 and 70 in
a `BitSet(96)`, `select(1) === 3` (right, nothing skipped) and `select(2) === 38` (wrong by 32).
Invisible upstream because both `select` tests use a length of 11 — a single word.

### B-19 — `bitwise.msb32` returns 0 for every input whose bit 31 is set
`status: VERIFIED against Node 24.18.1` · `mnemonist utils/bitwise.js`
`x |= (x >> 1)` is an **arithmetic** shift, so an input with the top bit set smears to `-1` at the
first step, and the final `x & ~(x >> 1)` is then `-1 & ~(-1)`, which is `-1 & 0`, which is `0`.
So the function reports "no bits set" for exactly the half of the 32-bit range where the answer is
most obvious: `msb32(0xFFFFFFFF) === 0`, `msb32(2**31) === 0`, `msb32(-1) === 0`. It is correct
everywhere below the sign bit, which is why nothing has noticed. `msb8` has the same shape but the
smear stops before bit 31, so it only misfires on input that is not a byte (`msb8(256) === 256`).

### B-20 — `bitwise.criticalBit32Mask`'s trailing `& 0xffffffff` undoes its own `>>> 0`
`status: VERIFIED against Node 24.18.1` · `mnemonist utils/bitwise.js`
```js
exports.criticalBit32Mask = function (a, b) {
  return (~msb32(a ^ b) >>> 0) & 0xffffffff;
};
```
The `>>> 0` produces the intended unsigned mask; `& 0xffffffff` then converts *both* operands to
signed 32-bit — `0xffffffff` is `-1` there — so the mask is an identity that re-signs the result.
`criticalBit32Mask(1, 2) === -3`, `criticalBit32Mask(0, 0) === -1`. Its byte-wide sibling
`criticalBit8Mask` ends in `& 0xff` and is correct, which makes the pair a nice illustration of the
same idiom being right at one width and wrong at another. **Low severity** — file with the others.

### B-21 — `BitVector.pop` never decrements `size` and `push(0)` never clears the slot
`status: VERIFIED against Node 24.18.1` · `mnemonist bit-vector.js`
Three defects in six lines. `push(0)` returns `++this.length` without storing anything, so a slot a
`pop` released keeps its stale `1`; `push(1)` does `this.size++` unconditionally, so a re-push over
a set bit counts it twice; and `pop` moves `length` and nothing else. Upstream's own `pop` test
walks the exact sequence and stops one assertion short:
```js
var v = new BitVector();
v.push(1); v.push(1);   // size 2
v.pop(); v.pop();       // length 0 -- size STILL 2, bits STILL set
v.push(0);              // length 1
v.get(0)                // 1, not 0     <-- the test asserts get(1) instead
v.push(1);              // size 3, with two bits actually set
```

### B-22 — `length % 32 || 32` treats a length of 0 as a full final word
`status: VERIFIED against Node 24.18.1` · `mnemonist bit-set.js` + `bit-vector.js`
Both iteration paths size the last word as `length % 32 || 32`. The `|| 32` is there for a length
that fills its last word exactly, and `0 % 32` is also falsy, so a length of **zero** over a
non-empty array yields 32 bits. `BitSet` cannot reach it — its array is empty when its length is —
but a `BitVector` can, because capacity outlives length: `new BitVector(); v.grow();` then `forEach`
calls back **32 times** on a vector of length 0.

### B-23 — `BitSet.set` accepts an index past `length` but inside the last word, and then nothing can see it
`status: VERIFIED against Node 24.18.1` · `mnemonist bit-set.js` + `bit-vector.js`
`new BitSet(10)` allocates one 32-bit word, and `set(20)` lands in it: `size === 1`, `array` is
`[1048576]`, while `rank(10) === 0`, `select(1) === undefined` and iteration yields ten zeros. So
`size` disagrees with every other view of the same set. This is the narrow survivor of the
`SparseSet` out-of-range family — **and it is worth recording that the family does NOT otherwise
recur**: `set`/`reset`/`flip` past the *array* are inert no-ops and `get` is `0`, because a
`BitSet`'s counter is derived from a before/after comparison that an `undefined` read makes false,
where `SparseSet` increments its counter unconditionally after a dropped store (B-8).

> Differential fuzzing has not run yet. Expect the best candidates to come from there, not from
> reading. Add them here with the minimised repro attached.

---

## Log

### Pre-kickoff — 2026-07-31

**Repo/track selection.** **Track G** (JS→Rust), solo. *Note: the website's track table says F is
JS→Go/Rust and G is C→Zig; the later admin FAQ says F is JS→Go and G is JS→Rust, dropping C→Zig
entirely. Planned as F for most of the day, corrected to G once the FAQ landed. Track is declared
at submission, not registration, so the cost was zero — but a good reminder that the "official"
page is not always the current source of truth.* The decisive criterion turned out not to be LOC
or difficulty but **test-corpus portability** — whether the original suite can run against a port
at all. That reframing eliminated most of the pool immediately.

**mnemonist is 15,386 LOC** (shipped source; my first `-maxdepth 2` sweep said 15,841 and swept in
`experiments/`/`docs/` — the correction was only ~625 lines, the size problem was real).
Chosen anyway because it is **perfectly modular**: 1 file per structure, 1:1 matching `test/<name>.js`,
tests importing by direct relative path, no `.mocharc`. A scoped subset port is therefore clean —
rare at this size.

**Four files gate 90% of the repo.** `obliterator/foreach` (22 dependents), `obliterator/iterator`
(18), `utils/typed-arrays` (14), `utils/iterables` (12). The dependency graph, not the LOC table,
determined the wave order.

**I mischaracterised `test/_utils.js` as a helper.** It is a real test file — 389 lines, 20
`describe` blocks, covering five utils modules. Consequence: Wave 0's utils work earns direct
test credit instead of being pure infrastructure. *Worth including in the write-up as an example
of how a wrong early assumption quietly reshapes a plan.*

**obliterator turned out to be the whole story.** Reading the source rather than inferring:
- `Iterator.prototype[Symbol.iterator] = function () { return this; }` — **self-returning, not
  restartable.** Idiomatic Rust `IntoIterator` has the wrong semantics.
- `fromSequence` is **hybrid**: length captured at creation, elements read lazily. Not snapshot,
  not live — the weirder of the two possible answers, and the same pattern recurs in every
  structure's own `values()` (`Stack`, `FixedDeque` confirmed).
- `forEach` has **5-branch dispatch where the callback's second argument changes type per branch** —
  number for sequences/iterators, *string key* for plain objects, host-defined for anything owning
  its own `.forEach` (a JS `Map` yields `(value, key)`).

**The grep that changed the architecture.** All ~26 `forEach` call sites across all 30 importing
modules are `forEach(iterable, cb)` inside `.from()` statics or iterable-accepting constructors,
on the **user-supplied argument** — never on a structure's own data. So `forEach` is a *boundary*
function. It moved out of the core entirely into the napi crate: more idiomatic, less work, and
correctly located. *A good write-up beat: one grep collapsing the ugliest item on the critical path.*

**Two-level `Symbol.iterator`.** Collection-level is a **factory** (`[...stack]` twice works);
iterator-level is **identity** (`const it = stack.values(); [...it]` twice does not). One level
apart, opposite semantics. A uniform "iterable" abstraction gets exactly one wrong.

**napi-rs already has the right semantics — measured, not assumed.** Smoke crate, napi 3.6.1,
Node 24.18.1: `c[Symbol.iterator]() === c` → `true`; first `[...c]` → `[1,2,3]`; second → `[]`;
`next(); next(); [...c]` → `[3]`. D-06 and half of D-07 need **no custom work**.
*Good beat: the FFI layer handed over the exact JS semantics that idiomatic Rust would have broken.*

**Node 26 breaks the test suite before a line of port code exists.** 26.5.1 runs fine standalone,
but mocha 9.1.3's bundled `yargs` dies: `require is not defined in ES module scope` (Node 26
ESM/CJS interop). **Node 24.18.1 is the newest that runs the upstream suite with zero deviation
from upstream devDeps.** Also: **22.23.2 segfaults on exec (exit 139)** — bad build, unrelated.
*Strong beat: "the newest runtime is not a neutral choice when your proof depends on a 2021 runner."*

**Environment archaeology.** Windows `link.exe` on PATH resolves to Git/scoop's **GNU coreutils
`link` 8.32**, shadowing MSVC's linker despite VS 2022 being installed — a cdylib build would fail
with errors that look nothing like PATH. Sidestepped by making Linux primary. Also: `rustup update
stable` hit a component conflict and left WSL's toolchain without `rustc`/`cargo` (clean reinstall
fixed it), and starting Docker Desktop put WSL into `getpwuid` failures → `E_UNEXPECTED`
(`wsl --shutdown` fixed it). **~90 minutes of pre-kickoff environment work that would otherwise
have been hour-3 hackathon work.**

**Upstream baseline:** `525 passing · 1 pending · 0 failing · 90ms` on Node 24.18.1, clean clone.
`npm install` clean, 165 packages, no native-build failures.

**Admin ruling (verbatim, see DESIGN.md header).** FFI bridge ratified; `tests/original/` +
kickoff SHA-256 named explicitly; *"unsafe at the FFI boundary is fine and expected — what counts
against you is unsafe code spread through the core port logic"* — which is exactly the crate split
we had already chosen. 1:1 native tests accepted as a fallback. **Tests now optional for
qualification**, but still the strongest proof.
*Still unanswered: repo size / scoped subset.*

---

**Second correction of the day: "no matching test file" ≠ "untested."** I had written off 1,086 LOC
across four LRU modules because none had a `test/<name>.js`. `test/lru-cache.js` requires all four
directly — 835 of those LOC are covered and scoreable. Combined with the `test/_utils.js` mistake
earlier, that is **two wrong coverage inferences in one day, both from reasoning about filenames
instead of grepping requires.** *Good write-up beat: the cheapest possible check kept beating my
structural intuition.*

**Evidence-driven resolution of the shrink-window question.** Rather than guess whether to
reproduce JS's iterator-invalidation behaviour, grepped all 41 test files for *stored* iterators —
the only sites that can observe mutation. 24 sites; every one constructs, stores, drains, asserts
`done`, with **no mutation in between**. So the cheap fallback was measured to cost nothing, which
turned a risky choice into a safe one. *Beat: how to make an architecture decision with a grep
instead of an argument.*

**Denominator honesty.** Decided to hash **all 41** upstream test files at kickoff rather than only
the in-scope subset, then report both numbers. Hashing only what we plan to run would mean choosing
our own denominator after picking modules. The timestamped full hash is proof we committed before
knowing outcomes. *Beat: subset ports have an integrity problem nobody talks about, and it has a
one-command fix.*

---

### H+2 — first real module ported (`StaticDisjointSet`)

**The rank bug has a second-order consequence that nearly bit us.** B-7 leaves non-root ranks
permanently zero, so the equal-ranks branch fires on almost every union, so one root's rank climbs
once per union — far past the `log2(size)` the array was sized for. And `ranks` is *always* a
`Uint8Array` in practice. So it **wraps**: a 300-element set ends with `ranks[0] == 43`. Node agrees
exactly. A naive `Vec<u32>` port diverges silently and no test catches it, because upstream's own
suite never builds a set that large.

*Two bugs compounding — one upstream logic error making an otherwise-unreachable overflow reachable
— is the best single argument for differential testing we have so far. Neither is visible from
reading one file.*

**Validated every case against real Node rather than reasoning about it.** All 10 scenarios matched.
That is now the working method: when a JS semantic is in question, run it, don't argue about it.

**Pinned the rank bug with a regression test** on a concrete input where it changes the elected root
(size 8; unions `(0,1) (0,2) (3,4) (1,3)` → upstream elects `3`, correct union-by-rank elects `0`).
A future "cleanup" now fails the suite instead of silently diverging.

**Process note:** the same apostrophe-quoting trap that broke `&'static str` earlier also truncated
a commit message (`find()'s` closed the outer `bash -lc '...'`). Dodged in Rust source by staging
files, forgotten for the commit body. *Small recurring tax of driving a WSL repo from a Windows
shell — the reason we moved the session into WSL.*

### H+5 — harnesses built, `StaticDisjointSet` backfilled to full DoD

**The fuzzer found nothing, and that is the interesting result.** 4.23 M operations across two
seeds, zero divergences. Expected, and worth saying out loud: **a faithful port reproduces
upstream's bugs, so differential fuzzing structurally cannot find them.** B-7 was found by reading.
What the fuzzer is actually for on a bug-for-bug port is the *opposite* direction — catching the
port drifting away from upstream, **including drifting towards correctness**.

**So the fuzzer was falsified by "fixing" B-7.** Gate 6's lesson applies to the fuzzer itself: one
that has never been observed to catch anything is a second green light, not a check. Changing
`ranks[x]` to `ranks[x_root]` in the core — the single most plausible way a future cleanup breaks
this port — was caught in **129 cases, 0.3 s**, and proptest shrank a 600-op program to three ops:
`new(23); union(10,7); union(11,7); find(10)` → upstream 11, "corrected" port 10. That seed is now
committed as a regression guard with a provenance header, because an unlabelled `cc` line in
`proptest-regressions/` would read as a real defect that was found and fixed.
*Write-up beat: the most valuable thing my differential fuzzer caught was my own code being too
correct.*

**proptest's default `max_shrink_iters` is tuned for small values and quietly gives up.** At the
default it stopped at a "minimal" 29-op program that was mostly noise, with a warning easy to miss
in the scroll. Raised to 2²², the same failure minimises to 3 meaningful ops. **The shrink budget
is the difference between a repro you can file upstream and a wall of text.**

**The benchmark's first result was too good, which was the tell.** Port won every metric on the
1e6 workload — p50, p99, RSS, startup. Against a library that is already typed-array-backed, that
should not happen, and §5.1 says so explicitly. Swept the size (200 → 5k → 65k → 1e6 → 4e6) looking
for the boundary and found it at **4e6: p99 275 ns vs 102 ns, the port 2.7× SLOWER**, while p50
stays 1.7× faster.

Cause is our own design, not the workload: `PointerVec` backs *every* logical width with a
`Vec<u32>`, so our `ranks` is 4× upstream's `Uint8Array`. At 4e6 that is 32 MB of structure vs
20 MB — exactly the 32 MB L3 boundary on this 7600X. **The port wins the median and loses the
tail, which is the inverse of the usual Rust-vs-V8 story**, and it only shows up because §5.2's
batch-level p99 exists to show it.
*Beat: "I went looking for the workload where my port loses, and the honest answer changed the
headline from 'faster' to 'faster at the median, worse at the tail'."*

**Two harness decisions that turned out to matter more than expected:**

1. **Both bench sides emit a checksum over every non-mutating op's result, and the driver refuses
   to write results unless all 20 runs agree.** Intended as cheap paranoia; it turns "same
   workload" from an assertion into a verified claim — same ops *and* same answers — and it
   incidentally re-proves the rank bug is reproduced, since a corrected port would elect different
   roots and move the checksum.
2. **Percentiles are computed once in the driver over both sides**, not twice in the two runners.
   §5.2 asks for "same percentile maths"; implementing it twice and hoping is strictly weaker.

**Cost of the persistent oracle, quantified at last:** ~23,600 op/s *including* a full
`mapping()` + `compile()` comparison after every op. At one `node` spawn per op the 120 s campaign
would have taken ~33 hours. D-23 paid for itself on the first module.

**Small trap:** JS bitwise operators produce *signed* 32-bit results, so the xorshift32 twin needs
`>>> 0` or the two streams part company within a handful of draws — silently producing two
different benchmarks. `--dump-prng` + `diff` catches it in one second; reasoning about it would
have taken longer and been less convincing.

### H+5 — the RSS lesson: a fix that worked for a reason we got wrong

**The prediction.** `PointerVec` backed every logical width with `Vec<u32>`, so at 4e6 items our
`ranks` was 16 MB where upstream's `Uint8Array` is 4 MB — 32 MB of structure against upstream's
20 MB, straddling this CPU's 32 MB L3. That was offered as the cause of a 2.7× p99 tail regression,
with a confident mechanism attached.

**The fix worked, emphatically.** Per-width backing store → p99 at `mixed-4e6` went
**275.0 → 43.6 ns/op** against upstream's 134.9. A 2.7× loss became a 3.1× win.

**The mechanism was wrong, and one number proved it.** If footprint were the cause, resident memory
should have dropped ~12 MB. `structure_rss_delta_mb` moved **12.8 → 13.0**. Nothing.

**Why.** `ranks` is `vec![0; n]`, and because of the rank bug (B-7) almost every entry is *never
written* — only roots are ever bumped. Linux does not fault in untouched zero pages, so the extra
12 MB was **never resident and never appeared in RSS in the first place**. We reasoned confidently
about memory that did not exist.

**Two generalisable lessons:**
1. **RSS measures resident, not allocated.** For zero-initialised or sparsely-written structures
   the two diverge without limit. Allocating 16 MB and touching 4 KB of it costs 4 KB of RSS.
   Any argument of the form "we allocate more, therefore we are slower" needs a residency check
   before it is believed.
2. **Check a causal story against a metric that would falsify it.** The fix and the explanation
   were bundled together, and only splitting them — "if footprint is the cause, RSS must drop" —
   exposed that one was right and the other wasn't. A correct prediction of *outcome* is not
   evidence for the predicted *mechanism*.

Current best hypothesis is address-space stride rather than resident size: at `u32` the same
indices span 4× the pages (4096 vs 1024 at 4 KB), and TLB pressure lands in the tail rather than
the median. **Unconfirmed** — needs `perf stat -e dTLB-load-misses` on both revisions. Recorded as
a hypothesis, not a finding.

**Bonus lesson from the same episode: benchmark noise is larger than it looks.** A run taken while
the machine was saturated inflated *both* sides 2–3×, and upstream's own p99 swung **102 → 135**
between two clean runs on the same host. Absolute ns/op are not comparable across runs; only the
within-run A/B comparison is sound. §5.2's interleaving requirement is what made the conclusion
survive a bad measurement rather than being poisoned by it — and the honest reporting rule is that
small ratios read as "roughly 2×", never as three significant figures.

*Write-up beats: "the fix worked and my explanation didn't" is a better story than a clean win, and
"RSS measures resident, not allocated" is the kind of thing people rediscover painfully.*

### H+5 — `cargo build` does not compile your tests

An agent died mid-task leaving uncommitted work. `cargo build --release` was **clean**. `cargo test`
had **three compile errors**, all on one line:

```rust
assert_eq!(values, PointerVec::U8(vec![300 % 256, 0]));
```

Both literals infer as `u8` from the `Vec<u8>` context, so `300` and `256` are out of range, `256`
truncates to `0`, and the modulo panics at compile time. Widened to `(300u32 % 256) as u8`, which
keeps the arithmetic documenting the truncation it tests instead of hardcoding `44`.

**The lesson is not the literal.** It is that `#[cfg(test)]` blocks are not compiled by `cargo
build`, so **a green build carries no information about whether test code even parses.** Any gate
that means to say "this compiles" must run `cargo test`, not `cargo build`. `tests/verify.sh`
already does — which is worth noting as a design choice that paid off rather than luck.

### THE NUMBER — the rigor gap, measured in one experiment
### H+? — `&self` is a promise the FFI boundary cannot keep

Found while porting `stack`/`queue`, by differentially probing the *bridge* rather than the core.
Three defects, two of them in code that was already green and one of them in a module already in
`tests/scope.txt`.

**1. `noalias` ate a mutation.** napi hands the same wrapped struct to JS as `&self` for one
method and `&mut self` for another, and JavaScript calls the second from inside a callback the
first is still running:

```js
q.forEach(function (value, i) { if (i === 0) { q.dequeue(); q.dequeue(); } });
```

Upstream yields `1, 4, undefined, undefined` — its `forEach` re-reads `this.items` every
iteration and the second dequeue rebinds it. The port yielded `1, 2, 3, 4`. rustc marks a `&T`
parameter `noalias readonly` whenever `T: Freeze`, so LLVM hoisted the read straight out of the
loop; the *same object* reported the new `offset` correctly through a separate call one line
later.

The fix is not a barrier, it is the type: a struct holding a `RefCell` inline is not `Freeze`, so
`&self` carries neither attribute. The bridges now hold `RefCell<Core*>` and release every borrow
before any JS call.

**This is also present in the `sparse-set` bridge, which is already in `tests/scope.txt`.**
Measured: a `forEach` callback that deletes yields `[1, 4, 3, 4]` against upstream's `[1, 4]`. Not
fixed in this pass — out of lane — and the same `RefCell` change fixes it.

**2. napi's iterator installs a `#.return` that latches.** `obliterator/iterator` has no `return`
and no `done` flag, so breaking out of a `for…of` leaves the cursor where it stopped and a later
`next()` resumes. napi's `#[napi(iterator)]` sets `next`/`return`/`throw` as own properties on
every instance, and its `return` writes `[[GeneratorState]] = true` **before** the Rust
`complete()` runs — so `complete` cannot prevent it and every later `next()` answered
`{done: true}`. The addon now deletes the method at load, which is exactly upstream's situation.

This **corrects a claim in `crates/mnemonist-napi/src/sparse_set.rs`'s docs** — that napi's
default `complete` is observably the same as having no `return`. It was reasoned about, not
measured, and it is wrong.

**3. `napi_create_reference` rejects primitives.** Below Node-API 10, and napi-rs 3.12 does not
export `node_api_module_get_api_version_v1`, so an addon built with it is a version-8 module
however its Cargo features are set — moving the workspace from `napi9` to `napi10` changes
nothing. Measured. Storing arbitrary JS values therefore needs an enum: references for
object/function/symbol, by value for primitives, which is observationally exact because
primitives are immutable and compared by value.

*Write-up beat: every one of these is the FFI layer being more honest than the type system. (5) in
the angle list gets stronger — "the boundary gave me the semantics idiomatic Rust would have
broken" now has a companion, "and it took the semantics `&T` would have broken away from me".*

### The pattern these three share

`sparse-map`'s B-11 is a plain correctness bug: `delete` moves the last *member* into the hole but
not the last *value*, so `set(3,'a') set(4,'b') set(5,'c'); delete(3)` leaves `get(5) === 'a'`.
In-range input, no edge case, straightforwardly wrong.

Sabotage the port to **fix** it — making our port more correct than upstream, therefore wrong —
and measure what each layer of evidence says:

| Evidence layer | Verdict on a port that diverges from upstream |
|---|---|
| **The upstream mocha suite, unmodified** | **9 passing / 0 failing — sees nothing** |
| Our native Rust tests | **4 red** |
| The differential fuzzer | **caught in 3.0 s**, shrunk to 3 operations |

**That is the entire thesis of this event in one pair of numbers.** The original test suite — the
artifact the 40% category is built on — cannot detect a behavioural divergence in a module it
nominally covers, because it only ever deletes from a one-element map where the swap is a
self-assignment. The 30% and 20% categories are where the detection actually lives.

Use this as the headline for the write-up and quote it in the README. It is far stronger than any
"we passed N tests" claim, because it is evidence about *evidence*.

### The pattern these five share

Five separate incidents, all the same meta-failure — *the check I ran did not check what I
believed it checked*:

| Incident | What it appeared to verify | What it actually verified |
|---|---|---|
| Falsification that stayed green | that the suite exercises Rust | nothing — it sabotaged a branch the test never takes |
| RSS as evidence for the L3 hypothesis | that the footprint shrank | nothing — the pages were never resident to begin with |
| `cargo build` clean | that the code compiles | that *non-test* code compiles |
| `cases=16666` in a fuzz campaign | 16,666 distinct programs | 32 programs, plus two saved seeds re-run ~8,300 times each |
| `cargo clippy \| tail -5` exit 0 | that clippy passed | that **`tail`** succeeded — a pipeline's status is its *last* command's |
| `sparse-set` bridge green on `forEach` | that the loop re-reads `size` | nothing — the read was hoisted; no test or fuzz case mutates from a bridge callback |
| A cursor's `complete()` returning `None` | that `break` leaves the walk resumable | the opposite — napi had already latched `[[GeneratorState]]` |
| `cargo build … \| tail -1` before a fuzz run | that the sabotage was compiled in | tail's exit status; the run used a stale binary and reported "clean" |

Each one produced a **confident green signal that was empty**, and in every case the failure was
invisible until something forced the question "what would this look like if it were broken?"

*This is the strongest write-up thread we have. It is the rigor gap in miniature — not "the port
was wrong" but "the evidence that the port was right did not say what we thought". A hackathon
premised on proving behavioural equivalence is exactly the place to argue that verification needs
verifying.*

## Write-up angle candidates

1. **"The rigor gap, measured."** The event's own thesis, tested: what differential fuzzing
   actually finds in a well-tested JS library. Strongest if the fuzzer produces real divergences.
2. **"Your iterator semantics are load-bearing."** Self-returning cursors, hybrid live/snapshot
   capture, two-level `Symbol.iterator` — the parts of JS that idiomatic Rust silently changes.
   *Most reusable by other people; probably the best insight-per-word.*
3. **"Node 26 broke my test suite before I wrote any code."** Short, punchy, concrete.
4. **"One grep moved the hardest code out of my core."** Boundary-vs-core as a porting principle.
5. **"The FFI layer gave me the semantics idiomatic Rust would have broken."** Counterintuitive,
   and it inverts the usual framing of FFI as a necessary evil.

Pick after the event based on what the fuzzer actually found. **(2) and (5) are strong regardless
of outcome; (1) depends on results.**

### B-31 (OUR BUG, not upstream's) — `&self` on a `Freeze` type is `noalias readonly`, and LLVM used it

`status: VERIFIED, sparse-set DESCOPED pending fix` · found by the forEach agent probing its own bridge

A napi method taking `&self` on a type that is `Freeze` (no interior mutability) compiles to a
`noalias readonly` pointer. LLVM is then entitled to hoist reads across the call — and does. When a
JS `forEach` callback re-enters Rust and mutates the collection, the port keeps iterating the
hoisted snapshot while the same object reports its updated state one line later.

Reproduced on `sparse-set`, which was **already in `tests/scope.txt`**:

```js
const s = new SparseSet(8); [1,2,3,4].forEach(m => s.add(m));
const seen = []; s.forEach(m => { seen.push(m); s.delete(m); });
// upstream [1,2]   port [1,2,3,4]
```

**Fix:** type it honestly — `RefCell<Core>` is not `Freeze`, so the aliasing assumption disappears.
Already applied in the forEach agent's branch for `stack`/`queue`; `sparse-set`'s bridge on main
still needs it.

**Structural exposure:** the defect needs JS to re-enter Rust mid-operation, so only bridges with a
callback-taking method are at risk. `sparse-set` has 8; `static-disjoint-set` has **0** and is
immune. Check this count before scoping any future module.

#### Why 2.94M fuzz ops missed it — a grammar gap, not bad luck

`sparse-set`'s grammar had `$iter`/`$next`/`$spread` interleaved with mutation, which is exactly
D-21's requirement, **but no `forEach` op at all** — and certainly not one whose callback mutates.
The fuzzer could not express the program that breaks it.

*The lesson generalises past this bug: an op alphabet that omits a method omits every bug reachable
only through it, and a clean campaign then reads as coverage it never had. Every module with a
callback-taking method needs a mutating-callback op in its grammar.*

#### And a sixth entry for the empty-green-signal table

`sparse-set` sat in `scope.txt` with all ten gates green — original suite passing, 2.94M fuzz ops,
zero divergences, benchmarks, divergence doc — while carrying a live behavioural divergence. **The
gates were all true and the conclusion was still wrong**, because the fuzz grammar defined what
could be found. Found only because a different agent, working on a different module, probed the
same bridge pattern from a different angle.

*Strongest argument yet for the write-up: passing your own verification is not the same as being
correct, and the only thing that caught this was a second, independent look.*

---

## sort — upstream bug candidates (B-80, B-81)

Both are the same defect wearing different clothes: **shared mutable module state inside a function
that can be re-entered through a comparison.** Both were found by reading, not by fuzzing, and the
reason is worth recording — see the note at the end.

### B-80 — `sort/insertion.js` declares its loop counter as a GLOBAL, and re-entry corrupts the sort
`status: VERIFIED against Node 24.18.1` · `mnemonist sort/insertion.js`

Both exported functions open with an undeclared assignment:

```js
function inplaceInsertionSort(array, lo, hi) {
  i = lo + 1;          // no var, no let
  var j, k;
```

The file is sloppy-mode CommonJS, so `i` is `globalThis.i` and every call in the realm shares one.
After a single `inplaceInsertionSort([3, 1, 2], 0, 3)`, `global.i` is `3`.

That alone is only untidy. The bug is that `>` invokes `valueOf`, so an element can re-enter the
sorter mid-comparison and the inner call leaves the outer call's counter wherever it finished:

```js
function reentrant(v, payload) {
  return {valueOf: function () { if (payload) { payload(); payload = null; } return v; }};
}
var inner = [3, 1, 2];
var outer = [reentrant(5),
             reentrant(1, function () { insertion.inplaceInsertionSort(inner, 0, 3); }),
             reentrant(3), reentrant(2)];

insertion.inplaceInsertionSort(outer, 0, 4);
outer.map(Number);   // [1, 5, 3, 2]   -- expected [1, 2, 3, 5]
```

`inplaceInsertionSortIndices` has the identical line and shares the same `i`, so the two corrupt
each other as readily as each corrupts itself. Under `'use strict'` the file would throw
`ReferenceError` outright.

**Strong candidate** — a one-word fix (`var i = lo + 1`) for a wrong-answer bug, plus a global leak.

### B-81 — `sort/quick.js`'s partition stack is module state, shared by all four sorts
`status: VERIFIED against Node 24.18.1` · `mnemonist sort/quick.js`

```js
var LOS = new Float64Array(64),
    HIS = new Float64Array(64);
```

Allocated once at module scope and used by `inplaceQuickSort` *and* `inplaceQuickSortIndices`. Here
`i` **is** a proper local, which makes the failure subtler than B-80's: the outer call's index keeps
pointing into a stack the inner call has overwritten, so the outer call resumes partitioning ranges
that no longer describe its own array. Measured on a 40-element array whose first compared element
re-enters:

```js
quick.inplaceQuickSort(arr, 0, 40);
// [0,1,4,7,10,13,16,...,35,37,38,32,29,26,...,6,3]   -- 38 of 40 elements out of order
```

The allocation is presumably deliberate — avoiding two `Float64Array(64)` per call — so the fix is
not "make them locals" but "make them locals, or reference-count re-entry". Worth reporting as a
correctness note rather than a style one.

**Strong candidate**, with the same caveat as B-80: both need an element that runs JavaScript during
a comparison, which is legal and which mnemonist's own callers never do.

#### Why the fuzzer could not have found either

`crates/mnemonist-napi/src/sort.rs` accepts numbers and nothing else (D-80), so no user code can run
during a comparison and the port cannot enter the regime at all. The port has locals where upstream
has module state, and with numeric elements the two are indistinguishable.

*This is the mirror image of the B-31 lesson.* There, a grammar that omitted a method omitted every
bug reachable only through it. Here the **bridge's accepted input domain** omits them, one layer
earlier, and no grammar over that domain could have expressed the program. Both come out the same
way: a clean campaign is coverage of what the harness can express, and saying which is part of
reporting the result.

---

## set — no upstream bugs, and that is the finding

`set.js` was read statement by statement, as `sort/` was, and produced **nothing to file**. Worth
recording rather than omitting: 356 lines with no shared mutable state, no typed arrays, no index
arithmetic and no re-entrancy. It is the cleanest upstream file this port has touched.

Three things that read like bugs and are not, all checked against Node 24.18.1:

* `jaccard(new Set(), new Set())` is `0`, not `NaN` — the `if (I === 0) return 0` guard fires
  before the division. Same for `overlap`. A convention.
* `intersection`'s result ORDER depends on which argument happened to be smallest, because it
  iterates the smallest one. Surprising, and correct: it falls out of the optimisation.
  `intersection(new Set([3,2,1]), new Set([1,2]))` is `[1, 2]`.
* `difference(A, new Set())` returns `new Set(A)` — a copy, not `A`. Deliberate.

**A near-miss in OUR port, caught by a boundary spec rather than by the original suite.** The first
bridge sketch for the four mutating functions was read-A, compute, `A.clear()`, re-add. It passes
all sixteen upstream blocks and is observably wrong, because a JS `Set` iterator is live and
`clear()` does not detach it:

```js
var A = new Set([1, 2]); var it = A.values(); it.next();
functions.add(A, new Set([2, 3]));
Array.from(it);   // upstream [2, 3];  clear-and-rebuild [1, 2, 3]
```

*The lesson is the one B-31 taught from the other side: the original suite's assertions define what
it can catch, and a bridge decision that is invisible to all of them still needs its own test. Here
the fix was to have core return the trace of `add`/`delete` calls and replay it, so the caller's
`Set` experiences what upstream's experiences, call for call.*

**A second, smaller correction, recorded because it is a documentation bug this project's own
process caught.** An early draft of `disjunct`'s doc claimed its add-before-delete write order was
what fixes the result's ordering. Sabotaging exactly that changed nothing — neither `test/set.js`
nor the boundary specs went red. The write order is unobservable; what is load-bearing is that the
`!A.has` test runs before any deletion. The claim was withdrawn in the code and the real property
pinned by its own test.

*Gate 6's discipline applied to prose: a sabotage that stays green does not vindicate the code, it
falsifies the sentence explaining it.*

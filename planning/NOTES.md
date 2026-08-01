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

### B-95 — `binary-search.lowerBoundIndices` defaults `hi` from the wrong array
`status: VERIFIED against Node 24.18.1` · `mnemonist utils/binary-search.js`
Every other reference in the function is to `indices`; the `hi` fallback is `array.length`. When
`indices` is shorter than `array` — the normal shape for a partial argsort, which is what an index
array is for — the walk runs off the end of `indices`, reads `undefined`, indexes `array[undefined]`
for another `undefined`, fails `value <= undefined`, and moves right.

```js
> require('mnemonist/utils/binary-search').lowerBoundIndices([0,1,2,3,4,5,6,7], [0,1], 1)
8            // indices has 2 entries; 8 is a position in neither array
> require('mnemonist/utils/binary-search').lowerBoundIndices([0,1,2,3,4,5,6,7], [0,1], 1, 0, 2)
1            // what the caller meant
```

Latent in the shipped library: `vp-tree.js`, the only caller, always passes an `indices` the same
length as `array`. Reachable from the public API, since `utils/` is `require`-able. Reproduced;
see `docs/modules/utils-binary-search.md`.

### B-96 — `binary-search.search` with an out-of-range `hi` reports a match at a hole
`status: VERIFIED against Node 24.18.1` · `mnemonist utils/binary-search.js`
`undefined` loses **both** comparisons, so `current > value` and `current < value` are each false
and the `else` arm — which means "equal" — returns the midpoint.

```js
> require('mnemonist/utils/binary-search').search([1, 2, 3], 9, 0, 100)
49
```

Worth recording alongside it: the two bound functions react to the same `undefined` in *opposite*
directions, `lowerBound(...)` walking right to `100` and `upperBound(...)` left to `3`. So there is
no single "undefined sorts high" rule to reason from — each call site has to be checked. The same
`undefined`-loses-both rule makes `search([NaN, NaN, NaN], 1)` return `1`. Reproduced.

### B-92 — `hash-tables.linearProbing.get`/`has`/`set` loop forever on a zero-length table
`status: VERIFIED against Node 24.18.1` · `mnemonist utils/hash-tables.js`
`i %= n` with `n === 0` is `NaN`. `keys[NaN]` is `undefined`, which is neither the key nor `0`, so
neither exit fires; and the "full turn" guard `i === j` can never be true, because `NaN !== NaN`.
`while (true)` then never exits.

```console
$ timeout 5 node -e "require('mnemonist/utils/hash-tables').linearProbing.get(
    require('mnemonist/utils/hash-tables').hashes.jenkinsInt32,
    new Uint32Array(0), new Uint32Array(0), 1)"
$ echo $?
124
```

NOT reproduced — hanging a `cargo test` or a fuzz campaign is not a behaviour worth porting. The
port guards all three entry points; see D-45 in `docs/modules/utils-hash-tables.md`.

### B-94 — `hash-tables`: the key `0` occupies a slot that still reads as empty
`status: VERIFIED against Node 24.18.1` · `mnemonist utils/hash-tables.js`
`0` is the empty sentinel *and* an ordinary `Uint32Array` value. The sequence is subtler than "key
0 cannot be stored":

```js
var lp = require('mnemonist/utils/hash-tables').linearProbing;
var keys = new Uint32Array(4), values = new Uint32Array(4), h = function () { return 0; };

lp.set(h, keys, values, 0, 42);
Array.from(keys);                  // [0, 0, 0, 0] -- indistinguishable from empty
lp.get(h, keys, values, 0);        // 42  -- `c === key` is tested BEFORE `c === 0`
lp.set(h, keys, values, 5, 43);    // slot 0 looks free, so this OVERWRITES
Array.from(keys);                  // [5, 0, 0, 0]
lp.get(h, keys, values, 0);        // 0  -- the 42 is gone, silently
```

So the entry is readable right up until something collides with it, then vanishes with no error.
Reproduced exactly, including the readable-until-it-isn't part.

### B-90 — `SuffixArray`'s radix sort silently narrows to 8 bits
`status: VERIFIED against Node 24.18.1` · `mnemonist suffix-array.js`
`sort()` picks its radix width from `j = Math.max(string[array[i] + offset], j)`, and that index runs
past the padded sequence for `offset` 1 and 2 — `convert()` pads with `length % 3` zeros, which is
not enough. The read is `undefined`, `Math.max(undefined, j)` is `NaN`, every shift of `NaN` is `0`,
so `j >> 24 && 32 || j >> 16 && 24 || j >> 8 && 16 || 8` falls through to **8**. The sort then
compares only the low byte of each 16-bit symbol.

Mechanism confirmed by instrumenting upstream's own `sort`, not inferred: for a 15-symbol input the
offset-2 and offset-1 passes read index 16 and report `bits = 8`, while the offset-0 pass reads in
range and reports `bits = 16` for a maximum symbol of 513.

```js
> new (require('mnemonist/suffix-array'))('ĀĀĀĀȁĀĀȁȁȁȁȁĀȁȁ').array
[0,1,2,5,3,12,6, 4,14,11,10,13,9,8,7]     // upstream
[0,1,2,5,3,12,6,14, 4,11,13,10,9,8,7]     // correct
```

Two transpositions rather than a scrambling, so a spot-check of the first entries looks fine. Any
alphabet where two symbols share a low byte is affected, including every character at or above
U+0100 whose low byte collides with the `0` padding. Measured: **81% wrong** over 10,000 random
inputs of length 1..30 drawn from `{'A', 'Ł'}` at `length % 3 == 0`. Pure ASCII is unaffected.
Reproduced; see `docs/modules/suffix-array.md`.

### B-91 — `SuffixArray` loses the DC3 sentinel when `length % 3 === 1`
`status: VERIFIED against Node 24.18.1` · `mnemonist suffix-array.js`
The reduced string DC3 recurses on is the ≡1 ranks concatenated with the ≡2 ranks, which is only
sound if the first group ends in a symbol nothing else can equal. `al = (2 * l / 3) | 0` omits the
≡1 position that would have carried it when `l % 3 === 1`, so the two halves run together and, once
the recursion fires (i.e. once a triple repeats), the answer is wrong.

```js
> new (require('mnemonist/suffix-array'))('aaaaaaa').array
[6,5,3,0,2,4,1]      // correct: [6,5,4,3,2,1,0]
```

Exhaustively over binary strings, failures occur at lengths **7, 10, 13 and 16** and at no other
length up to 16 — all ≡ 1 (mod 3); 4 is clean only because it is too short to recurse. The rule
applies at every recursion level, which is why the occasional length ≡ 2 (mod 3) also fails: its own
`al` is ≡ 1 (mod 3). Measured: **12% wrong** over 10,000 random 3-letter inputs of length 1..30 at
`length % 3 == 1`, 0% at the other two residues.

Upstream's own suite contains a length-22 input, which *is* ≡ 1 (mod 3), and passes — only because
`'This is a long string.'` has no repeated trigram, so `j === al` and the recursion never runs. The
suite is one repeated trigram away from having caught this.

Distinct from the module's own `it.skip('should work with int values (issue #196)')`, which is about
the token-case sentinel and needs token input; both of these fire on plain strings. Reproduced.

### B-93 — `murmurhash3`'s `sum32` is not a 32-bit adder, and a swapped constant hides it
`status: VERIFIED against Node 24.18.1` · `mnemonist utils/murmurhash3.js`
```js
function sum32(a, b) {
  return (a & 0xffff) + (b >>> 16) + (((a >>> 16) + b & 0xffff) << 16) & 0xffffffff;
}
```
The correct form takes `b & 0xffff` for the low half and `b >>> 16` for the high half. This one has
them the wrong way round in **both** places, so it adds `b`'s high half to `a`'s low half and `b`'s
low half to `a`'s high half. `sum32(1, 1)` is **65537**.

It is called exactly once, with `n = 0x6b64e654` — MurmurHash3's published `0xe6546b64` with its
halves swapped. The two errors cancel exactly: `sum32(hash, 0x6b64e654) === (hash + 0xe6546b64) mod
2^32` for every 32-bit `hash`, checked over 200,000 random inputs against BigInt arithmetic.

So the digest is correct, the helper is wrong, and the only thing holding them together is a
constant nobody would recognise as a typo. Anyone reusing `sum32` — it looks entirely general — gets
nonsense; anyone "correcting" `n` to the published constant breaks every filter the library has ever
produced.

Demonstrated end to end through the original suite as a control on gate 6: replacing
`sum32(hash, N)` with `hash + 0xe6546b64` leaves all six `test/bloom-filter.js` cases green, while
replacing it with `hash + N` turns two of them red. Reproduced, with both halves pinned by tests.

### B-97 — `BloomFilter` with zero hash functions answers `true` to everything
`status: VERIFIED against Node 24.18.1` · `mnemonist bloom-filter.js`
`hashFunctions = (length * 8 / capacity * Math.LN2) | 0`, unchecked. When it truncates to `0`, `add`
writes no bits and `test` returns `true` **vacuously** — the loop it would have returned `false`
from never runs.

```js
> var f = new BloomFilter(0.5);      // passes every validation upstream has
> f.hashFunctions                    // 0
> f.test('anything')                 // true
> f.test('anything else')            // true
```

`0.5` gets through because the check is `typeof capacity === 'number' && capacity > 0`, despite the
error message beside it saying "positive **integer**". `{capacity: 10, errorRate: 0.5}` reaches the
same state with a **non-empty** `data`, so this is not merely "an empty filter": the bit array
exists, is all zeros, and every query says yes. Reproduced.

### B-98 — every non-string item hashes identically
`status: VERIFIED against Node 24.18.1` · `mnemonist bloom-filter.js`
`stringToByteArray` does `new Uint16Array(string.length)`. On a number, `.length` is `undefined`,
the typed array is **empty**, and the loop never runs — so the item hashes as the empty sequence,
which is the same sequence `''` hashes.

```js
> var f = new BloomFilter(3);
> f.add(42);
> f.test(7)       // true
> f.test(true)    // true
> f.test('')      // true
```

A filter of numbers reports every number, every boolean and the empty string as present. The
neighbours are inconsistent rather than uniformly permissive, which is what makes it a bug and not a
coercion policy: `add(null)`/`add(undefined)` throw a `TypeError` from the property read,
`add(['a'])` throws `string.charCodeAt is not a function`, and `add(new String('hello'))` works and
equals `add('hello')`. Reproduced, all four cases.

### B-99 — an `errorRate` above 1 is a raw `RangeError`, but only for a large enough capacity
`status: VERIFIED against Node 24.18.1` · `mnemonist bloom-filter.js`
`Math.log` of anything above 1 is positive, so `bits` goes negative and the allocation throws:

```js
> new BloomFilter({capacity: 50, errorRate: 100})
RangeError: Invalid typed array length: -59
> new BloomFilter({capacity: 50, errorRate: 3})
RangeError: Invalid typed array length: -14
> new BloomFilter({capacity: 5, errorRate: 2})     // no error at all
BloomFilter { capacity: 5, errorRate: 2, hashFunctions: 0, data: Uint8Array(0) [] }
```

The third case is the interesting one: `(-7.2 / 8) | 0` truncates to `0`, so the *same* invalid
option gives a silent B-97 always-true filter instead of an error, and which of the two you get
depends on the capacity. Neither is the module's own error message, and `errorRate` is the one
option upstream believes it validates. Reproduced, including the split.

> Differential fuzzing has not run yet. Expect the best candidates to come from there, not from
> reading. Add them here with the minimised repro attached.

### B-100 — `StaticIntervalTree` crashes on zero intervals with an unrelated `TypeError`
`status: VERIFIED against Node 24.18.1` · `mnemonist static-interval-tree.js`
`buildBST` is called unconditionally from the constructor, even when `length === 0`. With
`high = length - 1 = -1`, `mid = (0 + (-1 - 0) / 2) | 0` truncates to `0`, so
`current = sortedIndices[0]` reads one past the end of a **zero-length** typed array
(`undefined`). The very next line, `intervals[current][1]`, indexes `intervals` with the
property name `"undefined"` and throws reading `1` off of `undefined`:

```js
> new StaticIntervalTree([])
TypeError: Cannot read properties of undefined (reading '1')
```

There is no guard anywhere upstream that catches a zero-length `intervals` before this point —
the crash comes from three levels down inside `buildBST`, with a message that says nothing about
empty input. Reproduced by [`Error::EmptyIntervals`] in
`crates/mnemonist-core/src/structures/static_interval_tree.rs`, which raises the *outcome*
(construction fails) without attempting the *mechanism* (a Rust panic unwinding across the napi
boundary would be worse than the JS exception it stands in for — napi does not `catch_unwind` a
sync call). See `docs/modules/static-interval-tree.md`.

### B-101 — `Vector.get`/`set` admit `index === length`, one past the last element
`status: VERIFIED against Node 24.18.1` · `mnemonist vector.js`
Both bounds guards are `<`, not `<=`:

```js
Vector.prototype.set = function(index, value) {
  if (this.length < index) throw new Error('...index out of bounds.');
  this.array[index] = value;
  return this;
};
Vector.prototype.get = function(index) {
  if (this.length < index) return undefined;
  return this.array[index];
};
```

So `get(length)`/`set(length, v)` are **admitted** rather than refused — one index past the last
element `push` has ever placed, landing in the capacity region instead of the array's logical
extent:

```js
var v = new Vector(Uint8Array, 5);   // length 0, capacity 5
v.set(0, 42);                        // 0 < 0 is false: WRITES. length stays 0.
v.get(0) === 42
```

`set(length, v)` does **not** advance `length`, so it writes into the vector without growing it,
silently. `test/vector.js` never exercises this: every `set`/`get` in the file is either well
inside the current length or far enough outside to hit the ordinary "out of bounds" throw — the
exact boundary is never probed. Reproduced by [`Vector::get`]/[`Vector::set`] in
`crates/mnemonist-core/src/structures/vector.rs`, which compare against `length` with the same
`<` upstream uses; see `docs/modules/vector.md`.

### B-102 — a `Vector`'s growth carries a popped slot's stale data forward, and B-101 keeps it reachable
`status: VERIFIED against Node 24.18.1` · `mnemonist vector.js`
`pop()` never clears the slot it releases — `return this.array[--this.length];` is a read, not a
write — so the region `length..capacity` can hold stale data from an earlier, larger `length`.
Growth (`reallocate`, when growing) copies the **whole old array**, not just up to `length`:

```js
if (typed.isTypedArray(this.array))
  this.array.set(oldArray, 0);   // the WHOLE old array, capacity included
```

So a value a caller has already popped survives a subsequent grow at the same position, and B-101's
`index === length` admission keeps it reachable afterwards:

```js
var v = new Vector(Uint8Array, 2);
v.push(9); v.push(8);   // array [9, 8], length 2
v.pop();                // length 1, array UNCHANGED: [9, 8]
v.reallocate(4);        // array [9, 8, 0, 0] -- the 8 survived the copy
v.get(1) === 8          // length(1) < index(1) is false: reads the stale 8
```

Neither defect alone reaches this state: without B-101's admission the stale slot would be
unreadable, and without the whole-capacity copy a grow would zero it. `test/vector.js`'s own
`pop`/`push`/`reallocate` tests never probe the boundary slot after a pop-then-grow, so the
compounding is untested upstream. Reproduced by `Storage::grown` in
`crates/mnemonist-core/src/structures/vector.rs`, which bulk-copies the old capacity rather than
reaching for the "tidier" copy-up-to-length a hand-written port would pick; see
`docs/modules/vector.md`.

### T2 — comparator callbacks (`heap`, `fixed-reverse-heap`, `utils/comparators`)

All ten below were found by reading the two files statement by statement and confirming each
against Node 24.18.1 (`bench/upstream/`, the pinned source). Every one is pinned by an assertion in
`tests/boundary/heap.js`, which passes unchanged when re-pointed at upstream.

### B-70 — a comparator that throws leaves `size` one behind `items.length`, permanently

`status: verified against Node 24.18.1` · `heap.js` · found by reading, pinned by
`tests/boundary/heap.js`

`Heap.prototype.push` is

```js
push(this.comparator, this.items, item);   // heap.push(item) FIRST, then sift
return ++this.size;                        // never reached if the sift throws
```

and the raw `push` grows the array *before* it sifts. There is no `try`/`finally` anywhere in
`heap.js`, so a comparator that throws on its first comparison leaves the element in the array and
`this.size` uncounted — and nothing ever reconciles them:

```js
var armed = false;
var heap = new Heap(function (a, b) { if (armed) throw new Error('boom'); return a < b ? -1 : a > b ? 1 : 0; });
heap.push(1); armed = true;
try { heap.push(2); } catch (e) {}
heap.size          // 1
heap.items.length  // 2
heap.pop()         // 1, and size drops to 0 with [2] still in the array
```

The two quantities disagree forever after. Every later `pop` reports one fewer than it removes, and
`#.consume` — which drains `items`, not `size` — returns more elements than the heap claims to
hold. Reproduced exactly; a port that pushed only after a successful sift would be *more* correct
and therefore wrong.

### B-71 — `nsmallest`/`nlargest` with `n === 1` answer with the `Infinity` sentinel itself

`status: verified against Node 24.18.1` · `heap.js`

Both fast paths open with `var min = Infinity` (respectively `-Infinity`) used as "nothing seen
yet", and neither checks afterwards whether anything was seen:

```js
Heap.nsmallest(1, [])                  // [Infinity]
Heap.nlargest(1, [])                   // [-Infinity]
Heap.nsmallest(1, new Set())           // [Infinity]
Heap.nsmallest(1, new Uint8Array(0))   // Uint8Array [0]  ← the sentinel, narrowed
Heap.nsmallest(2, [])                  // []              ← every other n is fine
```

The typed-array case is the sharpest: `new iterable.constructor(1)` then `result[0] = Infinity`
stores `0`, so an empty source answers with a plausible-looking element. Only `n === 1` is
affected, because every other `n` goes through the bounded-heap path, which has no sentinel.

### B-72 — the same sentinel is a real value, so an `Infinity` element resets it

`status: verified against Node 24.18.1` · `heap.js`

The test is `if (min === Infinity || compare(v, min) < 0)`, and `min` holds a real element after
the first iteration. So an element that *is* `Infinity` makes the identity test true again and the
**next** element replaces it unconditionally, whatever the comparator says:

```js
var descending = function (a, b) { return a < b ? 1 : a > b ? -1 : 0; };
Heap.nsmallest(descending, 1, [Infinity, 5])   // [5]        — wrong, Infinity is "smallest" here
Heap.nsmallest(descending, 2, [Infinity, 5])   // [Infinity, 5]  — one n up, the general path disagrees
Heap.nlargest(descending, 1, [-Infinity, -5])  // [-5]       — the mirror
```

Two adjacent `n` values give contradictory answers on the same input, which is the tell. Harmless
under the default comparator, where `Infinity` really is the largest thing; visible the moment a
custom comparator disagrees with `<`. Note the interaction with B-71: they are the same line, and
the empty-source case is the degenerate instance of this one.

### B-73 — `FixedReverseHeap`'s capacity guard is `&&` where `||` was meant

`status: verified against Node 24.18.1` · `fixed-reverse-heap.js`

```js
if (typeof capacity !== 'number' && capacity <= 0)
  throw new Error('mnemonist/FixedReverseHeap.constructor: capacity should be a number > 0.');
```

For any number the first half is false and the `&&` short-circuits, so the guard **cannot fire for
the very inputs it names**. `new FixedReverseHeap(Array, 0)` is accepted and then discards every
push in silence — `push` returns `0`, `size` stays `0`, `consume()` is `[]`. The only way to reach
the throw is a non-number that coerces to `<= 0`, e.g. `null`.

Two second-order notes. `new FixedReverseHeap(Array, -1)` *does* throw, but with `Array`'s own
`RangeError: Invalid array length` — because `this.items = new ArrayClass(capacity)` runs **before**
either guard. And the message the guard would have produced never appears for any negative number
at all.

### B-74 — `FixedReverseHeap#clear` leaves `items`, so `peek()` answers a discarded item

`status: verified against Node 24.18.1` · `fixed-reverse-heap.js`

```js
FixedReverseHeap.prototype.clear = function () { this.size = 0; };
FixedReverseHeap.prototype.peek  = function () { return this.items[0]; };
```

`clear` resets the count and nothing else, while `peek` reads the array directly and does not
consult `size`. So a cleared heap still reports a root:

```js
var heap = new FixedReverseHeap(Array, 3);
heap.push(45); heap.push(12); heap.push(46);
heap.clear();
heap.size      // 0
heap.peek()    // 46   ← an item that is no longer in the heap
heap.consume() // []   ← and consume, which slices to size, agrees it is gone
```

`consume` and `toArray` both slice to `size`, so the stale contents are invisible to them, which is
why the bug is latent. Upstream's own test calls `clear()` and then only ever `push`es again.

### B-75 — `MaxHeap.prototype = Heap.prototype`, so `instanceof` cannot tell them apart

`status: verified against Node 24.18.1` · `heap.js`

The line is upstream's, one statement after `MaxHeap`'s body:

```js
MaxHeap.prototype = Heap.prototype;
```

which shares the object rather than deriving from it. Consequences, all measured:

```js
MaxHeap.prototype === Heap.prototype   // true
new Heap()    instanceof MaxHeap       // true   ← a MIN heap passes a MaxHeap type check
new MaxHeap() instanceof Heap          // true
new MaxHeap().constructor.name         // 'Heap'
```

A `MaxHeap` also inherits `Heap.prototype.constructor`, so anything reconstructing by
`new x.constructor(...)` silently turns a max heap into a min heap. There is no way to distinguish
the two at runtime except by behaviour.

### B-76 — nothing stops a comparator from mutating the heap it is comparing

`status: verified against Node 24.18.1` · `heap.js` · not a defect in isolation; recorded because
it defines the hazard tier T2 exists for

The comparator is an arbitrary callback invoked from inside a sift, and both the heap and the
comparator are reachable from whatever scope built them. Three distinct shapes, all reproduced:

```js
// (a) grows the array the sift is walking
var budget = 2;
var heap = new Heap(function (a, b) { if (budget-- > 0) heap.push(99); return ascending(a, b); });
heap.push(5); heap.push(4); heap.push(3);
heap.items   // [3, 4, 99, 99, 5]   size 5

// (b) shrinks it, so the walk reads past its own frozen endIndex
// (c) REBINDS it: heap.clear() installs a new array and the sift finishes into the detached one
var cleared = false;
var heap = new Heap(function (a, b) { if (!cleared) { cleared = true; heap.clear(); } return ascending(a, b); });
heap.push(5); heap.push(4);
heap.items   // []     the sift's writes went to the old array
heap.size    // 1      because ++this.size ran on the zero clear had just written
```

Upstream has no defence and no error path; whatever the array looks like afterwards is the answer.
The reason this is written down as a bug candidate rather than as a curiosity is that it is a
*porting constraint*: an implementation whose algorithms take `&mut Vec<T>` cannot express (a) or
(b) at all, and one that models `items` as a `Vec` rather than as a reference answers (c)
identically to (b) — which is D-41's collapse, one module further on.

### B-77 — `#.consume` zeroes `size` first, so a throwing comparator strands the items

`status: verified against Node 24.18.1` · `heap.js`

```js
Heap.prototype.consume = function () {
  this.size = 0;                                 // FIRST
  return consume(this.comparator, this.items);   // …then the comparisons
};
```

A comparator that throws part-way leaves a heap reporting empty and holding elements:

```js
heap.push(3); heap.push(1); heap.push(2); armed = true;
try { heap.consume(); } catch (e) {}
heap.size    // 0
heap.items   // [3, 2]
```

Same family as B-70 and the mirror image of it: there the count lags the array, here it leads.

### B-78 — a comparator's return value is coerced, never checked

`status: verified against Node 24.18.1` · `heap.js`

Upstream never inspects the type of what a comparator returns; it writes `< 0`, `> 0` and `>= 0`
against it. So anything whose `ToNumber` is `NaN` reports "equal" for every pair and the heap
degenerates silently rather than raising:

```js
new Heap(function () { return 'x';  })   // every comparison NaN → toArray() is insertion order
new Heap(function () { return 0.5;  })   // fractional counts as "greater"
new Heap(function () { return -1n;  })   // a BigInt WORKS — `-1n < 0` is true
```

The BigInt case is the interesting one and it is not a rounding error: `ToNumber(-1n)` throws a
`TypeError`, but the relational operators use `ToNumeric`, which does not. A port that coerced the
result with `Number()` before comparing would throw where upstream sorts. A comparator returning a
`Symbol` *does* throw, in both.

### B-79 — a falsy comparator argument takes the default silently

`status: verified against Node 24.18.1` · `heap.js`, `fixed-reverse-heap.js`

```js
this.comparator = comparator || DEFAULT_COMPARATOR;
if (typeof this.comparator !== 'function') throw new Error('… should be a function.');
```

The `||` runs first, so the type check only ever sees a *truthy* non-function. `new Heap(0)`,
`new Heap('')`, `new Heap(NaN)` and `new Heap(null)` are all accepted as "use the default", while
`new Heap('test')`, `new Heap({})` and `new Heap([])` throw. `test/heap.js` asserts the second half
and not the first. Minor, but it is the reason the port cannot implement the guard as a plain
`Option<Function>`: an explicit falsy argument and an omitted one must behave alike, and an explicit
truthy non-function must not.



### Not allocated a `B-` number — `Heap.nsmallest(compare, -Infinity, arrayLike)` never terminates

`status: verified against Node 24.18.1` · `heap.js` · **needs an ID from the orchestrator**

The scan loop in the array-like branch is `for (i = n, l = iterable.length; i < l; i++)` with the
raw `n`. `-Infinity + 1` is `-Infinity`, so `i` never advances, `i < l` stays true and
`iterable[-Infinity]` — `undefined` — is read forever. Upstream hangs; the port hangs identically,
which is bug-for-bug correct and therefore untestable and unfuzzable.

Found while probing the `n`-validation defect the T2 review turned up, by which point this agent's
allocated range (B-70..B-79) was fully spent. CLAUDE.md says to say so rather than spill past the
range, so it is recorded here without an ID.

### Sixth entry for the confident-green-signal table — the T2 review

`status: three defects found, all fixed` · not upstream's; ours

The `heap` / `fixed-reverse-heap` unit had **21 upstream assertions, 47 boundary cases, three fuzz
campaigns and 5 M operations, all green**, while carrying:

1. a `RefCell` borrow held across a call into JavaScript, which **aborted the Node process** with
   `SIGABRT` when a re-entrant `clear()` reached it — not a catchable error;
2. `clear()` and `consume()` preserving an array class that upstream discards, i.e. the port being
   *more* faithful than upstream and therefore wrong;
3. `n` validated in the bridge before upstream would have validated it, so
   `Heap.nsmallest(cmp, 2.5, array)` threw where upstream answers `[2, 5]`.

None was reachable by the fuzzer **by construction**: its `VecStore` never calls JavaScript from
`allocate` (so 1 is structurally impossible there), has a single class (so 2 cannot appear), and
`nsmallest`/`nlargest` are outside the grammar (so 3 cannot). All three were found by a reviewer
poking the built addon by hand.

*Same lesson as B-31, and it is now twice: passing your own verification is not the same as being
correct, and both times the only thing that caught it was a second, independent look. The specific
generalisation is sharper than "fuzz more" — a differential fuzzer whose oracle-side store cannot
run user code cannot find a bug that needs user code to run, however many operations it does.*

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

### B-31 — RESOLVED 2026-08-01, across every exposed bridge

`status: FIXED` · repro flipped from `*** DIVERGENCE ***` to `MATCH`

Six bridges held a bare core value and had a callback-taking method:
`bit_set` · `bit_vector` · `sparse_map` · `sparse_queue_set` · `sparse_set` · `default_map`.
All six now hold `RefCell<Core>`, every `&mut self` method became `&self` + `borrow_mut()`, and the
`SharedReference` cursors project to the cell rather than to the structure — `CellCursor` and the
new `CellMapCursor` re-borrow on every `step()`. `queue`/`stack` were already correct and were the
reference.

**The two "immune" bridges really are immune, and it is worth saying why rather than that.**
`static_disjoint_set` and `hashed_array_tree` were re-checked for the actual precondition, which is
not "has no `forEach`" but "JavaScript cannot run while a `&self` method is on the stack". Neither
takes a `Function` or `FunctionRef` anywhere; neither hands out a `Reference`/`share_with` cursor;
and every argument either is a plain number (napi's `f64`/`u32` extractors do **not** call
`valueOf`) or reaches only a constructor, where no `self` exists yet. Left alone.

#### The rule the fix imposes, and the two places that nearly broke it

`RefCell` alone is not the fix. **No borrow may be alive across a call that can run JavaScript** —
because a `RefCell` panic inside a `#[napi]` method does not become a JS exception. napi 3.12 does
not `catch_unwind` a sync call, and a panic unwinding out of an `extern "C"` frame **aborts the
process**. Measured twice, on a first draft of this fix that had shipped the abort:

* `DefaultMap.get` ran the JS factory *inside* `try_get_or_insert_with`, which owns the map for the
  whole insertion. A factory that did nothing but read `map.size` aborted Node. Fixed by splitting
  the call into upstream's own three steps — read, factory (map unlocked), write — which needed one
  new core method (`insert_from_factory`) and is **closer** to upstream than the single core call it
  replaced, since upstream's factory also runs between its read and its write. A re-entrant factory
  now behaves exactly as upstream's, reading and writing included; verified differentially.
* `BitVector`'s growth policy is JS that *core* calls from inside `grow`, so the borrow genuinely
  cannot be released around it. A policy that read `vector.length` aborted Node. Every borrow in
  that bridge is now fallible and raises a named error. Upstream serves such a call from a half-grown
  vector; this port refuses it — a stated narrowing (decision B31-b) that replaces an abort, which
  replaced UB.

*Both were caught the same way, and it is the transferable part: after fixing the `forEach` case,
ask what **else** can run JavaScript while a `&self` method is on the stack. The answer was "a
factory" and "a growth policy", and neither is shaped like a callback.* A `code-reviewer` pass over
the diff independently flagged the same two, from the call graph rather than from a repro.

#### Regression cover: `tests/boundary/reentrancy.js`

Twenty-two specs, differential against `bench/upstream/` wherever the two sides are supposed to
agree, over all six fixed bridges plus `stack`/`queue` as controls, plus the two non-`forEach`
re-entry paths above. Falsified before being trusted: run against the pre-fix bridges, **7 of the
first 18 fail**; after, all pass. Note which ones failed — `sparse-set` (3), `sparse-map` (2),
`default-map` (2). `bit-set`, `bit-vector` and `sparse-queue-set` passed even pre-fix, because
their loops snapshot a word or freeze a bound and the hoist had nothing to change; they were still
UB and are still fixed.

**The specs must be run against a release build.** A debug addon has no `noalias` optimisation to
exploit and would pass while wrong. `tests/run.sh` builds `--release`, so this is already true —
but it is a property of the bug, and it is why the fix is a *type* and not a rearranged loop.

#### The grammar gap: closed, but not by as much as it looks

Every module whose bridge takes a callback now has a `$forEach(method, rule, limit)` op:
`sparse-set` · `sparse-map` · `sparse-queue-set` · `bit-set` · `bit-vector` · `stack` · `queue` ·
`default-map`. It generates a `forEach` whose callback calls back into the collection, and compares
both the callback argument sequence and the state left behind. Falsified: freezing the live
`this.size` bound in the Rust `sparse-set` walk turns a campaign red in 0.7s.

**It does not, and cannot, catch B-31.** `difffuzz` compares `mnemonist-core` against upstream JS;
the napi bridge is not in that loop. What the op does catch is the *loop shape*, where upstream is
inconsistent on purpose — live `this.size` in `sparse-set`/`sparse-map`, a frozen bound in
`sparse-queue-set`/`stack`/`queue`, a snapshotted word in `bit-set`/`bit-vector`, and a live `Map`
walk in `default-map`. Seven modules, four answers, and no program the old alphabets could generate
told any of them apart.

*So the honest version of the lesson is one step further than the entry above: the grammar gap was
real and worth closing, but the layer gap was the one that let B-31 through. A differential fuzzer
that skips a layer cannot find bugs in it, however complete its alphabet — and the alphabet being
incomplete made that easy to not notice.*

#### And the fuzzer found something on its first run — in itself

The first 20-second `sparse-set` campaign after `$forEach` landed went red immediately. The `arg0+1`
rule folded its arithmetic into the argument selection, so an `undefined` callback argument became
`NaN`, which is not `undefined`, sailed past the skip, and reached `SparseSet.add(NaN)` — where
upstream's `sparse[NaN]` comparison falls through and increments `size`, and the port cannot follow
because core takes a `usize`. A fuzzer bug, not a port bug; fixed by separating selection from
arithmetic, seed kept, log line commented out with its reason rather than deleted.

### The B-31 post-mortem corrects my own diagnosis: it was a LAYER gap, not a grammar gap

I recorded — and put in a commit message — that 2.94M fuzz operations missed B-31 because
`sparse-set`'s alphabet had no `forEach` op. True, and not the reason.

**`difffuzz` compares `mnemonist-core` against upstream JS. The napi bridge is not in that loop
at all.** B-31 lives entirely in the bridge. So adding `$forEach` to the grammar — which we did,
across all eight callback-taking modules — **still cannot catch it.** A grammar gap is a hole in
what you ask; a layer gap is a hole in what you are asking *about*, and no amount of asking
harder closes it.

What the new `$forEach` op does earn is real but different: upstream's `forEach` loop shape is
deliberately inconsistent across modules — live `this.size` in `sparse-set`/`sparse-map`, frozen
bounds in `sparse-queue-set`/`stack`/`queue`, snapshotted words in `bit-set`/`bit-vector`, a live
`Map` walk in `default-map`. **Seven modules, four different answers, and no program in the old
alphabet could tell them apart.**

B-31 needed a different instrument entirely: `tests/boundary/reentrancy.js`, 22 differential specs
run through the *bridge*, of which **8 fail against the pre-fix build**. And it only works against
a release build — a debug addon passes while wrong, because the hoist is an optimisation. That is
the argument for fixing the **type** rather than rearranging the loop.

**Ask of every clean campaign: which layer did this actually exercise?**

### Two more empty green signals, making seven

- **The fuzzer had a defect that produced a real red.** Re-fuzzing went red immediately — not on
  the port, but on the harness: `arg0+1` folded its arithmetic into argument *selection*, so
  `undefined + 1` became `NaN`, slipped the skip, and hit `SparseSet.add(NaN)`. A fuzzer bug
  wearing a divergence's clothes. Seed kept with provenance, log line commented rather than deleted.
- **I broke my own falsification of the B-31 fix.** I stashed the fix, rebuilt, copied the
  artefact to `prefix.node` — and the repro loads `addon.node`. Both runs therefore tested the
  *fixed* addon and both said MATCH, which I nearly reported as confirmation. Redone against the
  file the repro actually reads: pre-fix `[1,2,3,4]` DIVERGENCE, post-fix `[1,2]` MATCH.

*Seven instances now, and the through-line has sharpened: it is never "the check was wrong", it is
always "the check was answering a different question than the one I thought I asked".*
## Wave 1 — fixed-capacity modules (B-60..B-69 range)

Appended at the end of the file rather than into the bug-candidate section above: several agents
edit this file at once and only an addition at the very end can never land inside another one's
hunk.

### B-60 — `X.from(iterable, ...)` calls `iterables.forEach`, which `utils/iterables.js` never exported

`status: verified against Node 24.18.1` · `mnemonist fixed-stack.js`, `fixed-deque.js`,
`circular-buffer.js`

`utils/iterables.js` exports exactly four functions — `isArrayLike`, `guessLength`, `toArray`,
`toArrayWithIndices`. All three fixed-capacity modules end their `from` static with

```js
iterables.forEach(iterable, function (value) { structure.push(value); });
```

so the branch that would accept a `Set`, a `Map`, a generator or a string is not a slow path, it is
a `TypeError`:

```
FixedStack.from(new Set([1,2,3]), Array, 3)
TypeError: iterables.forEach is not a function
```

Confirmed for all three classes. Every `from` call in all three upstream test files passes an array
or a typed array, which takes the array-like fast path and returns before the last line — so the
branch has, as far as the suite is concerned, never run. The fix is one identifier: these files'
siblings already `require('obliterator/foreach')`.

**Strong candidate.** Concrete, reproducible in three lines, obviously unintended, and it makes a
documented API (`X.from(anyIterable)`) not work at all.

### B-61 — `FixedStack.prototype.forEach` walks `items.length`, not `this.size`

`status: verified against Node 24.18.1` · `mnemonist fixed-stack.js`

Every other method in the file is written against `this.size`. `forEach` alone:

```js
for (var i = 0, l = this.items.length; i < l; i++)
  callback.call(scope, this.items[l - i - 1], i, this);
```

`this.items.length` is the **capacity**, so an under-full stack invokes the callback `capacity`
times, handing it the unused slots first — `undefined` from an `Array`, `0` from a `Uint8Array`:

```js
var s = new FixedStack(Array, 5); s.push(1); s.push(2);
s.forEach(function (v, i) { … });
// (undefined, 0) (undefined, 1) (undefined, 2) (2, 3) (1, 4)
```

`FixedDeque.prototype.forEach`, three files away, does it correctly (`l = this.size`), which makes
this a slip rather than a choice.

**The suite is structurally unable to see it.** Its one `forEach` block builds a capacity-3 stack
and pushes three items — the single shape in which `items.length === size`.

**A seventh entry for the empty-green-signal table.** The most plausible mis-port of this module is
to "correct" it to `self.size`. Measured: with that sabotage in place, `test/fixed-stack.js` stays
**fully green, 12 passing**. It is caught in 57 fuzz cases once the grammar has a mutating
`forEach` op, and by two native tests written from the source rather than from the tests. This is
the cleanest available demonstration of why gate 6 insists the sabotage be chosen by naming the
assertion it must break: the sabotage that matters most here is precisely the one no assertion
covers.

### B-62 — `FixedDeque.prototype.get` is bounded by the CAPACITY, and has no lower bound

`status: verified against Node 24.18.1` · `mnemonist fixed-deque.js`, and therefore
`circular-buffer.js`

```js
FixedDeque.prototype.get = function (index) {
  if (this.size === 0 || index >= this.capacity) return;
  index = this.start + index;
  if (index >= this.capacity) index -= this.capacity;
  return this.items[index];
};
```

Every other reader in the file guards on `this.size`. `get` guards on the capacity, and on nothing
at the bottom end. Two consequences, both measured:

```js
var d = new FixedDeque(Array, 3); d.push(1); d.push(2); d.pop();
d.size;    // 1
d.get(1);  // 2      <- popped, still returned
d.get(3);  // undefined -- 3 >= capacity, the one guard that fires

var e = new FixedDeque(Array, 4);
[1,2,3,4].forEach(function (v) { e.push(v); });
e.shift(); e.shift();     // start === 2, holding [3, 4]
e.get(-1);  // 2          <- shifted out, still returned
e.get(-2);  // 1
```

`CircularBuffer` gets it **literally**: `circular-buffer.js` builds its prototype with
`Object.keys(FixedDeque.prototype).forEach(paste)`, so the two classes share the same function
object. One bug, two classes.

**Why the suite cannot see it.** All four `get` calls in `test/fixed-deque.js` — and all four in
`test/circular-buffer.js` — are on a *full* capacity-3 deque with `start === 0`, which is the single
shape in which "bounded by the capacity" and "bounded by the size" are the same statement.

There is a third form that the port deliberately does **not** reproduce (D-65): a non-numeric index
reaches string concatenation, so `this.start + "1"` is `"21"` and the next comparison coerces it
back to a number. On a deque with capacity > 21, `get("1")` can therefore return the element at
physical slot 21.

**Also worth filing alongside B-60: `CircularBuffer.from` bypasses the overwriting this class exists
for.** The `from` static is the same fourteen lines in all three modules, so its array-like branch
copies by index and assigns `size` rather than pushing — leaving `size > capacity` on the one class
whose entire purpose is to prevent that. `CircularBuffer.from([1,2,3], Array, 2)` gives `size 3`,
`items [1,2,3]` and `toArray() [1, 2, 1]`. Verified on Node 24.18.1. Kept under B-60's umbrella
rather than given its own ID: the shared `from` is one piece of code with two problems.

### B-63 — `X.from` assigns `size` from `iterable.length` without checking it is a number

`status: verified against Node 24.18.1` · `mnemonist fixed-stack.js`, `fixed-deque.js`,
`circular-buffer.js`

The array-like branch of the shared `from` static ends with

```js
for (i = 0, l = iterable.length; i < l; i++) stack.items[i] = iterable[i];
stack.size = l;
```

and the predicate that selects it is `isArrayLike(t) = Array.isArray(t) || typed.isTypedArray(t)`,
where `isTypedArray` is **`ArrayBuffer.isView`**. `ArrayBuffer.isView` is true for a `DataView`, and
a `DataView` has no `.length` — it has `byteLength`. So `l` is `undefined`, the loop runs zero
times, and `size` is assigned `undefined`:

```js
var s = FixedStack.from(new DataView(new ArrayBuffer(4)), Array, 3);
s.size;       // undefined
s.toArray();  // [ undefined ]
```

`toArray()` is a **one**-element array rather than empty because `new this.ArrayClass(this.size)` is
`new Array(undefined)`, the single-argument form holding one `undefined`, and the `while (i--)`
loop then never runs because `undefined--` is `NaN`.

A `DataView` is the only reachable input — every value `Array.isArray` accepts has a numeric
`length`, and so does every typed array — and it needs an explicit capacity, because with none
`guessLength` returns `undefined` and the `could not guess iterable length` throw fires first.

**Found by probing the port against upstream rather than by reading the file**, which makes it the
one bug in this wave that the differential method found rather than confirmed. It is also the one
behaviour in the wave the port does **not** reproduce (D-66): a `usize` cannot hold `undefined`, and
a structure whose `size` is `undefined` is arithmetic on `NaN` from then on.

**Moderate candidate.** Narrower than B-60 and B-61 — a `DataView` is an odd thing to hand a stack
— but it is a genuine type confusion inside a predicate whose name says "array like", and the fix
is one `typeof` check.
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

### Six merges in, the merge rule I wrote does not work

After three merges broke identically, I added a rule: agents append new registry entries at the
**end** of each list, never in the middle, so a conflict boundary cannot land inside someone else's
hunk. Two agents adopted it and left comments in the source saying why.

**Merges five and six broke in exactly the same place anyway.** Git picks conflict boundaries by
line similarity, not syntax, and it split the *previous* entry — closing an existing match arm or
test function mid-body so both sides shared its tail. Seven hand repairs across those two merges.

The rule did help: merge four dropped from nine conflicts to five, and none in the four files that
had broken every previous merge. It narrows the surface; it does not close it. **A mechanical
resolver that treats a conflict as a set of lines is wrong whenever the hunk is not a whole
syntactic unit, and no authoring convention can fully prevent that.** What actually catches it every
time is the compiler — and for three of the seven, only `cargo test`, because `cargo build` does not
compile `#[cfg(test)]` blocks.

**Parallel agents also solve the same sub-problem twice.** Three instances: the `{"$global": …}`
constructor encoding, invented independently with the same JSON shape and the same function name;
the `tests/run.sh` fresh-clone bug, found and fixed twice; and a `$forEach` fuzz op built twice with
**incompatible signatures**, whose duplicate handlers landed in one JS `switch` — where the first
silently wins and `node --check` passes, because duplicate cases are legal syntax. That last one
also masked two further defects that only surfaced once the protocols were reconciled.

*Write-up beat: the cost of parallelism is not the merge, it is the convergent design you then have
to reconcile — and the reconciliation is where the interesting bugs were hiding.*

## Wave 2 — bi-map, fuzzy-map, bk-tree (B-120..B-139 range)

Appended at the end of the file rather than into the bug-candidate section above, for the same
reason Wave 1 is: only an addition at the very end can never land inside another agent's hunk.

### B-120 — `BiMap.prototype.clear` resets only ONE of its two size counters

`status: verified against Node 24.18.1` · `bi-map.js` · found by differential fuzzing (proptest,
seed 42), caught twice while fixing the port to match

`BiMap`/`InverseMap` share one `clear` function:

```js
function clear() {
  this.size = 0;
  this.items.clear();
  this.inverse.items.clear();
}
```

Both underlying `Map`s are genuinely emptied regardless of which side calls it, but only `this.size`
is reset — `this.inverse.size` (or, from the inverse view, `this.size`) is left at whatever it was.
Reproduced exactly:

```js
var m = new BiMap(); m.set('a', 'a');
m.clear();
m.size          // 0
m.inverse.size  // 1, STALE — items.size and inverse.items.size are both 0
```

The stale counter is not permanent: the next `set`/`delete` on either side recomputes both counters
from the live maps (`this.size = this.items.size; this.inverse.size = this.inverse.items.size;`),
so it heals on the very next mutation. `delete` on an absent key, however, is a no-op that returns
`false` **before** touching either counter — so a `clear()` immediately followed by a `delete()`
that finds nothing leaves the stale counter stale for a second op, which is the case fuzzing caught
on the fix's own first re-run (see below).

**Two rounds, because a naive fix "healed" the desync (more correct than upstream, i.e. wrong):**

1. The first port derived `size()`/`inverse_size()` from `OrderedMap::len()`, so `clear()`
   incidentally zeroed both — a real defect (the port more correct than upstream), caught in 18
   cases (0.3s) on `set("a","a"); clear();`.
2. The fix added two real stored counters, reset asymmetrically by `clear`/`clear_reverse` — but
   resynced both counters from the live maps unconditionally after every `set`/`delete` call. Since
   `delete` on an absent key is upstream's other place that must NOT touch either counter, and that
   is reachable right after a genuine `clear()` (the maps really are empty, so the next `delete` on
   any key is a no-op), the unconditional resync "healed" the still-stale counter one op early.
   Caught in 177 cases (0.3s) on `set("a","a"); clear(); delete("a");` — the very next campaign run
   against round 1's fix.

Fixed by resyncing `delete`/`delete_reverse` only on an actual removal (`Some(_)`), matching
upstream's `del`, which only touches the counters on its own falling-through path. Both seeds are
committed with provenance in `crates/difffuzz/proptest-regressions/bi-map.txt`; the campaign is
clean on the current tree (5,000 cases / 38.3s, seed 42, zero divergences).

**Strong candidate**, and a useful cautionary tale for this project's own porting discipline: the
module doc for `mnemonist_core::structures::bi_map` had already analysed and named this exact bug
(B-120) in prose before the implementation caught up to it — a reminder that a doc comment
describing intended behaviour is not evidence the code behind it does that, only a claim to verify.

### fuzzy-map's difffuzz spec — a harness bug, not an upstream one

Not a bug candidate (recorded here so the fix has a paper trail): `crates/difffuzz/src/modules/
fuzzy_map.rs`'s `Hash::named` matched literal names `"identity"`/`"lower"`, but the ctor strategy
and `fuzz/oracle.js`'s `FACTORIES` table both use the prefixed names `fuzzyIdentity`/`fuzzyLower`
(chosen precisely so this module's factory names cannot collide with `default-map`'s — see
`fuzz/oracle.js`). Every generated program panicked at construction before ever reaching a real
comparison, which is why this spec had never actually run: `cargo test` never wired it into
`tests/differential.rs`, and the one manual campaign attempt persisted a regression seed that
turned out to be the harness panic, not a divergence. Fixed by matching the prefixed names; that
spurious regression file was deleted rather than kept, since it recorded a harness defect, not a
finding.

### bk-tree — clean, and the grammar was already sound

No divergence found. `crates/difffuzz/src/modules/bk_tree.rs`'s grammar was already built the way
CLAUDE.md's fuzz-campaign guidance asks for: items and queries drawn from a 12-wide range with
`distance = |a - b|`, dense enough that repeated `add`s collide on distance constantly (the only way
a node grows more than one child), and `search`'s radius reaching up to twice the item range so it
usually visits the whole tree — the closest thing to an observation of `root`'s shape this grammar
has, since core exposes no direct equivalent. 5,000 cases / ~35s at seed 42, zero divergences.
## lru-cache family (`lru-cache`, `lru-map`, `lru-cache-with-delete`, `lru-map-with-delete`)

Allocated range B-140..B-159. Full write-ups also live in `docs/modules/lru-cache.md`; this is the
capture-log version, filed here because that file's own note said the previous agent died before
writing one up.

### B-140 — `setpop` silently drops the eviction report when the evicted key is JS-falsy

`status: VERIFIED against Node 24.18.1` · `lru-cache.js`, `lru-map.js`, and both `-with-delete`
siblings via prototype copy · found by the differential fuzzer's very first campaign against this
grammar, before any campaign for this unit had been logged

```js
// setpop, the eviction branch:
if (oldKey) {
  return {evicted: true, key: oldKey, value: oldValue};
}
else {
  return null;
}
```

`if (oldKey)` is JS truthiness, not "was something evicted". Evict a key that happens to be falsy —
`0`, `""`, `false`, `NaN`, `null`, `undefined` — and `setpop` reports `null`, indistinguishable from
"nothing was evicted or overwritten", even though the entry really was displaced:

```js
var cache = new LRUCache(3);
cache.set(0, 'a'); cache.set(1, 'b'); cache.set(-1, 'c');   // full, size === capacity
cache.setpop('d', 'e');   // evicts key 0 -- returns null, not {evicted:true, key:0, value:'a'}
```

`test/lru-cache.js`'s three `setpop` blocks all evict/overwrite a plain non-empty string, so gate 4
never touches this path. The port's own core (`LruCache::set_pop`) has no notion of JS truthiness at
all and reports every eviction correctly regardless of the key — which is *more correct than
upstream* and therefore a defect per CLAUDE.md's bug-for-bug mandate. Reproduced at the bridge via
`is_js_truthy` (`crates/mnemonist-napi/src/lru_cache.rs`), gating the `Evicted` arm of all four
bridges' `setpop`; mirrored in the fuzz spec's own `FuzzKey::is_js_truthy` so campaigns compare
against the now-correctly-buggy behaviour rather than manufacturing a false divergence out of a bug
that is supposed to be there. Found on the third generated case: the key pool includes four
JS-falsy raw values out of ten, deliberately, for exactly this reason.

### B-141 — reserved, unused

No second upstream defect surfaced during this unit's reading or its ~6.37M fuzzed operations
beyond B-140 and B-142 below. Recorded so a future agent extending this range does not assume B-141
was skipped by mistake.

### B-142 — `lru-map.js`'s own `.from` names the wrong module in its error message

`status: VERIFIED against Node 24.18.1` · `lru-map.js:241` · found by reading

```js
throw new Error('mnemonist/lru-cache.from: could not guess iterable length. ...');
```

A copy-paste artefact: `lru-cache.js`'s own `.from` has the identical line with the *correct*
module name, and both `-with-delete` siblings (`lru-cache-with-delete.js`, `lru-map-with-delete.js`)
get their own name right too — confirmed by grepping all four upstream files for the exact string.
So this is specific to one file, not systemic. Reproduced verbatim in `mnemonist_napi::lru_map`'s
`CANNOT_GUESS` constant. Not independently fuzzable: it fires during `Cache.from`'s argument-arity
resolution, before any instance exists, which is an `init`-time failure in the oracle protocol
(`fuzz/oracle.js`) rather than an op comparison the harness can shrink toward.

### Two port defects, not upstream's, both found and fixed before this unit's logged campaigns

Recorded here per CLAUDE.md ("do not overclaim causation" cuts the other way too — these are not
upstream bugs and get no B-number), and in full in `docs/modules/lru-cache.md`'s "Bugs this found".

**1 — `LruCache::unlink` (used by `delete`/`remove`) nulled `this.K[pointer]`/`this.V[pointer]`,
which upstream never does.** A walk left open across a delete of a not-yet-visited pointer hit the
walk's own liveness invariant, now falsified, and **panicked**. Found by reading, before any fuzz
campaign for this unit ran — the shape (a hole-bearing `-with-delete` variant, an open walk, an
interleaved mutation) is exactly what this unit's brief named as the interesting territory, so it
was checked directly with a scratch probe before the fuzz grammar even existed. Fixed by leaving
both slots stale, matching upstream's own (never-nulled) arrays, and by changing `remove` to clone
the value it returns instead of taking it (which independently zeroed the slot a second way).

**2 — `forEach` in both the fuzz spec and the napi bridge advanced its pointer before the callback
ran, where upstream's own loop advances after.** Reused the `Sequence`/`CursorState` machinery built
for the lazy `keys`/`values`/`entries` iterators — correct for them, because their closures advance
before ever returning control to the caller, which is NOT how `forEach`'s callback-then-advance loop
body works. Found by the differential fuzzer's first campaign against this grammar: a `$forEach`
program that promotes an entry mid-walk disagreed on the third callback invocation. Fixed by
`ForEachWalk` (`mnemonist_core::structures::lru_cache`), which splits "read" from "advance" into two
calls so the caller's mutation always lands between them. `test/lru-cache.js`'s own `forEach` block
never mutates from inside its callback, so gate 4 could not have found this either.

**Both defects are pinned**: three Rust unit tests for the first
(`crates/mnemonist-core/src/structures/lru_cache.rs`), and a checked-in proptest regression seed
with a provenance header for the second (`crates/difffuzz/proptest-regressions/lru-cache.txt`).

### Falsification (gate 6) — breaking recency-on-`get`, not storage

For an LRU the sharp target is the recency bookkeeping, not the store, so the sabotage was
`LruCache::get` with its `splay_on_top` call commented out — reads still return the right value,
they just stop promoting anything. Named before running: the last assertion of
`reproduces_the_upstream_walkthrough` (`entries(&cache)` after `cache.get("four")`), and the
equivalent line in `test/lru-cache.js`.

**All three instruments caught it, independently:** the named Rust assertion went red exactly where
predicted; the original suite dropped from 88 passing to 72 passing / 16 failing; and the
differential fuzzer found a divergence in 74 operations and 0.4 seconds, minimised to nine ops,
disagreeing on `head`/`tail` immediately after a `get`. Reverted; confirmed green at all three again.
Nothing here was found to be blind.

## _utils (typed-arrays, binary-search, hash-tables, iterables, merge)

Allocated range B-180..B-199. `typed-arrays`, `binary-search`, `hash-tables` and `iterables` were
already ported by earlier work (as members of this eventual unit — see each one's own module docs);
this pass ported the missing sibling, `utils/merge.js` (563 LOC), wired the whole require-closure
through the napi bridge, and ran the unit's first differential-fuzz campaigns. Full write-up in
`docs/modules/_utils.md`.

### B-180 — the k-way `merge`/`unionUnique` throw a `TypeError` whenever filtering an empty array
### out leaves three-or-more arrays live

`status: VERIFIED against Node 24.18.1` · `utils/merge.js`, `kWayMergeArrays` and
`kWayUnionUniqueArrays` · found by reading, while porting, then confirmed against a real
`pm-recon/mnemonist` v0.40.4 checkout before any Rust code depended on the finding

```js
function kWayMergeArrays(arrays) {
  var length = 0, max = -Infinity, al, i, l;
  var filtered = [];

  for (i = 0, l = arrays.length; i < l; i++) {   // `l` captured HERE, before filtering
    al = arrays[i].length;
    if (al === 0) continue;
    filtered.push(arrays[i]);
    length += al;
    if (al > max) max = al;
  }

  if (filtered.length === 0) return new arrays[0].constructor(0);
  if (filtered.length === 1) return filtered[0].slice();
  if (filtered.length === 2) return mergeArrays(filtered[0], filtered[1]);

  arrays = filtered;                              // reassigned; `l` is now stale
  // ...
  for (i = 0; i < l; i++)                         // `l` is the ORIGINAL length, not `filtered.length`
    heap.push(i);
  // ...
}
```

Whenever at least one input array was empty (so `filtered.length < l`) *and* three-or-more arrays
remain after filtering (so the code reaches the heap section at all), the heap is seeded with
indices past the end of the now-shorter `arrays`. The first `heap.pop()` that touches one of them
reads `arrays[p]` (`undefined`) and then indexes it, throwing
`TypeError: Cannot read properties of undefined (reading 'undefined')`. Confirmed directly:

```js
require('mnemonist/utils/merge').merge([], [1, 2, 3], [4, 5, 6], [4, 7])       // throws
require('mnemonist/utils/merge').unionUnique([1, 2], [], [3, 4], [5, 6])       // throws
require('mnemonist/utils/merge').merge([1, 2], [], [3, 4])                     // OK -- filtered.length is 2
require('mnemonist/utils/merge').intersectionUnique([], [1, 2, 3], [4, 5, 6]) // OK -- returns [] before any heap exists
```

`kWayIntersectionUniqueArrays` has no `FibonacciHeap` at all (a sequential binary-search fold) and
returns `[]` on the very first empty array it scans, before the stale-`l` code path would ever be
reached — it is structurally immune, not merely untested.

Not one case in `test/_utils.js`'s own `'should properly merge k arrays.'` /
`'should properly perform the union of k unique arrays.'` blocks mixes an empty array in with
two-or-more non-empty ones, so gate 4 cannot reach this. Reproduced in the port as
`KWayError::StaleLengthMismatch` (`mnemonist_core::utils::merge`), surfaced at the napi boundary as
the identical thrown message text, rather than as a panic — `mnemonist-core` has no exceptions, so
this follows the same convention as `hash_tables::TABLE_IS_FULL` (D-44).

### Two port defects, not upstream's, both found by differential fuzzing and fixed before logging

Recorded here per CLAUDE.md ("do not overclaim causation" cuts the other way too — these are not
upstream bugs and get no B-number).

**1 — `union_unique_two`'s prefix loop deduplicated where upstream's does not.** Upstream's
`unionUniqueArrays` has a dedup check (`array.length === 0 || array[array.length - 1] !== v`) in its
overlap loop and both its filling loops, but its *prefix* loop (the one before any overlap is
detected) pushes unconditionally — relying on the caller's arrays already being internally unique.
A first draft of this port called the same `push_unique` helper in the prefix loop too, which is
*more correct* than upstream on an internally non-unique input and therefore a defect. Found inside
the first 300 cases of this unit's very first fuzz campaign:
`unionUnique([-5, -5, 0], [-0.5])` — port `[-5, -0.5, 0]` (deduped), upstream `[-5, -5, -0.5, 0]`
(kept both). Fixed by making the prefix loop push unconditionally, matching the source exactly.
Pinned: `mnemonist_core::utils::merge`'s
`the_prefix_loop_does_not_deduplicate_an_already_non_unique_input`.

**2 — not a defect, but worth recording precisely: the k-way linear scan's tie-break disagrees with
`FibonacciHeap`'s.** `fibonacci-heap.js` is not ported (T2 tier); the k-way merge/union here picks
the minimum head via a linear scan that keeps the earliest array on a tie. Upstream's
`FibonacciHeap.push` updates its `min` pointer with `<=` (favouring the *most recently pushed* node),
and after the first `pop`'s consolidation pass, which node ends up favoured on a later tie depends on
the heap's internal degree-bucket merging — not on insertion order alone. Found by the same
campaign: `merge([3], [2, -5], [2])` disagreed in element ORDER alone (upstream `[2, 2, -5, 3]`,
port `[2, -5, 2, 3]`), and `unionUnique([3], [2, -5], [2])` disagreed in which values survive
deduplication (upstream `[2, -5, 3]`, port `[2, -5, 2, 3]`). Both are the SAME root cause. This is a
genuine algorithmic-substitution gap, not a bug to fix here — closing it means porting
`fibonacci-heap.js` itself, a separate T2-tier unit. Recorded as D-18x (see
`planning/DECISIONS-CANDIDATES.md`) and worked around in the fuzz grammar by generating
globally-distinct values for every three-or-more-array case, which sidesteps the gap structurally
(with no ties, every correct implementation's extraction order is identical) rather than hiding it.

### Falsification (gate 6) — two attempts that stayed green, reported honestly, then one that didn't

First attempt: relaxed `k_way_scan`'s tie-break comparison from `<` to `<=` (favouring the latest
array on a tie instead of the earliest). Named target: `'should properly merge k arrays.'`. Stayed
**green** — the test's own tie (two arrays both starting at `1`, two both starting at `4`) resolves
to the same VALUES regardless of which array supplies them, so the sabotage was unobservable there,
consistent with the tie-break analysis above.

Second attempt: reversed `merge_two`'s swap condition, `a[0] > b[0]` to `a[0] < b[0]`. Named target:
`'should properly merge two arrays.'`, case `[[4, 5, 6], [1, 2, 3], ...]`. Stayed **green** — the
swap is an optimisation for the fast-concatenation-path check, not a correctness requirement of the
two-pointer walk itself, which is symmetric in which side is called `a`.

Third attempt, the one that worked: reversed the overlap loop's comparison, `a_head <= b_head` to
`a_head >= b_head`. Named target: `'should properly merge two arrays.'`, case
`[[1, 2, 2, 3], [2, 3, 3, 4], [1, 2, 2, 2, 3, 3, 3, 4]]`. **Confirmed red**: 25 passing / 1 failing,
exactly that assertion, with the actual output `[1, 2, 2, 3, 2, 3, 3, 4]` (unsorted) against the
expected `[1, 2, 2, 2, 3, 3, 3, 4]`. Reverted; **confirmed green again**, 26/26.

The two failed attempts are reported rather than discarded because they are themselves a finding:
the two-array merge is tie-order-invariant and swap-side-invariant by construction (no third array
can interleave), which is exactly why the k-way case (§ above) is the one place that invariant
breaks down — a third array's advancing pointer can land between what looked, in the two-array
case, like an unobservable choice.

## Empty green signals nine and ten, and one of a different shape

Appended at the end of the file rather than into the section above: several agents edit this file at
once and only an addition at the very end can never land inside another one's hunk.

- **A fuzz spec that had never run, reporting clean.** `fuzzy-map`'s `Hash::named` matched
  `"identity"`/`"lower"` while `fuzz/oracle.js` registers `fuzzyIdentity`/`fuzzyLower`, so every
  case panicked at construction — and the campaign reported zero divergences *truthfully*, because
  zero comparisons produce zero disagreements. The strongest instance yet: nothing was broken, the
  arithmetic was correct, and the number still meant nothing. 1,210,496 real ops after the fix.
- **Our own float decoding manufacturing divergences.** Both `vector` specs opened with 1-ULP
  disagreements indistinguishable from genuine port bugs. `serde_json`'s default float parser is
  not correctly-rounded: `38403.356486892444` lands one ULP from `f64::from_str`. Fixed with the
  `float_roundtrip` feature (D-103). The inverse of the usual failure — a check answering a
  different question and reporting *red*.

### And the gate catching itself, which is the point of gate 6

`_utils` named three sabotages in advance. **Two stayed green.** Relaxing the k-way tie-break and
reversing `merge_two`'s swap condition both left every assertion passing. They were reported as
findings — tie-invariance and swap-invariance in our own tests — rather than quietly replaced with
an easier target that would have gone red. The third went red exactly as predicted.

This is the first time the falsification gate has failed *and been recorded as a failure*. That is
the behaviour the rule was written for: a falsification that cannot fail is a second green light,
and two of these could not fail.

### A green campaign that is narrower than it sounds

`_utils` logs 1,012,101 ops and zero divergences, but D-105 is a real, **unfixed** divergence: the
k-way tie-break uses a linear scan and disagrees with `FibonacciHeap`'s ordering whenever three or
more arrays tie. Fixing it requires porting `fibonacci-heap`. The grammar was changed to generate
globally distinct k-way values, so the campaign is green *over a region that excludes the known
disagreement*. Documented rather than hidden — but it must be re-examined before `_utils` is
scoped, on the same reasoning that descoped `sparse-set`.
### B-200 — a token equal to `SENTINEL` corrupts the trie: `size` overcounts and the word is lost

`status: VERIFIED against Node 24.18.1` · `trie-map.js` and `trie.js` (inherited, same engine) ·
found by reading, before any fuzz campaign for this unit ran

`SENTINEL` (`String.fromCharCode(0)`) is not a reserved namespace, just an ordinary property key on
the same plain object every token is stored under. `set`'s walk is

```js
node = node[token] || (node[token] = {});
```

so if a real token equals `SENTINEL` at a node that is *already* a stored word, `node[token]` reads
the **value** stored there (a JS primitive, since `SENTINEL` is what `set` uses for the value slot
too), not a sub-object. `node` becomes that primitive. Every later iteration's
`node[token] = {}` is then a property write on a primitive, which is a **silent no-op** in sloppy
mode — the assignment *expression* still evaluates to the fresh `{}` on its right-hand side, so the
loop's local `node` variable keeps reassigning to a chain of brand-new, entirely unlinked objects,
but nothing is ever stored anywhere reachable from `root`. Confirmed by direct execution:

```js
var t = new TrieMap();
t.set('a', 'word-a');
t.set('a' + TrieMap.SENTINEL + 'b', 'word-a0b');
t.size;                              // 2 -- incremented for the ORPHAN, not for anything stored
t.get('a' + TrieMap.SENTINEL + 'b'); // undefined -- unreachable through any public method
t.root;                              // { a: { '\x00': 'word-a' } } -- no trace of the second set
```

`size` increments because the *final* orphan object genuinely has no `SENTINEL` property of its
own (`!(SENTINEL in node)` is true for a fresh `{}`), so upstream's own bookkeeping cannot tell "a
value was stored" from "a value was silently discarded into the void." Neither `test/trie.js` nor
`test/trie-map.js` ever embeds the sentinel character in a real token — every key in both suites is
an ordinary word — so gate 4 cannot reach this. The port's own `Node` (stores value and children in
separate fields rather than a shared keyspace, per D-200 in DECISIONS-CANDIDATES.md) does not
reproduce the corruption; both operations in the sequence above succeed and are independently
retrievable, pinned by
`crates/mnemonist-core/src/structures/trie_map.rs`'s
`a_token_equal_to_the_sentinel_character_is_an_ordinary_token`.

### B-201 — a `delete` that prunes from an ancestor leaves an open walk still able to yield the
### deleted word

`status: VERIFIED against Node 24.18.1` · `trie-map.js` (and `trie.js`, same engine) · found by
reading, then confirmed by direct execution before any fuzz campaign for this unit ran

`values`/`prefixes`/`keys`/`entries` return a closure over two live JS arrays, `nodeStack` and
`prefixStack`, holding **actual node object references** it has already discovered but not yet
visited. `delete`'s own pruning is `delete toPrune[tokenToPrune]` — it removes the *parent's*
reference to a node, not necessarily any property on the node object itself. So a `delete` whose
pruning point is an **ancestor** of a node the walk has already queued removes that node from the
trie's reachable structure while leaving the node object itself, and its own `SENTINEL` property,
completely untouched — and the walk, holding the object directly, keeps reporting it:

```js
var t = new TrieMap();
t.set('a', 1); t.set('ab', 2); t.set('abc', 3);
var it = t.prefixes();
it.next();          // {value: 'a'}
t.delete('abc');     // prunes 'c' off the 'ab' node
t.delete('ab');       // prunes the WHOLE 'ab' node off 'a' -- but 'ab' node's own SENTINEL is
                      // never itself deleted, only the parent's reference to the node is
it.next();           // {value: 'ab'} -- the just-deleted word, still yielded
it.next();           // {done: true}
```

Neither original test file interleaves a `delete` with an open `values`/`prefixes`/`keys`/`entries`
walk over the region being deleted, so gate 4 cannot reach this either. The port's walk
(`mnemonist_core::structures::trie_map::Walk`) re-navigates from the root by token path on every
step rather than holding a live reference, so it disagrees with upstream in exactly this scenario —
recorded as D-201 in DECISIONS-CANDIDATES.md rather than reproduced, since doing so would mean
storing live aliased references inside a structure this port also needs to hand across the FFI
boundary as a detached, resumable cursor.

**Independently confirmed by the differential fuzzer, twice, before this narrowing existed.** The
first ungated campaign for each unit diverged inside a few hundred operations: `trie-map`'s over
`delete` unlinking a queued `entries()` frame (the scenario above, exactly), `trie`'s over `clear`
replacing the whole root out from under an open `keys()` cursor (upstream's stale root, having
nothing on it yet, correctly answers `{done: true}`; the port's live re-navigation sees the
addition that happened after `clear` and wrongly keeps going). Both are the same underlying gap,
not two bugs — `clear` is `delete` of everything at once, from the cursor's point of view. Both
campaigns were fixed by excluding `delete`/`clear` from ever sharing a generated program with a
persistent `$iter`/`$next` cursor; see `crates/difffuzz/src/modules/trie_map.rs`'s module docs for
the exact repros and the regime-flag mechanism.
## multi-map, multi-set, fuzzy-multi-map (B-160..B-179 range)

Full write-ups live in `docs/modules/multi-map.md`, `multi-set.md`, `fuzzy-multi-map.md`; this is
the capture-log version. All three units' original suites pass unmodified (17 + 26 + 11 blocks, 91 +
83 + 27 assertions), gate 6 falsified and reverted for each, six 60-90s fuzz campaigns (~4.7M ops,
zero divergences) plus `grammar_self_check` tests giving direct counts of multi-value-key and
drain-to-zero states reached.

### B-160 — `multi-set`'s `#.set` on an existing item adds, it does not replace

`status: verified by reading` (deterministic plain JS, no runtime ambiguity to double-check against
Node) · `multi-set.js`'s `set`. `set.set('hello', 4); set.set('hello', 3);` gives multiplicity `7`,
not `3` — the `else` branch that would replace only runs for a key not already present as a number;
an existing one goes through `this.items.set(item, currentCount + count)`. `test/multi-set.js`'s own
double-`.set` case never calls it twice with two *positive* counts (its second call is negative,
taking the early delete branch instead), so gate 4 never touches this. Reproduced bug-for-bug.

### B-161 — `multi-set`'s `#.delete` on an absent item corrupts `size` to `NaN` and reports `true`

`status: verified by reading` · `multi-set.js`'s `delete`. The guard `if (count === 0) return
false;` is meant to catch "item not present", but `this.items.get(item)` on a missing item is
`undefined`, and `undefined === 0` is `false` in JS — so the guard is **dead code** (no live entry is
ever exactly `0`; every write path here deletes an item outright instead of leaving a zero). Falls
through to `this.size -= undefined` (`NaN`), `this.dimension--` unconditionally, a harmless no-op
`items.delete`, and returns `true` regardless. This is the one that forced `MultiSet::dimension` to
be a real tracked counter rather than derived from `items.len()` the way `multi-map`'s is — a
derived counter would have silently *fixed* this bug instead of reproducing it, the same trap the
bi-map counters (B-120) already taught this project once.

### B-162 — `multi-set`'s `#.edit` never adjusts `dimension`, even when it removes a real key

`status: verified by reading` · `multi-set.js`'s `edit`. When `b` already exists, `edit(a, b)`
merges `a`'s multiplicity into `b` and deletes `a` — the real distinct-key count drops by one — but
`this.dimension` is never touched by this method at all. `test/multi-set.js`'s own third `edit` case
exercises exactly this shape (`set.add('c'); set.edit('b', 'c');`) but only reads `multiplicities()`
afterwards, never `.dimension`, so gate 4 cannot see the drift. Reproduced bug-for-bug — `edit` does
not touch the tracked counter either.

### Two resource leaks in this port's own bridge, found and fixed — `fuzzy-multi-map`

Not upstream bugs, no B-numbers; recorded because both were caught by watching stderr during gate 4,
not by any assertion failing, which is worth remembering next time a suite reports "all green" — see
`docs/modules/fuzzy-multi-map.md`'s "Bugs this found" for the full write-up. (1) `.from`'s collector
retained every value up front, then the first draft resolved it back to a live view and re-retained
it a second time before storing, leaking the first `napi_ref`. Fixed by `store_retained`, which takes
the already-retained value directly. (2) `MultiMap::set_with` used to drop a `Set`-kind duplicate
candidate silently on rejection; for a plain `JsKey` (`multi-map`'s own bridge) that's harmless, but
for `fuzzy-multi-map`'s `Rc<RefCell<Retained>>` items it leaked the freshly-retained handle every
time a genuine duplicate object was `.set()` a second time. Fixed by changing `set_with`'s return
type to `Result<Option<V>, E>`, handing the rejected value back so the bridge can release it.

### Two harness bugs in the fuzz specs themselves, found and fixed — `multi-set`, `fuzzy-multi-map`

Also not port or upstream defects. `multi-set`'s spec initially echoed a fixed return value for
`add`/`remove`/`edit` regardless of upstream's actual per-branch semantics (the sign-flip delegation
between `add`/`remove`, and `edit`'s early `undefined` when `a` is absent) — caught by the very first
generated case in each instance. `fuzzy-multi-map`'s spec initially rendered `items` as a flat
`{$map: ...}`, but upstream's own `this.items` is a `MultiMap` *instance*, not a raw `Map`, so
`fuzz/oracle.js`'s generic `encode()` renders it as the nested `{items: {$map}, size, dimension}`
shape its own enumerable properties are — diverged on case 0 of every run until fixed. Neither
survived past the first campaign attempt; both are recorded in the fuzz specs' own doc comments.

## fibonacci-heap (B-220..B-239 range)

Full write-up: `docs/modules/fibonacci-heap.md`. `fibonacci-heap.js` (321 LOC) has **no
`decreaseKey`, no `delete`, no `mark` field, no cut and no cascading cut anywhere in the source or
its `.d.ts`** — grepped, not assumed. The public surface is `clear`, `push`, `peek`, `pop`,
`inspect`, `.from`. This matters directly for this unit's fuzz campaign: cascading cuts cannot be
reached by any grammar, however wide, because there is no operation upstream that could ever
trigger one. Stated plainly rather than glossed over, per this unit's own brief.

### B-220 — a comparator that `clear()`s the heap during the first `pop`'s `consolidate` leaves
### `this.size` at `-1`, and the next `pop` then crashes

`status: verified by tracing upstream's own execution model` (deterministic JS integer/falsy
semantics, no runtime ambiguity to double-check against Node) · `fibonacci-heap.js`'s `pop`.

`pop`'s tail is:

```js
if (z === z.right) { this.min = null; this.root = null; }
else { this.min = z.right; consolidate(this); }
this.size--;
return z.item;
```

`this.size--` runs **after** `consolidate`, unlike `heap.js`'s `pop`, which decrements `size`
*before* its sift (see `docs/modules/heap.md`'s D-70-adjacent note on that method). A comparator
invoked from inside `consolidate` that calls `heap.clear()` — a legitimate re-entrant call, no more
exotic than the `pushy`/`popper`/`clearer` factories `heap.js`'s own fuzz grammar already
uses — sets `this.size = 0` mid-consolidate. The pending `this.size--` then computes `0 - 1`,
i.e. **`-1`**, a real value JavaScript holds without complaint (there are no unsigned integers).

The second half is the sharper one: `pop`'s own entry guard is `if (!this.size) return undefined;`,
a **falsy** check, and `-1` is truthy in JavaScript. So the *next* `pop()` call does not see an
"empty" heap — it proceeds to `var z = this.min;` (`null`, from the `clear()`) and then
`if (z.child)`, which is `null.child`: a `TypeError: Cannot read properties of null (reading
'child')`. Upstream crashes on the pop *after* the one that corrupted `size`, not on the corrupting
one itself.

`test/fibonacci-heap.js` never uses a mutating comparator at all (see "What upstream does NOT
test" in the module doc), so gate 4 cannot reach this from either half.

**Port:** `size` is `i64`, not `usize`, in `mnemonist_core::structures::fibonacci_heap`'s
`FibonacciHeap` — matching `multi-set`'s D-163 precedent for the identical class of problem (a
tracked counter that upstream's own arithmetic can drive to a state a "cleaner" derived value would
never reach). `-1` is reproduced exactly. The follow-on crash is reproduced as a Rust panic (core
has no exceptions) rather than a `Result::Err`; pinned by
`a_pop_after_b_220s_negative_size_panics_matching_upstreams_null_dereference`
(`#[should_panic]`), immediately below the test that pins the `-1` itself
(`a_comparator_that_clears_the_heap_mid_pop_does_not_panic`).

Found while writing this unit's own re-entrancy tests (the shape CLAUDE.md's brief for this unit
named directly: "the comparator ... can mutate the heap mid-operation"), not by the differential
fuzzer — the fuzz grammar does not generate a program long enough after a `clear()`-capable
comparator's budget to specifically retrigger a *second* `pop` against the corrupted state; see
`docs/modules/fibonacci-heap.md`'s "What we test in addition" for why this is stated as a native-
test-only finding rather than claimed as fuzz coverage it does not have.

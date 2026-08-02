# Bugs found in upstream `mnemonist`

This document is our Bug Catcher submission: genuine defects in the JavaScript library this project
ports, found while reading its source line by line and while differentially fuzzing it against our
Rust port. Every claim below is independently reproducible — paste the code block into `node` and
compare against the stated output.

**Verification.** Every repro in this document was executed directly, moments before writing it,
against `mnemonist@0.40.4` and `obliterator@2.0.4` — the exact versions this port targets — on
**Node v24.18.1**. Nothing here is inferred from reading the source alone; every code block is a
transcript, not a prediction.

**How our port handles each one.** With two exceptions noted inline, we reproduce every bug below
bug-for-bug rather than fixing it. That is a deliberate choice, not an oversight: the contract of
`mnemonist-core` is "behaves like `mnemonist`, bugs included," because the JavaScript library's own
test suite is the evidence of equivalence, and a port that silently repairs a bug the suite doesn't
test for is no longer verifiably equivalent — it has just replaced one unverified belief with
another. In a few places fidelity cost us idiomatic Rust: `MultiSet`'s size counter is tracked
rather than derived so that a delete-on-absent-key bug (below) reproduces instead of healing itself,
and one bridge deliberately reports `null` for a falsy evicted key because upstream's own
truthiness check does the same.

## How these are ranked

The organisers named the sharpest filter themselves: a defect whose upstream test *asserts* the
buggy output as if it were correct proves the bug survived review; one the tests merely never reach
proves only that nobody looked. We checked for the former specifically and did not find one —
across every bug below, the existing test suite simply never exercises the state where the bug
lives. That is worth saying plainly rather than stretching a weaker finding to fit the stronger
claim. So the ranking here is severity first — silent data corruption ranks above a crash, which
ranks above a wrong error message — then how narrowly the existing suite missed it, then how
surprising the mechanism is.

---

## 1. `SparseMap.delete` moves the key and leaves the value behind

**File:** `sparse-map.js` · **Severity:** silent value corruption, no out-of-range input needed.

```js
const SparseMap = require('mnemonist/sparse-map');
const m = new SparseMap(10);
m.set(3, 'a'); m.set(4, 'b'); m.set(5, 'c');
m.delete(3);
console.log(m.get(5));   // 'a'
console.log([...m]);     // [[5,'a'],[4,'b']]
```
```text
a
[ [ 5, 'a' ], [ 4, 'b' ] ]
```

Member 5 now reports member 3's value. This is not an edge case reachable only with unusual input —
it happens on the very first `delete` from a map with more than one entry.

**What upstream does wrong.** `delete` is `SparseSet`'s swap-with-last removal, copied into a
structure that has a third parallel array (`vals`) the copy never touches:

```js
index = this.dense[this.size - 1];
this.dense[this.sparse[member]] = index;   // the last MEMBER moves into the hole
this.sparse[index]              = this.sparse[member];
this.size--;                               // `vals` is never touched
```

The dense/sparse index bookkeeping is updated correctly; the values array is not, so the value at
the old position keeps belonging to whichever member's dense slot it physically occupies.

**Why the suite misses it.** `test/sparse-map.js` calls `delete` exactly twice, and both times the
map holds exactly one entry — the swap-with-last is a self-assignment in that case, indistinguishable
from correct. Confirmed by sabotaging our port to *fix* this bug: `test/sparse-map.js` stays at
**9 passing, 0 failing**, while four of our own native tests turn red and this project's differential
fuzzer (comparing our Rust port against real upstream JS) catches the divergence in 3.0 seconds,
minimised to a three-operation repro.

**How our port handles it.** Reproduced verbatim: `SparseMap::delete` performs the identical
swap-with-last on the key/index arrays while leaving the value array's stale slot in place.

---

## 2. `SuffixArray`'s radix sort silently narrows to 8 bits

**File:** `suffix-array.js` · **Severity:** silently wrong output, measured **81% of inputs wrong**
over a realistic distribution.

```js
const SuffixArray = require('mnemonist/suffix-array');
console.log(new SuffixArray('ĀĀĀĀȁĀĀȁȁȁȁȁĀȁȁ').array);
```
```text
[
  0, 1, 2, 5, 3, 12,
  6, 4, 14, 11, 10, 13,
  9, 8, 7
]
```
The correct suffix array for that string is
`[0,1,2,5,3,12,6,14,4,11,13,10,9,8,7]` — two positions (`6,4,14` vs `6,14,4`) are transposed.

**What upstream does wrong.** `sort()` decides how many bits of each symbol to compare by reading
`string[array[i] + offset]` at an index that runs past the end of the padded working array for
`offset` 1 and 2. The read is `undefined`, `Math.max(undefined, j)` is `NaN`, every bit-shift of
`NaN` is `0`, and the radix-width cascade (`j >> 24 && 32 || j >> 16 && 24 || j >> 8 && 16 || 8`)
falls all the way through to **8 bits** — so the sort ends up comparing only the low byte of every
16-bit character code, for every symbol above U+00FF whose low byte collides with another symbol's.
Measured over 10,000 random inputs (length 1–30, alphabet `{'A','Ł'}`, length divisible by 3): **81%
produce a wrong suffix array**; pure-ASCII input is unaffected because no low-byte collision exists
in that range.

**Why the suite misses it.** Every string in `test/suffix-array.js` is ASCII. The bug requires a
non-Latin1 character whose low byte collides with another symbol in the same string — nothing in
the existing suite reaches that.

**How our port handles it.** Reproduced: the Rust port performs the identical radix-width
computation and inherits the identical narrowing.

---

## 3. `murmurhash3`'s 32-bit adder is broken, and a swapped constant hides it exactly

**File:** `utils/murmurhash3.js` · **Severity:** latent — correct today only by numeric coincidence.

```js
function sum32(a, b) {
  return (a & 0xffff) + (b >>> 16) + (((a >>> 16) + b & 0xffff) << 16) & 0xffffffff;
}
console.log(sum32(1, 1));   // should be 2
```
```text
65537
```

**What upstream does wrong.** A correct 32-bit split-add takes `b`'s *low* half for the low
contribution and `b`'s *high* half for the high contribution. `sum32` has both halves backwards, so
it adds `b`'s high half into `a`'s low half and vice versa — genuinely broken as a general 32-bit
adder, confirmed above.

It is called exactly once in the file, with the constant `n = 0x6b64e654`. MurmurHash3's real
published mixing constant is `0xe6546b64` — the same eight hex digits with the two 16-bit halves
swapped. The two bugs cancel exactly:

```js
function sum32(a, b) {
  return (a & 0xffff) + (b >>> 16) + (((a >>> 16) + b & 0xffff) << 16) & 0xffffffff;
}
let allMatch = true;
for (let i = 0; i < 200000; i++) {
  const hash = (Math.random() * 4294967296) >>> 0;
  const viaSum32  = sum32(hash, 0x6b64e654) >>> 0;
  const viaCorrect = (hash + 0xe6546b64) >>> 0;
  if (viaSum32 !== viaCorrect) { allMatch = false; break; }
}
console.log(allMatch);
```
```text
true
```
`sum32(hash, 0x6b64e654) === (hash + 0xe6546b64) mod 2^32` for all 200,000 sampled 32-bit inputs.
The digest MurmurHash3 actually produces is correct; the general-purpose-looking helper it is built
from is not. Anyone reusing `sum32` elsewhere, or "correcting" the constant to the textbook value,
breaks every hash the library has ever produced.

**Why the suite misses it.** `sum32` is private, called from exactly one call site with exactly one
constant, and the two errors are numerically self-cancelling at that call site — there is no
observable symptom for any test to catch, deterministically, ever, at this call site. We confirmed
this is not a "the suite happens not to test it" situation but a "there is nothing to test" one, by
swapping the constant to the textbook value: `test/bloom-filter.js` (the only consumer) goes from
6/6 to 4/6 passing, while replacing `sum32(hash, N)` with plain `hash + 0xe6546b64` leaves it at 6/6
— i.e. the *bug* is required for the shipped constant to keep working, and the *shipped constant* is
required for the bug to be invisible. Neither survives being changed alone.

**How our port handles it.** Reproduced verbatim, both halves pinned by tests.

---

## 4. `SparseSet.add()` past capacity corrupts the set three different ways at once

**File:** `sparse-set.js` · **Severity:** silent corruption plus a broken invariant (`size` can
exceed `length`).

```js
const SparseSet = require('mnemonist/sparse-set');
const s = new SparseSet(10);
s.add(300);
console.log(s.size, Array.from(s.dense).slice(0, 3));
console.log(s.has(300), s.has(44));
```
```text
1 [ 44, 0, 0 ]
false false
```

```js
const u = new SparseSet(2);
u.add(100); u.add(101); u.add(102); u.add(103);
console.log(u.size, [...u]);
```
```text
4 [ 100, 101, undefined, undefined ]
```

**What upstream does wrong.** Neither of `add`'s guards fires for an out-of-range member, because
`sparse[m]` on an unwritten index is `undefined`, and every relational comparison against
`undefined` is `false`:

```js
this.dense[this.size] = member;    // truncates: add(300) on a length-10 set stores 44 (300 & 0xff-ish index wrap)
this.sparse[member]   = this.size; // dropped: an out-of-range typed-array store is a silent no-op
this.size++;                       // happens anyway
```

The member is stored under the wrong value, counted, and unfindable under either its real name or
the truncated one. The second-order effect is worse: because `size` increments unconditionally,
`size` can exceed `length` — and `values()`/spread freeze `size` while `dense` is a fixed-length
typed array, so iterating past `length` yields `undefined` for every excess slot.

**Why the suite misses it.** `test/sparse-set.js` never calls `add` with a member at or above the
set's configured length.

**How our port handles it.** Reproduced: every step here is a well-defined typed-array read, a
truncating store, or a silently dropped store, so the faithful behaviour is directly expressible —
and cheaper to implement than adding a guard upstream itself doesn't have.

---

## 5. `DefaultMap.get` tests the *value*, not the key — an unbounded, self-healing size drift

**File:** `default-map.js` · **Severity:** silent, unbounded state corruption; masks its own
evidence.

```js
const DefaultMap = require('mnemonist/default-map');
let calls = 0;
const m = new DefaultMap(() => { calls++; return undefined; });
m.set('a', undefined);
console.log('after set: ', m.size, m.items.size);
m.get('a');
console.log('after get1:', m.size, m.items.size, calls);
m.get('a');
console.log('after get2:', m.size, m.items.size, calls);
m.delete('a');
console.log('after del: ', m.size, m.items.size);
```
```text
after set:  1 1
after get1: 2 1 1
after get2: 3 1 2
after del:  0 0
```

**What upstream does wrong.**

```js
DefaultMap.prototype.get = function(key) {
  var value = this.items.get(key);
  if (typeof value === 'undefined') {    // asks about the VALUE, not items.has(key)
    value = this.factory(key, this.size);
    this.items.set(key, value);
    this.size++;                         // a counter, where set/delete read items.size directly
  }
  return value;
};
```

Any key whose stored value is `undefined` makes every subsequent `get` believe the key is missing:
the factory re-runs (so a stateful factory, e.g. an auto-incrementing one, advances on a *read*),
and `size` grows once per read rather than once per entry. The drift is silent and **self-healing**
— any later `set` or `delete` recomputes `size` from `items.size` and snaps it back to correct — so
an interleaved program shows a `size` that is sometimes right, with nothing to distinguish which.

**Why the suite misses it.** None of the seven assertions in `test/default-map.js` ever reads a
value of `undefined` back out and calls `get` on that key a second time. Confirmed with this
project's own gate-6 falsification: sabotaging the port to use `items.has` instead of the value
check — the *more correct* behaviour — leaves the original 7-assertion suite at 7 passing, while our
differential fuzzer (comparing the port against real upstream) catches the divergence in 136 cases
(0.1s), minimised to two operations.

**How our port handles it.** Reproduced: the correction is what a careful porter writes by
accident — deriving `size` from the entry count is tidier and matches what the name suggests — which
is exactly why it had to be checked against the real behaviour rather than assumed.

---

## 6. A token equal to the trie's own sentinel silently deletes the word it names

**File:** `trie-map.js` (and `trie.js`, same engine) · **Severity:** silent, permanent data loss;
`size` counts the loss as a gain.

```js
const TrieMap = require('mnemonist/trie-map');
const t = new TrieMap();
t.set('a', 'word-a');
t.set('a' + TrieMap.SENTINEL + 'b', 'word-a0b');
console.log(t.size);
console.log(t.get('a' + TrieMap.SENTINEL + 'b'));
console.log(JSON.stringify(t.root));
```
```text
2
undefined
{"a":{" ":"word-a"}}
```

`t.size` reports 2, but only one word is actually reachable from the root. The second `set` call
returns normally and increments the counter for a value that was never stored anywhere.

**What upstream does wrong.** `TrieMap.SENTINEL` (`String.fromCharCode(0)`) is not a reserved
namespace — it is an ordinary property key on the same plain object every real token is stored
under. `set`'s walk is `node = node[token] || (node[token] = {})`. If a real token equals
`SENTINEL` at a node that already stores a word, `node[token]` reads the **stored value** (a JS
primitive) instead of descending into a child object. `node` becomes that primitive, and every
subsequent `node[token] = {}` in the walk is a property write on a primitive — a silent no-op in
sloppy mode. The loop's local `node` variable keeps reassigning to a chain of freshly-created,
completely unlinked objects, so nothing from that point on is ever stored anywhere reachable from
`root`. The bookkeeping only sees "this final orphan object has no `SENTINEL` property of its own,"
which is true and tells it nothing about whether the value went anywhere.

**Why the suite misses it.** Neither `test/trie.js` nor `test/trie-map.js` ever embeds the sentinel
character inside a real token — every key in both suites is an ordinary word.

**How our port handles it.** This is the one bug in this document our port does *not* reproduce.
`mnemonist-core`'s trie node stores its value and its children in separate fields rather than a
shared keyspace, so both operations in the sequence above succeed and are independently retrievable
— a documented, deliberate structural divergence, not a silent improvement.

---

## 7. `LRUCache.setpop` silently drops the eviction report for a falsy evicted key

**File:** `lru-cache.js`, `lru-map.js`, and both `-with-delete` siblings · **Severity:** silent,
and the trigger (`0`, `''`, `false`, `NaN`, `null`, `undefined` as a key) is realistic production
input, not an edge case.

```js
const LRUCache = require('mnemonist/lru-cache');
const cache = new LRUCache(3);
cache.set(0, 'a'); cache.set(1, 'b'); cache.set(-1, 'c');
console.log(cache.setpop('d', 'e'));
console.log(cache.has(0));
```
```text
null
false
```

Key `0` really was evicted to make room, but the caller is told `null` — indistinguishable from "no
eviction happened."

**What upstream does wrong.**

```js
// setpop's eviction branch:
if (oldKey) {
  return {evicted: true, key: oldKey, value: oldValue};
}
else {
  return null;
}
```

`if (oldKey)` is JavaScript truthiness, not "was an eviction reported." Any falsy key that gets
evicted is reported identically to no eviction at all.

**Why the suite misses it.** All three `setpop` blocks in `test/lru-cache.js` evict or overwrite a
plain non-empty string. Found instead by this project's differential fuzzer on the very first
generated case for this grammar — the fuzz key pool deliberately includes falsy values.

**How our port handles it.** Reproduced deliberately, and it cost us idiom to do it: the port's core
`LruCache::set_pop` has no concept of JS truthiness and correctly reports every eviction regardless
of key — which is *more correct than upstream* and therefore, under a bug-for-bug contract, itself a
defect. The bridge re-introduces the bug with an explicit `is_js_truthy` check on the evicted key,
gating whether the eviction is reported at all.

---

## 8. `BiMap.clear()` resets only one of its two size counters

**File:** `bi-map.js` · **Severity:** silent, self-healing counter drift.

```js
const BiMap = require('mnemonist/bi-map');
const m = new BiMap();
m.set('a', 'a');
m.clear();
console.log(m.size, m.inverse.size);
```
```text
0 1
```

**What upstream does wrong.**

```js
function clear() {
  this.size = 0;
  this.items.clear();
  this.inverse.items.clear();
}
```

Both backing `Map`s really are emptied, but only `this.size` is reset — `this.inverse.size` is left
stale. The drift heals itself on the next `set`/`delete` on either side, because both of those
recompute both counters from the live maps — except a `delete` of an absent key, which returns
`false` before touching either counter, so `clear()` immediately followed by a no-op `delete()`
leaves the stale counter stale for one extra operation.

**Why the suite misses it.** `test/bi-map.js` never reads `.inverse.size` after a `clear()`. Found
by this project's differential fuzzer, and worth recording as a caution about our own process: a
naive first port derived both sizes from the underlying map's length, which "healed" the desync —
*more correct than upstream* and therefore itself a bug — caught by the fuzzer in 18 cases (0.3s).
The second attempt resynced both counters unconditionally after every `set`/`delete`, which
re-healed the one-extra-operation case above; caught in 177 cases (0.3s) on the very next campaign.
The final, correct port only resyncs on an actual removal, matching upstream's own conditional
exactly.

**How our port handles it.** Reproduced, on the third attempt.

---

## 9. `SuffixArray` loses its DC3 recursion sentinel at specific lengths

**File:** `suffix-array.js` · **Severity:** silently wrong output; **12% of inputs wrong** at the
affected residue class.

```js
const SuffixArray = require('mnemonist/suffix-array');
console.log(new SuffixArray('aaaaaaa').array);
```
```text
[
  6, 5, 3, 0,
  2, 4, 1
]
```
The correct suffix array for seven repeated characters is `[6,5,4,3,2,1,0]`.

**What upstream does wrong.** The DC3 algorithm recurses on a reduced string built by concatenating
the ranks of the "≡1 mod 3" suffixes with the ranks of the "≡2 mod 3" suffixes — sound only if the
first half ends in a symbol nothing else can equal. `al = (2 * l / 3) | 0` omits exactly the
position that would carry that terminator when the input length is ≡1 (mod 3), so the two halves run
together once the recursion actually fires (i.e. once some triple repeats). Exhaustive testing over
binary strings up to length 16 finds failures at lengths 7, 10, 13 and 16 — all ≡1 (mod 3) — and
nowhere else; measured 12% wrong over 10,000 random 3-letter inputs at that residue, 0% at the other
two.

**Why the suite misses it.** `test/suffix-array.js` has one length-22 input (≡1 mod 3, so nominally
in the affected class), but it is `'This is a long string.'`, which has no repeated trigram — so the
recursive branch this bug lives in never runs. The suite is one repeated trigram away from having
caught this.

**How our port handles it.** Reproduced: the DC3 recursion boundary is ported with the identical
omission.

---

## 10. `FixedCritBitTreeMap` has no capacity guard at all

**File:** `fixed-critbit-tree-map.js` · **Severity:** silent corruption that later becomes a crash
with a misleading message. Upstream's own source admits the gap in a comment.

```js
const FixedCritBitTreeMap = require('mnemonist/fixed-critbit-tree-map');
const t = new FixedCritBitTreeMap(4);
t.set('a', 1); t.set('ab', 2); t.set('abc', 3); t.set('abcd', 4);
console.log('at capacity:', t.size);
t.set('abcde', 5);
console.log('past capacity, no error:', t.size);
console.log(t.get('abcd'), t.get('abcde'));
console.log(t.get('a'), t.get('ab'), t.get('abc'));
t.set('abcdef', 6);
```
```text
at capacity: 4
past capacity, no error: 5
undefined undefined
1 2 3
```
```text
Uncaught TypeError: Cannot read properties of undefined (reading 'length')
    at findCriticalBit (fixed-critbit-tree-map.js:55:20)
    at FixedCritBitTreeMap.set (fixed-critbit-tree-map.js:199:17)
```

**What upstream does wrong.** The constructor's own comment says `// TODO: yell if capacity is
already full!` — and nothing does. `lefts`/`rights` are real, fixed-size typed arrays sized
`capacity - 1`; inserting past capacity writes an internal node's children past the end of those
arrays, which JavaScript silently drops rather than growing or throwing. `get`/`has` degrade
silently on the corrupted node (returning "not found" for entries that really were inserted); the
*next* insert that has to walk through the corrupted node throws a `TypeError` several stack frames
away from anything mentioning capacity.

**Why the suite misses it.** `test/fixed-critbit-tree-map.js` never inserts more than `capacity`
distinct keys; its "bad arguments" test only covers the constructor's own numeric-capacity
validation, a different check entirely.

**How our port handles it.** Reproduced as a typed error (`Error::Corrupted`, carrying upstream's own
message text) rather than a Rust panic — raising the *outcome* upstream's crash reaches, without a
panic unwinding across the FFI boundary, which napi does not catch on a synchronous call and which
would abort the whole Node process instead of throwing a catchable exception.

---

## 11. A re-entrant `clear()` drives `FibonacciHeap`'s size negative, then crashes the next call —
## two different ways, from two different methods

**File:** `fibonacci-heap.js` · **Severity:** state corruption (`size` becomes a real, JavaScript-
legal negative number) that later crashes with two different, unrelated-looking `TypeError`s.

**Repro A — from inside `pop`:**
```js
const FibonacciHeap = require('mnemonist/fibonacci-heap');
let armed = false;
const heap = new FibonacciHeap((a, b) => {
  if (armed) { armed = false; heap.clear(); }
  return a < b ? -1 : a > b ? 1 : 0;
});
heap.push(5); heap.push(3); heap.push(8); heap.push(1);
armed = true;
console.log(heap.pop(), heap.size);
heap.pop();
```
```text
1 -1
Uncaught TypeError: Cannot read properties of null (reading 'child')
```

**Repro B — from inside `push`:**
```js
let armed = false;
const heap = new FibonacciHeap((a, b) => {
  if (armed) { armed = false; heap.clear(); }
  return a < b ? -1 : a > b ? 1 : 0;
});
heap.push(5);
armed = true;
heap.push(3);
console.log(heap.root === null, typeof heap.min, heap.size);
heap.pop();
```
```text
true object 1
Uncaught TypeError: Cannot read properties of null (reading 'right')
```

**What upstream does wrong.** `pop`'s tail decrements `size` *after* calling `consolidate`:
`if (...) consolidate(this); this.size--;`. A comparator invoked from inside `consolidate` — no more
exotic than any other re-entrant comparator this codebase already allows — can legally call
`heap.clear()`, which sets `size = 0` mid-call; the pending `this.size--` then computes `0 - 1`,
a real value JavaScript holds without complaint. The *next* `pop()` doesn't see an empty heap either
— its guard is `if (!this.size) return undefined;`, and `-1` is truthy — so it proceeds into a heap
whose fields `clear()` already nulled and crashes several lines later.

`push` has the same shape from a different angle: `mergeWithRoot(this, node)` sets `this.root`
*before* the comparator runs, but `this.min = node` is assigned *after*. A `clear()` fired from the
tie-break comparator leaves `root === null` (never repaired) while `min` becomes a live node
(assigned after the clear) — an internally inconsistent pair that reaches a *different* null
dereference on the next `pop`.

**Why the suite misses it.** `test/fibonacci-heap.js` never uses a mutating comparator. More
tellingly, this project's own 12 native tests and 6-block boundary suite for this module also could
not have caught the *sibling* bug we deliberately sabotaged for gate 6 (flipping `push`'s tie-break
comparison) — the values it reorders are equal, so no expected-value assertion can observe the
difference. The differential fuzzer caught that one in 425 generated cases; only a reference
implementation comparison can see a tie-break reordering equal values, which is the sharpest
evidence in this whole project that a fuzzer and a test suite are not redundant instruments.

**How our port handles it.** Reproduced: `size` is a signed `i64`, not an unsigned type, specifically
so it can hold `-1` the way upstream's own arithmetic can drive it there. The follow-on crash is
reproduced as a Rust panic carrying upstream's exact `TypeError` text.

---

## 12. `StaticDisjointSet.union` compares the ranks of the *items*, not their *roots*

*(included despite being a performance bug rather than a correctness one — see below for why)*

**File:** `static-disjoint-set.js` · **Severity:** the union-by-rank heuristic is silently disabled;
results stay correct, but `find()` can degrade toward O(n).

```js
const StaticDisjointSet = require('mnemonist/static-disjoint-set');
const s = new StaticDisjointSet(4);
s.union(0, 1);   // tie: parents[1]=0, ranks[0]++  -> ranks[0]=1, covers {0,1}
s.union(2, 3);   // tie: parents[3]=2, ranks[2]++  -> ranks[2]=1, covers {2,3}
console.log(Array.from(s.ranks), Array.from(s.parents));

s.union(1, 2);   // 1 is a non-root member of {0,1}; 2 is the root of {2,3}
console.log(Array.from(s.ranks), Array.from(s.parents));
```
```text
[ 1, 0, 1, 0 ] [ 0, 0, 2, 2 ]
[ 1, 0, 1, 0 ] [ 2, 0, 2, 2 ]
```

The third `union` call reads `ranks[1]` (item `1`, a non-root leaf, whose rank was never touched and
stays `0`) instead of `ranks[find(1)]` (the actual root, rank `1`). It compares `0 < 1` and attaches
the *entire first tree* under the second — and because the branch taken was the `<` branch, not the
tie branch, **`ranks[2]` is never incremented**, even though the merged tree now has four elements.
The rank array no longer reflects true subtree height at all.

**What upstream does wrong.**
```js
var xRoot = this.find(x), yRoot = this.find(y);
var xRank = this.ranks[x],        // reads x, not xRoot
    yRank = this.ranks[y];        // reads y, not yRoot
if (xRank < yRank)      this.parents[xRoot] = yRoot;
else if (xRank > yRank) this.parents[yRoot] = xRoot;
else { this.parents[yRoot] = xRoot; this.ranks[xRoot]++; }   // writes xRoot
```
Reading at `x`/`y` but writing at `xRoot`/`yRoot` is internally inconsistent. Because non-root ranks
are never subsequently touched, they stay `0` forever, and the tie branch (the only one that ever
increments a rank) fires almost every time two non-singleton trees merge — union-by-rank is
effectively disabled.

**Why we include a performance bug in this list, and rank it last.** It is the one bug in this
document that our own instruments could never have found by running programs against it: `find()`
returns the correct root regardless of which tree got attached to which, so no differential
comparison against upstream will ever disagree, however many operations it runs. Only reading the
source catches this class of bug, which is worth stating for anyone weighing how much a clean fuzz
campaign proves.

**How our port handles it.** Reproduced: `find(x)` returns a root, and which element becomes root is
externally observable, so "fixing" the rank comparison would be a real behavioural divergence, not
just an internal cleanup.

---

## The rest, compressed

Every row below was independently confirmed against real Node 24.18.1 while preparing this document
(not merely carried over from development notes). "Reproduced" means the port matches upstream's
buggy behaviour on purpose; "narrowed" means the port makes the triggering state harder or impossible
to reach without changing observable behaviour on any input upstream's own tests cover.

| Bug | File | What happens | Port disposition |
|---|---|---|---|
| `SparseSet.delete` past capacity | `sparse-set.js` | writes a string-keyed expando (`sparse.undefined`) onto what should be a typed-array index, rather than landing on a numeric index | Reproduced — caught our own first draft, which wrote the "obvious" wrong index |
| `SparseQueueSet.dequeue`'s absence sentinel doesn't fit its own array | `sparse-queue-set.js` | at capacity exactly 256 or 65536, the "member is absent" sentinel truncates to a live slot value, producing a false-positive `has()` that can never be re-enqueued | Reproduced |
| `SparseQueueSet.enqueue` never checks the ring is full | `sparse-queue-set.js` | one out-of-range `enqueue` silently evicts a legitimately queued member and leaves `size` above `capacity` | Reproduced |
| `SparseQueueSet` with `capacity === 0` | `sparse-queue-set.js` | modulo-by-zero `NaN` index turns every read/write into a dropped expando write; `start` climbs without bound | Reproduced |
| `HashedArrayTree.pop` reads the wrong block once more than one block exists | `hashed-array-tree.js` | returns a stale/wrong element and can return the same value twice | Reproduced |
| `HashedArrayTree`'s bounds guard admits `index === length` | `hashed-array-tree.js` | `get(0)` on a brand-new tree returns `0`, not `undefined`; `set(length, v)` writes without growing `length` | Reproduced |
| `BitSet`/`BitVector.reset` omits the `>>> 0` that `set`/`flip` apply | `bit-set.js`, `bit-vector.js` | `size` can drift to a **negative** number after resetting an already-clear high bit | Reproduced |
| `BitSet`/`BitVector.select` doesn't advance across skipped all-zero words | `bit-set.js`, `bit-vector.js` | answers off by a multiple of 32 whenever a query crosses a zero word | Reproduced |
| `bitwise.msb32` | `utils/bitwise.js` | reports "no bits set" (`0`) for every 32-bit input with the sign bit set | Reproduced |
| `bitwise.criticalBit32Mask` | `utils/bitwise.js` | a stray `& 0xffffffff` re-signs its own unsigned result (`criticalBit32Mask(1,2) === -3`) | Reproduced |
| `BitVector.pop`/`push(0)` | `bit-vector.js` | `pop` never decrements `size`; `push(0)` never clears the released slot, so a popped bit can resurface | Reproduced |
| `length % 32 \|\| 32` on length zero | `bit-set.js`, `bit-vector.js` | a zero-length `BitVector` with spare capacity iterates 32 phantom bits | Reproduced |
| `BitSet.set` past `length` but inside the last word | `bit-set.js` | `size` counts a bit that `rank`/`select`/iteration can never see | Reproduced |
| `binary-search.lowerBoundIndices` defaults `hi` from the wrong array | `utils/binary-search.js` | walks off the end of a shorter `indices` array when `array` is longer | Reproduced |
| `binary-search.search` with an out-of-range `hi` | `utils/binary-search.js` | reports a "match" at whatever midpoint the bad range computes to | Reproduced |
| `hash-tables.linearProbing` on a zero-length table | `utils/hash-tables.js` | `get`/`has`/`set` loop **forever** (a real DoS, confirmed by timeout) | NOT reproduced — a port that hangs on demand is not a behaviour worth carrying; the port guards all three entry points |
| `hash-tables`: key `0` occupies a slot that still reads as empty | `utils/hash-tables.js` | an entry stored under key `0` is silently overwritten by any later key that collides with slot 0 | Reproduced |
| `BloomFilter` with zero hash functions | `bloom-filter.js` | `test()` answers `true` for every input, vacuously — the filter accepts, e.g., `errorRate: 0.5` despite its own error message demanding an integer | Reproduced |
| `BloomFilter` hashes every non-string item identically | `bloom-filter.js` | numbers, booleans and `''` all hash as the empty byte sequence and are indistinguishable in the filter | Reproduced |
| `BloomFilter` with `errorRate > 1` | `bloom-filter.js` | throws a `RangeError` at large capacity, but silently builds an always-true filter (same root cause as above) at small capacity — same invalid input, two different failure modes depending on an unrelated parameter | Reproduced |
| `StaticIntervalTree` on zero intervals | `static-interval-tree.js` | crashes three stack frames deep with a `TypeError` naming neither "empty" nor "interval" | Narrowed — the port raises a named `EmptyIntervals` error instead of letting a Rust panic cross the FFI boundary, where it would abort the process rather than throw |
| `Vector.get`/`set` admit `index === length` | `vector.js` | writes/reads one slot past the logical end without growing `length` | Reproduced |
| `Vector`'s growth carries a popped slot's stale data forward | `vector.js` | combined with the bug above, a value already `pop()`ped can resurface after the vector grows | Reproduced |
| Heap: a throwing comparator leaves `size` one behind `items.length` (`push`) | `heap.js` | every later `pop` under-reports by one; `#.consume` over-returns | Reproduced |
| Heap: `nsmallest`/`nlargest(1, empty-or-typed-source)` | `heap.js` | answers with the internal `Infinity`/`-Infinity` sentinel itself, not "nothing found"; on a typed array the sentinel narrows to a plausible-looking `0` | Reproduced |
| Heap: an `Infinity`-valued element resets the sentinel | `heap.js` | with a custom comparator, an `Infinity` element can silently overwrite the true answer | Reproduced |
| `FixedReverseHeap`'s capacity guard is `&&` where `\|\|` was needed | `fixed-reverse-heap.js` | `new FixedReverseHeap(Array, 0)` is accepted and then silently discards every `push` | Reproduced |
| `FixedReverseHeap#clear` leaves `items` in place | `fixed-reverse-heap.js` | `peek()` after `clear()` returns a discarded item; `consume()`/`toArray()` correctly see it as gone | Reproduced |
| `MaxHeap.prototype = Heap.prototype` | `heap.js` | `instanceof` cannot distinguish a min-heap from a max-heap; `.constructor` is always `Heap` | Reproduced |
| `#.consume` zeroes `size` before draining | `heap.js` | a throwing comparator mid-consume leaves the heap reporting empty while still holding items | Reproduced |
| Heap comparator return values are coerced, never checked | `heap.js` | a comparator returning a `BigInt` sorts correctly; one returning a non-numeric string makes every comparison "equal" silently | Reproduced |
| A falsy comparator argument silently takes the default | `heap.js`, `fixed-reverse-heap.js` | `new Heap(0)`/`new Heap(null)` are accepted as "use the default"; `new Heap('x')` throws — the type guard only ever sees a *truthy* non-function | Reproduced |
| `Heap.nsmallest(cmp, -Infinity, arrayLike)` | `heap.js` | the scan loop's index never advances (`-Infinity + 1 === -Infinity`); **hangs forever** | NOT reproduced, same reasoning as the hash-tables infinite loop |
| `sort/insertion.js` declares its loop counter as an undeclared global | `sort/insertion.js` | a comparator that re-enters the sorter mid-comparison (via `valueOf`) corrupts the outer sort's counter; confirmed: `[1, 2, 3, 5]` expected, `[1, 5, 3, 2]` produced | Reproduced |
| `sort/quick.js`'s partition stack is shared module state | `sort/quick.js` | a re-entrant comparator corrupts a concurrent sort's partition bounds; measured 38 of 40 elements out of order | Reproduced |
| `X.from(iterable)` calls an `iterables.forEach` that doesn't exist | `fixed-stack.js`, `fixed-deque.js`, `circular-buffer.js` | `FixedStack.from(new Set(...))` throws `TypeError: iterables.forEach is not a function` — the documented "any iterable" API only ever works for arrays and typed arrays | Reproduced |
| `FixedStack.prototype.forEach` walks capacity, not `size` | `fixed-stack.js` | an under-full stack's callback is invoked `capacity` times, fed unused slots first | Reproduced |
| `FixedDeque.prototype.get` is bounded by capacity, has no lower bound | `fixed-deque.js`, `circular-buffer.js` | returns already-popped or already-shifted-out elements instead of `undefined` | Reproduced |
| `X.from` with a `DataView` | `fixed-stack.js`, `fixed-deque.js`, `circular-buffer.js` | `isArrayLike` accepts it via `ArrayBuffer.isView`, but a `DataView` has no `.length` — `size` becomes `undefined` and `toArray()` returns one spurious `undefined` element | Narrowed — a Rust `usize` cannot hold `undefined`, so this specific state is not reachable |
| `CircularBuffer.from` bypasses its own overwrite semantics | `circular-buffer.js` | can leave `size > capacity` on the one class that exists to prevent that | Reproduced |
| k-way `merge`/`unionUnique` throw when filtering an empty array out leaves ≥3 arrays | `utils/merge.js` | `merge([], [1,2,3],[4,5,6],[4,7])` throws `TypeError` from a stale cached length | Reproduced (as a named error at the FFI boundary, not a panic) |
| `lru-map.js`'s own `.from` names the wrong module in its error | `lru-map.js:241` | the thrown message says `mnemonist/lru-cache.from`, not `lru-map.from` | Reproduced |
| Trie: an open `values`/`prefixes`/`keys`/`entries` walk can still yield a word `delete` just removed | `trie-map.js`, `trie.js` | pruning an ancestor doesn't touch the orphaned node object the walk already holds a live reference to | Not reproduced — our walk re-navigates from the root by token path rather than holding a live object reference, a documented structural divergence forced by the FFI boundary |
| `InvertedIndex.prototype.forEach` never calls its callback | `inverted-index.js` | `this.documents` inside `forEach` refers to the zero-argument *method*, not the `items` property — `.length` is always `0` | Reproduced |
| `LinkedList.prototype.shift` never updates `tail` | `linked-list.js` | emptying a list via `shift()` leaves `last()` returning the just-removed item until the next insert | Reproduced |
| `DefaultWeakMap.prototype.get` tests the value, not key presence | `default-weak-map.js` | the factory re-runs on every `get` of a key whose stored value is `undefined`, even though `has()` correctly reports it present | Reproduced |
| `FixedCritBitTreeMap.root` is `0` fresh off the constructor, `null` after `clear()` | `fixed-critbit-tree-map.js` | two different JS values both mean "empty tree," assigned by two places that never agree; behaviourally unobservable except by reading `.root` directly | Reproduced |
| `forEach` on a truthy primitive dies inside the `in` operator | `obliterator/foreach.js` | `forEach(5, cb)` throws `TypeError: Cannot use 'in' operator to search for 'Symbol(Symbol.iterator)' in 5` — a caller sees an error naming V8's operator, not the library | Reproduced |
| `toArray` produces a sparse array when its length guess is wrong | `mnemonist/utils/iterables.js` | `toArray({length: 5})` returns `[5, <4 empty items>]` — the plain-object `forEach` branch enumerates `length` itself as a property | Reproduced |

---

## Lower-confidence candidates

These are real, executable, and confirmed exactly as described — but each has a plausible reading as
intentional design rather than a defect, so we are not resting the submission's credibility on them.

**`iter`/`forEach` disagree on whether a plain object is iterable** (`obliterator`). `take({a:1})`
throws (`iter.js` has no plain-object branch); `forEach({a:1}, cb)` iterates the values. Confirmed:
```js
const iter = require('obliterator/iter');
try { iter({a:1,b:2}); } catch (e) { console.log(e.message); }
```
```text
obliterator: target is not iterable nor a valid iterator.
```
Plausibly intentional — `iter` must return a genuine iterator, `forEach` only needs to visit — but
two helpers in the same small library disagreeing about the same input is at minimum a documentation
gap.

**`forEach`'s falsy-input guard rejects the empty string** (`obliterator`). `forEach('', cb)` throws;
`forEach('a', cb)` correctly iterates one character. Confirmed:
```js
const forEach = require('obliterator/foreach');
try { forEach('', () => {}); } catch (e) { console.log(e.message); }
```
```text
obliterator/forEach: invalid iterable.
```
An empty string is a legitimately iterable value that should yield zero times, not throw. Plausibly
an intentional "don't accept nothing" guard that just wasn't thought through for this case.

**`forEach` calls `.toString()` on arbitrary input during type dispatch** (`obliterator`). An object
whose `.toString()` returns the exact string `'[object Arguments]'` gets routed into the
arguments-like branch and has its real enumerable properties skipped entirely:
```js
const forEach = require('obliterator/foreach');
const obj = { a: 1, toString() { return '[object Arguments]'; } };
const seen = [];
forEach(obj, (v, k) => seen.push([v, k]));
console.log(seen);
```
```text
[]
```
Real and reproducible, but it requires an adversarial `toString`, and the practical impact is low.

## Judged not to be bugs

Two candidates we considered and are deliberately leaving out of the claims above, because on
inspection they are unreachable through any public sequence of calls:

**`Stack.prototype.values()` reads `this.items.length` instead of `this.size`.** Every other method
in `stack.js` is written against `this.size`, so this looks like an inconsistency worth flagging.
But `Stack` backs its storage with a genuine, unbounded JS `Array`, and both `push` and `pop` mutate
`items` and `size` in lockstep on every call — there is no code path in the file that can make
`items.length !== this.size`. The inconsistency is real in the source; it has no observable
consequence.

**`obliterator/take`'s `n`-tracking guard (`if (i !== n) array.length = i`) is always true when `n`
is omitted**, because `n` is then `undefined` and `i` is a number. Confirmed by reading the source
against the resulting behaviour: with `n` omitted, `array` starts as `[]` (not pre-sized), and
setting `array.length = i` at the end is a no-op — the array already has length `i`. The guard
doesn't express what it appears to intend, but there is no input that makes it produce a wrong
result.

We also read `set.js` end to end — 356 lines, no shared mutable state, no typed arrays, no index
arithmetic, no re-entrancy hazards — and found nothing to file. Worth recording rather than omitting:
not every file in this library has a bug in it.

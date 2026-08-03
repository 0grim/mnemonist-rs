# bloom-filter

Upstream: `bloom-filter.js` (186 LOC) + `utils/murmurhash3.js` (87 LOC) ·
`test/bloom-filter.js` (88 lines, 18 assertions)

Require-closure: `bloom-filter.js` → `utils/murmurhash3.js` + `obliterator/foreach`. The unit is
therefore **two** upstream files; `obliterator/foreach` is the third member and
was already ported to the boundary (`crates/mnemonist-napi/src/foreach.rs`), which is where a
JavaScript-value coercion belongs. It is reused here, not reimplemented.

Port: `crates/mnemonist-core/src/structures/bloom_filter.rs` +
`crates/mnemonist-core/src/utils/murmurhash3.rs` · bridge `crates/mnemonist-napi/src/bloom_filter.rs`
· shim `tests/bridge/bloom-filter.js`

---

## What upstream tests

Six `it()`s:

* **invalid options** — four `assert.throws`: `-34`, `{capacity: -34}`, no argument, and
  `{capacity: 3, errorRate: -45}`.
* **correct settings** — `new BloomFilter(3)` has `data.length === 4` and `hashFunctions === 7`.
* **adding items** — three adds into a capacity-3 filter, each checked against a hardcoded
  four-byte array.
* **more items** — 50 items into a capacity-50 filter, one hardcoded 68-byte array.
* **testing items** — `add('hello')`, then `test('hello') === true` and `test('world') === false`.
* **`from`** — a `Set` (capacity inferred from `.size`), the `/capacity/` throw for a bare
  iterator, and the explicit-capacity form.

`utils/murmurhash3.js` has **no test file at all** and no direct assertion anywhere; its only
coverage is whatever the byte arrays above happen to pin.

## What upstream does NOT test

1. **Any `murmurhash3` output.** Not one direct assertion, in the whole repo. The function is
   verified only through four frozen byte arrays of a capacity-3 filter.
2. **Any input above U+007F.** Every item is ASCII, so the fact that
   `stringToByteArray` produces a `Uint16Array` — and that `murmurhash3` then reads each 16-bit
   element *as if it were a byte*, overlapping neighbours — is never exercised. Nor is an astral
   character, which becomes **two** code units.
3. **Non-string items.** `add(42)` does not throw and does not hash 42; see BUG-BLOOM-FILTER-3.
4. **`clear`.** Defined, never called. It is also the one method that can throw after construction,
   because it re-derives the sizing rather than merely zeroing.
5. **`toJSON`.** Defined, never called.
6. **`errorRate` as anything but `-45`.** The three-way reading — omitted defaults, explicit `0`
   throws, explicit `NaN` **also** defaults — is entirely unexercised. So is any rate above the
   default.
7. **`errorRate >= 1`,** where the sizing goes negative. See BUG-BLOOM-FILTER-4.
8. **A `hashFunctions` of zero,** where the filter answers `true` to everything. See BUG-BLOOM-FILTER-2.
9. **Fractional and non-integer capacities.** The error message says "positive **integer**"; the
   check is `typeof capacity === 'number' && capacity > 0`, so `2.5` and `0.5` are both accepted and
   neither is tested.
10. **Any capacity but 3 and 50.**
11. **False negatives at scale.** One item is checked for presence. The single property a Bloom
    filter actually guarantees is tested once.
12. **The false-positive rate.** Never approached; a hash that had collapsed to a constant would
    pass every assertion above except the two byte arrays.

## What we test in addition

15 native tests in `bloom_filter.rs`, 7 in `murmurhash3.rs`, closing every gap above: `murmurhash3`
checked against 23 (seed, data) pairs including the exact negative seeds `hashArray` derives, a
fresh capacity-10 filter per item (including U+0000, non-ASCII and an astral character) so a hash
defect can't hide behind cumulative state, the empty sequence treated as an ordinary item, `clear`
resetting bits while keeping the sizing, the error-rate option/default read, an error rate above 1
producing a `RangeError` only sometimes (BUG-BLOOM-FILTER-4), a zero-hash-function filter saying yes to everything
(BUG-BLOOM-FILTER-2), 15 (capacity, errorRate) pairs checked against Node, an infinite capacity producing an empty
filter, 200 items with zero false negatives, and a false-positive rate measured at 500 items in and
2,000 queries out, asserted under 5% against a nominal 0.5%. Full test-to-gap mapping: evidence
file.

The one that matters most is the fresh-filter-per-item test: upstream only ever checks *cumulative*
state on a capacity-3 filter, so a hash defect that happened to preserve those four bytes would go
unnoticed; a fresh filter per item makes each digest independently observable.

## Bugs this found

Four, all verified against Node 24.18.1, all reproduced.

### BUG-BLOOM-FILTER-1 — `sum32` is not a 32-bit adder, and a swapped constant hides it

```js
function sum32(a, b) {
  return (a & 0xffff) + (b >>> 16) + (((a >>> 16) + b & 0xffff) << 16) & 0xffffffff;
}
```

The correct form takes `b & 0xffff` for the low half and `b >>> 16` for the high half. This one has
them the wrong way round in *both* places, so it adds `b`'s **high** half to `a`'s low half and
`b`'s **low** half to `a`'s high half. `sum32(1, 1)` is `65537`.

It is called exactly once, with `n = 0x6b64e654` — MurmurHash3's published `0xe6546b64` with its
halves swapped. The two errors cancel exactly: `sum32(hash, 0x6b64e654)` is
`(hash + 0xe6546b64) mod 2^32` for every 32-bit `hash`, checked over 200,000 random inputs against
big-integer arithmetic.

So the digest is right, the helper is wrong, and the only thing holding them together is a constant
nobody would recognise as a typo. Anyone reusing `sum32` — it looks entirely general — gets nonsense,
and anyone "correcting" `n` to the published constant breaks every filter the library has ever
produced.

Demonstrated end to end through the *original* suite, as a control on the gate-6 falsification
below: replacing `sum32(hash, N)` with `hash + 0xe6546b64` (the unswapped constant) leaves all six
upstream tests green, while replacing it with `hash + N` (the swapped one) turns two of them red.

### BUG-BLOOM-FILTER-2 — a filter with zero hash functions says `true` to everything

`hashFunctions` is `(length * 8 / capacity * Math.LN2) | 0`, and nothing checks the result. When it
truncates to `0`, `add` writes no bits and `test` returns `true` **vacuously** — the loop it would
have returned `false` from never runs.

This is not an exotic corner:

```js
> var f = new BloomFilter(0.5);            // passes every validation upstream has
> f.hashFunctions                          // 0
> f.test('anything')                       // true
> f.test('literally anything else')        // true
```

`0.5` gets through because the check is `typeof capacity === 'number' && capacity > 0`, despite the
error message next to it saying "positive **integer**". `{capacity: 10, errorRate: 0.5}` reaches the
same state with a **non-empty** `data`, so it is not simply "an empty filter"; the bit array exists,
is all zeros, and every query says yes.

### BUG-BLOOM-FILTER-3 — every non-string item hashes identically

```js
function stringToByteArray(string) {
  var array = new Uint16Array(string.length);
  for (var i = 0; i < string.length; i++) array[i] = string.charCodeAt(i);
  return array;
}
```

On a number, `string.length` is `undefined`, `new Uint16Array(undefined)` has length `0`, and the
loop never runs. So `add(42)` hashes the empty sequence — the same sequence `add('')` hashes:

```js
> var f = new BloomFilter(3);
> f.add(42);
> f.test(7)       // true
> f.test(true)    // true
> f.test('')      // true
```

A filter of numbers reports every number, every boolean and the empty string as present. The
neighbouring cases are inconsistent rather than uniformly permissive, which is part of what makes it
a bug rather than a coercion policy: `add(null)` and `add(undefined)` throw a `TypeError` from the
property read, `add(['a'])` throws `string.charCodeAt is not a function`, and
`add(new String('hello'))` works and equals `add('hello')`.

### BUG-BLOOM-FILTER-4 — an `errorRate` above 1 is a raw `RangeError`, but only sometimes

`Math.log` of anything above 1 is positive, so `bits` goes negative and `new Uint8Array(-59)` throws
from the *allocator*:

```js
> new BloomFilter({capacity: 50, errorRate: 100})
RangeError: Invalid typed array length: -59
> new BloomFilter({capacity: 50, errorRate: 3})
RangeError: Invalid typed array length: -14
> new BloomFilter({capacity: 5, errorRate: 2})     // no error at all
BloomFilter { capacity: 5, errorRate: 2, hashFunctions: 0, data: Uint8Array(0) [] }
```

The third one is the interesting case: `(-7.2 / 8) | 0` truncates to `0`, so the same invalid option
yields a silent BUG-BLOOM-FILTER-2 filter instead of an error, and which of the two you get depends on the
capacity. Neither is the module's own error message, and `errorRate` is the one option upstream
believes it validates.

## Deliberate divergences

**DIV-BLOOM-FILTER-1 — items are `&[u16]`, not `&str`.** Upstream hashes a string's UTF-16 code units via
`charCodeAt`. Taking `&str` in the core would mean hashing UTF-8, which produces different bits for
every non-ASCII input and would silently make this a different filter. The `charCodeAt` conversion
is at the bridge, where the JavaScript values are.

**DIV-BLOOM-FILTER-2 — `murmurhash3` takes `&[u16]`, and the overlap is preserved.** Upstream's JSDoc says
`ByteArray` and its loop composes four *bytes* per 32-bit word, but its only caller hands it a
`Uint16Array`. So elements above `0xFF` overlap their neighbours' byte positions, and `data.length`
counts code units rather than bytes. Not corrected: it is the function's only observed input and
every published bit pattern depends on it.

**DIV-BLOOM-FILTER-3 — `BuildError` has three variants, not one message.** Upstream raises two *different*
JavaScript error classes here: its own `Error` for the two validation failures and the allocator's
`RangeError` for a negative length. The core has no exceptions, so the distinction is carried as
data and the bridge re-throws with upstream's message verbatim — which is what upstream's own
`assert.throws(..., /capacity/)` matches on.

**DIV-BLOOM-FILTER-4 — a `RangeError` is thrown as a napi `GenericFailure` carrying the right message.** napi has
no direct `RangeError` constructor. The message is upstream's, character for character
(`Invalid typed array length: -59`), and nothing in the upstream suite discriminates on the class.

**DIV-BLOOM-FILTER-5 — `#.data` hands back a fresh `Uint8Array` per read.** napi cannot lend out a typed array
whose backing store a later `add` may reallocate (`clear` genuinely reallocates). Every upstream
assertion reads it through `Array.from`, and `filter.data.length` is a length either way. What this
does *not* reproduce is upstream's `filter.data === filter.data`, which nothing checks.

**DIV-BLOOM-FILTER-6 — `from` collects before adding.** Upstream adds inside the `forEach` callback; the bridge
runs `crate::foreach::collect` — the same five-branch coercion, unmodified — and then adds. The
difference is observable only if a callback could reach the filter being built, and it cannot: the
filter is local to `from` and is not yet a JavaScript object. Same pattern as `Stack.from`.

**DIV-BLOOM-FILTER-7 — `inspect` is not ported.** No upstream assertion, no Rust equivalent.

## Fuzz + bench

### Fuzz

**Two campaigns, 1,385,597 operations, zero divergences.** Spec:
`crates/difffuzz/src/modules/bloom_filter.rs`. Full campaign table: evidence file.

The op alphabet is `add`, `test`, `clear` and `toJSON` — every method upstream defines apart from
the static `from` and the unported `inspect`. `data`, `capacity`, `errorRate` and `hashFunctions`
are observations, compared after every step. `clear` is an op rather than an observation because it
*mutates*: it re-derives `hashFunctions` and reallocates `data`, so putting it in the observation
set would wipe the filter before every comparison.

`add` and `test` take numbers and booleans as well as strings, deliberately — BUG-BLOOM-FILTER-3 is reachable only
if the grammar can express a non-string item. `null` and `undefined` are excluded because upstream
throws a `TypeError` there and the oracle compares thrown messages verbatim, which would turn an
engine-wording difference into a false divergence.

`errorRate` is held strictly below 1, an explicit exclusion: at or above it the sizing goes negative
and a large enough capacity throws from the *constructor*, which reaches the oracle's `init` and is
apparatus failure by protocol. That is BUG-BLOOM-FILTER-4, and it is pinned by a native test instead. The
zero-hash-function region (BUG-BLOOM-FILTER-2) is **not** excluded and is reached routinely.

A harness-side JSON number-encoding mismatch was found and fixed during this campaign's first run;
full account: log.

**Falsification (gate 6), two runs plus one control, each named before it was performed.** The
original test suite (gate 4): replacing `sum32`'s call with unswapped addition must break
`'should be possible to add items to the filter.'` — confirmed red (4 passing, 2 failing, that
assertion among them); reverted, 6 passing. The fuzz spec: an early `if hash_functions == 0 {
return false }` in `test` (i.e. "fixing" BUG-BLOOM-FILTER-2) must break a return-value divergence on a filter
whose `hashFunctions` truncated to zero — confirmed red, minimised to two lines; reverted, clean.
Control, to check the BUG-BLOOM-FILTER-1 cancellation rather than assert it: swapping in the *unswapped* constant
should change nothing if the cancellation is real — confirmed green, 6 passing, so the cancellation
is real. Full record: evidence file.

The first sabotage is worth one more sentence. `'should be possible to test items'` stayed **green**
under it — because it only checks self-consistency (`add x`, then `test x` is true and `test y` is
false), which a completely different hash satisfies just as well. Two of upstream's six cases can
detect a change to the hash at all, and both do it through a frozen byte array rather than through
any property of the filter.

### Bench

`bench/results.json` → `modules["bloom-filter"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 2,000 samples/side.

**`mixed-2e5`** — 200,000 mixed `add`/`test`-hit/`test`-miss (50/25/25), hex-encoded keys, capacity
200,000 at upstream's default 0.5% error rate. The filter is **prefilled to a stated 50% fill
ratio** before timing starts — an empty or near-empty filter answers every `test` the same way
all-zero bits would, trivially fast and proving nothing about the hashing/bit-setting this module
exists to measure. `test` queries split across two disjoint pools, both measured directly before
committing to the mix: the **hit** pool answers `true` **61.1%** of the time, and the **miss** pool
has a **0.028%** false-positive rate — both a genuine mix of true/false answers, not a degenerate
all-one-answer workload: the port is 1.7× faster at p50 (97.36 vs 163.93 ns/op), 1.4× faster at p99
(162.72 vs 235.92). No regressions. Checksum `30440`, identical on both sides. Full table: evidence
file.

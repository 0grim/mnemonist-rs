# utils/hash-tables

Upstream: `utils/hash-tables.js` (107 LOC) · **no test file of its own.**

Port: `crates/mnemonist-core/src/utils/hash_tables.rs`. No bridge, for the same reason as
`utils/binary-search`: the only file that requires it cannot run.

---

## Scope note: this is not a "unit"

Same standing as `utils/binary-search`. `test/_utils.js` is the only caller, its require-closure
needs `merge` and `iterables`, and a missing sibling makes the whole file fail with zero partial
credit. Gates **1**, **2**, **7** and **8** apply; gates 3, 4, 6, 9 and 10 have no target and this
file never appears in `tests/scope.txt` on its own.

One extra fact worth recording: **nothing in the shipped library calls these helpers.** `git grep`
across the published package finds exactly one caller, `benchmark/misc/hashmap.js`, which is not
published. So the file is public API with no internal consumer — the case where a defect can sit
indefinitely without any structure's test suite tripping over it.

## What upstream tests

One `it()`, `'should be possible to use linear probing.'`:

* eight `(key, value)` pairs inserted into `Uint32Array(8)` / `Uint32Array(8)`, filling the table
  exactly;
* every pair read back with `get`, and confirmed with `has`;
* a ninth `set` asserted to throw `/full/`;
* one `get` miss and one `has` miss, both against the now-full table.

`jenkinsInt32` is exercised only as the hash argument — **no assertion anywhere pins a single hash
value.**

## What upstream does NOT test

1. **Any `jenkinsInt32` output.** Not one. The function could return `key & 7` and the test would
   still pass, because a hash only has to be a function for linear probing to round-trip. That
   matters here: the mixed float/ToInt32 arithmetic in it is exactly the shape that broke
   `bitwise.msb32` (B-19).
2. **The resulting slot layout.** Only that reads round-trip, never *where* anything landed — so
   the probe order, the mask, and the wrap are all unpinned.
3. **Setting a key that is already present.** Every one of the eight keys is distinct, so the
   overwrite-in-place branch (`c === key` at the top of `linearProbingSet`) never runs.
4. **The key `0`.** It is the empty sentinel, and it is also a perfectly ordinary `Uint32Array`
   value. Nothing upstream stores it.
5. **A zero-length table.** Upstream hangs (see B-92).
6. **A non-power-of-two table.** `hash(key) & (n - 1)` is only a modulo for powers of two; upstream
   uses 8 and nothing else.
7. **Any table size other than 8.** No small tables, no large ones.
8. **Termination from an arbitrary starting slot on a full table.** The one full-table miss upstream
   checks starts wherever `jenkinsInt32(485385)` happens to point.

## What we test in addition

| gap | test |
|---|---|
| — (upstream's own case, transcribed) | `linear_probing_matches_the_upstream_suites_own_case` |
| 1 | `jenkins_int32_matches_node_24_18_1` — 28 inputs against real Node, including `0`, `±1`, `i32::MIN`, `i32::MAX`, `255/256`, `65535/65536` and every key upstream's own test uses |
| 2 | `the_upstream_pairs_land_in_a_known_layout` — the exact `keys`/`values` arrays upstream's eight pairs produce |
| 3 | `setting_an_existing_key_overwrites_in_place` |
| 4 | `the_key_zero_occupies_a_slot_that_still_reads_as_empty` |
| 5 | `a_zero_length_table_is_refused_rather_than_hung` |
| 6 | `a_non_power_of_two_table_still_terminates` — `n = 5`, layout pinned against Node |
| 7 | `round_trips_at_every_power_of_two_size` — `n` from 2 to 64 |
| 8 | `a_full_table_terminates_from_every_starting_slot` |

The `jenkinsInt32` table is the load-bearing one. It is what makes "matches upstream" an executed
comparison rather than an assertion, and it is the reason the port can use wrapping `u32` arithmetic
in place of upstream's float/ToInt32 alternation with a straight face.

## Bugs this found

**B-92 — `linearProbing.get`/`has`/`set` loop forever on a zero-length table.**
`i %= n` with `n === 0` is `NaN`; `keys[NaN]` is `undefined`, which is neither the key nor `0`; and
the "full turn" guard `i === j` can never be true because `NaN !== NaN`. So the `while (true)` never
exits. Confirmed by running

```js
require('mnemonist/utils/hash-tables').linearProbing.get(h, new Uint32Array(0), new Uint32Array(0), 1)
```

under `timeout 5 node`, which exited 124. Not reproduced — see D-45.

**B-94 — the key `0` occupies a slot that still reads as empty, and the next colliding insert
silently destroys it.** `0` is the empty sentinel *and* a storable `Uint32Array` value. The precise
behaviour is more interesting than "key 0 cannot be stored":

* `linearProbingSet(h, keys, values, 0, 42)` writes `keys[j] = 0`, leaving the table
  byte-identical to an untouched one.
* `linearProbingGet(...)` still finds it — `c === key` is tested *before* `c === 0`, so the read
  works.
* The next key that hashes to the same slot sees `c === 0`, treats the slot as free, and
  **overwrites**. The `42` is gone with no error, and `get(0)` now returns whatever sits in the next
  still-empty slot.

Verified on Node 24.18.1. Reproduced exactly, including the "still findable until it isn't" part.

## Deliberate divergences

**D-44 — a full table returns `Err`, not a thrown error.** `mnemonist-core` has no exceptions. The
error value is the `&'static str` `TABLE_IS_FULL`, holding upstream's message verbatim
(`mnemonist/utils/hash-tables.linearProbingSet: table is full.`) so a future bridge can re-throw it
unchanged and upstream's `assert.throws(..., /full/)` still matches.

**D-45 — a zero-length table is refused instead of hung.** B-92 is an infinite loop, and "reproduce
upstream bug-for-bug" does not extend to hanging the process — a fuzz campaign or a `cargo test`
would never terminate. `get`/`has` return "absent" and `set` returns `TABLE_IS_FULL`, all three
guarded before the probe starts. This is the one place the port is deliberately *more* terminating
than the original, and it is stated here rather than left implicit.

**D-46 — keys are `u32`, values are generic.** Upstream is untyped, but the sentinel comparison
`c === 0` and its one real call site both assume integer keys held in a typed array, and the
`Uint32Array` in its own test fixes the width. Making the key type generic would mean inventing a
"zero" trait for a function whose only observed key type is `u32`.

**D-47 — an out-of-range initial slot is probed past, not bounds-checked.** Only reachable with a
non-power-of-two table (see above), where `hash(key) & (n - 1)` can select a slot at or past the
end. Upstream reads `undefined`, treats it as "occupied but not equal", and probes on; the port does
the same through an `Option`-returning read. If the probe wraps all the way back to a starting slot
that was out of range, `set` refuses with `TABLE_IS_FULL` rather than writing out of bounds.

## Fuzz + bench

**Neither applies**, for the same reason as `utils/binary-search`: gate 9's oracle protocol is built
around `new Ctor(...)` and an instance with observable state, and these are free functions over
caller-owned arrays. The substitute is `round_trips_at_every_power_of_two_size`, which is the
property upstream checks once at one size, checked at six.

Gate 10 will be recorded against the `_utils` unit once that unit exists.

Gate 6 (falsification), performed through the native suite with the target assertion named first:

| sabotage | must break | result |
|---|---|---|
| `jenkinsInt32`'s `(a as i32) >> 19` → `a >> 19` (arithmetic shift becomes logical) | `jenkins_int32_matches_node_24_18_1` | **red**, together with `the_upstream_pairs_land_in_a_known_layout` — which is the point: the layout test is downstream of the hash, so a hash defect moves the data too |
| `linearProbingGet`'s branch order: `c === 0` tested before `c === key` | `the_key_zero_occupies_a_slot_that_still_reads_as_empty` | **red**, and *only* that test — the seven others do not store `0`, so nothing else could see it |

Both reverted; 9/9 green afterwards. The second sabotage is the more informative of the two: it is a
one-line reordering that no upstream assertion can distinguish, and it is caught only because the
port has a test for a key upstream never stores.

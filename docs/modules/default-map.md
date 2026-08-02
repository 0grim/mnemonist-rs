# default-map

Upstream: `default-map.js` (162 LOC) · `test/default-map.js` — **111 lines, 7 `it` blocks,
21 assertion statements**.

Port: `crates/mnemonist-core/src/map/mod.rs` (the `Map` itself),
`crates/mnemonist-core/src/structures/default_map.rs`.
Bridge: `crates/mnemonist-napi/src/js_key.rs`, `crates/mnemonist-napi/src/js_value.rs`,
`crates/mnemonist-napi/src/map_cursor.rs`, `crates/mnemonist-napi/src/default_map.rs`.

This is the **pilot for bridge tier T3** (DESIGN.md §3.3, §3.8). T3 is not a family of related
structures; it is one capability — *reproduce JavaScript's `Map`* — that eleven modules share,
because `default-map`, `bi-map`, `fuzzy-map`, `fuzzy-multi-map`, `multi-map`, `multi-set`,
`lru-map` and `lru-map-with-delete` all keep their state in a `new Map()`. `default-map` was chosen
because it is the *thinnest* wrapper over one: 162 lines, of which four are not delegation. The
hard part is therefore isolated in exactly the place the next ten modules will inherit it from.

---

## What upstream tests

Seven `it` blocks, all on a map built with `FACTORY = function() { return []; }`:

```js
map.get('one').push(1);          // the central idiom of the file
map.set('two', [2]);
assert.deepStrictEqual(map.get('one'), [1]);
assert.strictEqual(map.size, 2);
assert.deepStrictEqual(map.get('unknown'), []);   // reading creates
assert.strictEqual(map.size, 3);
map.clear();
// …delete/has, forEach, entries/keys/values via obliterator's take(), autoIncrement, peek…
```

Characterising the shape of that coverage:

* **Every key is a string literal.** `'one'`, `'two'`, `'unknown'`, `'test'`, `'test2'`. Nothing
  else — no numbers, no objects, no `NaN`, no `-0`, no `null`, no `undefined`.
* **Every stored value is defined.** An array from the factory, a number from `autoIncrement`, or
  a literal. **`undefined` is never stored**, which turns out to be the whole ballgame — see B-40.
* **A key is never overwritten.** `set` is only ever called on a key that does not yet exist.
* **A key is never deleted and re-added.** The one `delete` block deletes `'one'` and stops.
* **No map ever holds more than three entries**, and the largest is built in three calls.
* **Iteration is drained immediately.** `take(map.entries())` creates a cursor and exhausts it in
  one expression, so nothing about the cursor's *state* is observable.
* **`map.get(k).push(v)` appears six times.** This is the one place upstream leans hard on
  something difficult: the value must come back **by reference**, so that a mutation through the
  returned handle is visible on the next read.

## What upstream does NOT test

The point of this document. Everything below is reachable through the public API and never
exercised by the original suite.

**The `size` counter, which is upstream's one real defect**

1. **`undefined` is never stored as a value**, so the entire B-40 chain is unreachable: `get`
   testing the *value* rather than the *key*, the factory re-running on every read of such a key,
   and `size` drifting away from `items.size` without bound.
2. **`size` is never compared against `items.size`.** They are asserted to be equal only in cases
   where they cannot differ.
3. **`size` after an overwrite is never checked**, because no key is ever overwritten.

**`Map` semantics — the capability this unit exists to establish**

4. **Insertion order is only ever checked on an append-only map.** Order after a delete, after a
   re-insert, or after an overwrite is never asserted, so nothing pins the two rules that
   distinguish a `Map` from a `HashMap`: *delete-then-reinsert moves the key to the end, overwrite
   does not*.
5. **Iterator liveness is entirely untested.** All three rules — an entry appended behind a live
   cursor **is** visited, an entry deleted ahead of it is **skipped**, and a cursor that has once
   reported `{done: true}` stays detached even if the map grows — have zero coverage.
6. **`clear()` under a live iterator is never done.** Nor is the sequel: `clear()` then `set()`
   then `next()`, which yields the *new* entry.
7. **A cursor is never re-drained**, so the non-restartability of D-06 is unobserved.
8. **`[...map]` is never used**, so the collection-level `Symbol.iterator` — the *factory* half of
   D-07, the half napi does not provide for free, and the one upstream aliases to `entries` rather
   than to `values` — has **zero** upstream coverage despite being the last line of the module.
9. **No map is ever large enough to compact.** The port's tombstone reclamation, and the cursor
   relocation that has to survive it, are unreachable at three entries.

**Keys — the whole of SameValueZero**

10. **No non-string key is ever used.** `NaN` as a key (which `Map` treats as equal to itself,
    unlike `===`), `-0` and `+0` collapsing to one key, `0` versus `'0'` being two keys, and
    `null`/`undefined`/booleans as keys are all untested.
11. **An object key is never used** — which is also true of every other T3 test file; see
    "Deliberate divergences".

**Values**

12. **`null` as a value is never stored**, so nothing distinguishes it from `undefined`. They are
    very different: one is a value, the other is absence.
13. **The falsy sweep is never done.** `false`, `0`, `''` as values are untested here (though
    `lru-cache` does test them, which is why the port handles them).
14. **A value is never overwritten**, so nothing observes that the displaced value is released.

**The factory**

15. **The factory's arguments are never inspected.** Upstream calls `this.factory(key, this.size)`;
    the test's factories declare no parameters. So neither the key nor — more interestingly — the
    *drifted* size is checked.
16. **A throwing factory is never used**, so nothing pins that a failed `get` leaves the map
    completely untouched, `size` included.
17. **`autoIncrement`'s independence is never checked** — that two calls to
    `DefaultMap.autoIncrement()` produce two separate counters.

**`forEach`**

18. **The third callback argument is never inspected.** Upstream delegates to
    `this.items.forEach(...)`, so the native `Map` passes **itself** — the *inner* map, not the
    `DefaultMap`. The test's callback declares two parameters. This one is a real divergence in the
    port; see below.
19. **`scope` is never passed**, so the `arguments.length > 1 ? scope : this` branch is untested.
20. **`forEach` is never mutated during**, so its liveness is untested for the same reason (5) is.

**Never called at all**

21. `inspect()` and the `nodejs.util.inspect.custom` symbol. ~15 LOC of the module.

## What we test in addition

**`crates/mnemonist-core/src/map/mod.rs` — 21 tests** on the `Map` itself, deliberately against a
plain `OrderedMap<&str, u32>` rather than through `DefaultMap`, so the semantics are pinned once
for all eleven T3 modules:

| Test | Closes gap |
|---|---|
| `iterates_in_insertion_order` | 4 — the baseline |
| `overwriting_a_key_keeps_its_position` | 4 |
| `deleting_then_reinserting_moves_the_key_to_the_end` | 4 |
| `deleting_a_missing_key_reports_it_and_changes_nothing` | — |
| `an_append_behind_a_live_cursor_is_visited` | 5 |
| `a_delete_ahead_of_a_live_cursor_is_skipped` | 5 |
| `a_delete_behind_a_live_cursor_is_not_revisited` | 5 |
| `a_cursor_that_reported_the_end_stays_done_even_if_the_map_grows` | 5 |
| `clear_then_set_is_visible_to_a_cursor_that_has_not_finished` | 6 |
| `clear_then_a_step_detaches_the_cursor_before_the_set` | 6 — the same two operations in the other order, which is the whole distinction |
| `a_cursor_opened_on_a_used_map_starts_at_the_first_live_entry` | 4, 5 |
| `compaction_reclaims_tombstones_without_disturbing_order` | 9 |
| `a_compaction_under_a_live_cursor_does_not_disturb_the_walk` | 9 — see "Bugs this found" |
| `a_compaction_ahead_of_a_live_cursor_skips_the_deleted_entries` | 9 |
| `a_compaction_between_two_maps_of_cursors_is_invisible_to_iteration` | 9 |
| `from_iter_lets_later_duplicates_overwrite_in_place` | 4 |
| `get_and_contains_key_agree_with_the_walk` | — |
| `get_mut_and_values_mut_reach_the_stored_values` | — |
| `slot_of_and_entry_at_round_trip` | — |
| `an_empty_map_yields_nothing_and_reports_done_once` | 5 |
| `debug_shows_iteration_order_not_the_representation` | — |

**`crates/mnemonist-core/src/structures/default_map.rs` — 15 tests:**

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all seven upstream blocks, as a baseline |
| `size_drifts_when_a_stored_value_is_undefined` | 1, 2 — B-40, pinned value by value across three reads |
| `a_write_resynchronises_a_drifted_size` | 1, 2, 3 — the other half: `set` and `delete` repair it, `get` does not |
| `a_refilled_undefined_keeps_its_insertion_position` | 1, 4 |
| `a_defined_value_written_by_the_factory_ends_the_drift` | 1 |
| `has_and_peek_disagree_on_a_stored_undefined` | 1, 12 |
| `delete_distinguishes_a_missing_key_from_a_stored_undefined` | 1, 12 |
| `set_reports_the_defined_value_it_displaced` | 3, 14 |
| `a_deleted_key_is_reinserted_at_the_end` | 4 |
| `a_cursor_sees_entries_the_factory_creates_after_it_was_opened` | 5 |
| `clear_repairs_a_drifted_size_and_empties_the_map` | 1, 6 |
| `the_factory_receives_the_key_and_the_current_size` | 15 |
| `a_failing_factory_leaves_the_map_untouched` | 16 |
| `a_failing_factory_leaves_a_stored_undefined_untouched` | 16 — the harder half, where the key already exists |
| `an_empty_map_reports_nothing` | — |
| `values_mut_reaches_every_stored_slot_including_the_undefined_ones` | 14 |

**`crates/mnemonist-napi/src/js_key.rs` — 8 tests** on SameValueZero, closing gap 10. Each asserts
**both** halves — equal *and* equally hashed — because a key type where `Eq` and `Hash` disagree
grows two entries for one key and every other test still passes:
`nan_is_the_same_key_as_nan`, `every_nan_payload_folds_onto_one_key`,
`negative_zero_is_the_same_key_as_positive_zero`, `negative_zero_is_stored_as_positive_zero`,
`ordinary_numbers_are_distinct_and_integral_forms_coincide`,
`infinities_are_keys_and_are_not_each_other`, `the_primitive_shapes_do_not_collide` (which pins
`0` ≠ `'0'` ≠ `false` ≠ `null` ≠ `undefined` ≠ `''`), `strings_are_compared_by_content`.

**27 side-by-side probes against the real upstream module**, run through the built addon and the
vendored `bench/upstream/default-map.js` in one process, comparing JSON-serialised results.
All 27 agree. They cover, end to end through the bridge, what the Rust tests cover in core:
value identity across a round trip (gap for free), the B-40 drift and its resynchronisation,
delete-then-reinsert order, overwrite position, `NaN` and `-0` as keys, mixed primitive keys,
`null` versus `undefined` as values, all three liveness rules, `clear`-then-`set` under a cursor,
non-restartability next to collection restartability, `forEach` liveness and both `scope` bindings,
`autoIncrement` independence, the factory's two arguments, a throwing factory, the falsy sweep,
spreading the map, a 40-key churn, and a walk across a compaction.

The **differential fuzzer** then covers gaps 1–9 and 12 continuously rather than at hand-picked
points; see "Fuzz".

**Still untested, stated rather than glossed:** gap 21 (`inspect`, not bridged), gap 18 in its
exact form (a deliberate divergence, below), gap 19 in its `arguments.length` form (same), and gap
11 (object keys — a deliberate divergence, below).

## Bugs this found

**B-40 — `DefaultMap.get` tests the *value*, not the *key*, and then increments a counter instead
of reading one. `size` drifts without bound on a map holding one entry.**
`status: verified against Node 24.18.1`.

```js
DefaultMap.prototype.get = function(key) {
  var value = this.items.get(key);
  if (typeof value === 'undefined') {          // (1) tests the VALUE
    value = this.factory(key, this.size);
    this.items.set(key, value);
    this.size++;                               // (2) not `this.items.size`
  }
  return value;
};
```

Line (1) asks whether the stored *value* is `undefined`, which is not the same question as whether
the key is present — they differ for exactly the keys whose value is `undefined`. Line (2) then
increments a counter, where `set` and `delete` both resynchronise from `this.items.size`. Measured:

```text
m.set('a', undefined);   size 1   items.size 1
m.get('a');              size 2   items.size 1   factory called again
m.get('a');              size 3   items.size 1   factory called again
m.delete('a');           size 0   items.size 0   resynchronised
```

Three consequences, all reproduced:

* **The factory re-runs on every read** of a key whose value is `undefined`, which for a stateful
  factory such as `DefaultMap.autoIncrement()` means the counter advances on a *read*.
* **`size` is unbounded** in the number of reads, not in the number of entries, and can be
  arbitrarily larger than `items.size`.
* **The drift is silent and self-healing.** Any `set` or `delete` snaps `size` back, so a program
  that interleaves reads and writes shows a `size` that is sometimes right and sometimes not, with
  nothing to indicate which.

Reproduced rather than corrected. `DefaultMap::size()` is a stored counter and `items().len()` is
the truth. **The correction is what a careful porter would write by accident**: making `size`
return the entry count is tidier, is what the name suggests, and passes the entire upstream suite —
confirmed, see "Falsification" below.

**A defect in the differential fuzzer, found by its own first campaign.**
`status: fixed, this series`. Not an upstream bug; recorded because it would have produced a false
divergence report. JavaScript has one number type and JSON has one number syntax, so `1` serialises
as `1` and never as `1.0`. serde_json *does* distinguish the two and compares them unequal, so the
Rust side's `json!(1.0)` disagreed with the oracle on every integral key. Caught on the very first
run of the grammar, before any real result was recorded. Fixed by `number_json`, which encodes a
double the way `JSON.stringify` does. The seed is committed.

**A defect in our own falsification method, found while writing the core tests.**
`status: fixed, this series`. The first version of
`a_compaction_under_a_live_cursor_does_not_disturb_the_walk` deleted the entries **ahead** of the
cursor. That does force a compaction — but it removes only slots the cursor had not yet reached, so
the cursor's physical index stays *accidentally* correct and the test passed against a `locate` that
was deliberately broken to return its unvalidated hint. Rewritten to delete the entries **behind**
the cursor, where compaction shifts every remaining entry left, and then confirmed red against the
same sabotage. Rewriting it also exposed a real out-of-range index panic: the hint validation read
`map.slots[hint - 1]` before checking `hint <= slots.len()`, so a hint left past the end of a
shrunken vector panicked instead of being rejected. This is the same lesson as DESIGN.md §1.1's
"gate 6 exists because of a real miss", one level down: **a falsification test that cannot fail is
just a second green light**, and that applies to the tests as much as to the gate.

**What the fuzzer found in the port: nothing.** Two campaigns, 4.37 M operations, zero divergences.
As with `sparse-set`, that is the expected outcome — a faithful port reproduces upstream's bugs, so
differential fuzzing structurally cannot find them (D-33). B-40 was found by reading the file line
by line and confirming each step against Node. What the fuzzer is for is the other direction, and
here it is *sharper than the original suite by a wide margin*: see "Fuzz".


### B-31 — `&self` on a `Freeze` type was `noalias readonly` (fixed 2026-08-01)

This bridge held a bare core value, so `&self` compiled to a `noalias readonly` pointer and LLVM was
entitled to hoist reads across the JS callback — which it did. It now holds `RefCell<Core>`, which
is not `Freeze`, and every `&mut self` method became `&self` + `borrow_mut()`. The borrow is taken
per step and released before the callback runs, so a re-entrant callback never meets an outstanding
borrow. See `planning/NOTES.md` B-31 and `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **Object, symbol, function and bigint keys are rejected**, with an error naming the limit. | `Map` compares objects by identity, and there is no identity hash for a JS object reachable from Rust. Two designs are implementable and both cost something real: tagging each object with a hidden id under a private `Symbol` is O(1) but mutates the caller's object (visible to `Object.getOwnPropertySymbols`) and fails on a frozen one; an association list of `napi_ref` probed with `napi_strict_equals` touches nothing but is O(n) per operation and holds a strong reference to every key it has ever seen, making release a leak problem in its own right. **No upstream test in the entire T3 family uses an object key** — audited across `default-map`, `set`, `bi-map`, `fuzzy-map`, `fuzzy-multi-map`, `multi-map`, `multi-set`, `lru-cache` (all four variants) and `sparse-map`; every key that reaches a `Map` is a string or a number, and `fuzzy-map` hashes its object arguments to strings *before* the `Map` sees one. Building machinery no test can reach is worse than a stated limit; answering silently and wrongly is worse than both. |
| — | **Primitive values are stored by value; only objects are stored by reference.** | Forced twice over. `napi_create_reference` **rejects a number** at `NAPI_VERSION` 9, which this addon declares — measured, not assumed: it is what made two of the seven upstream assertions fail on this bridge's first run. And it is right independently: a `napi_ref` is a V8 global handle, and one per stored value would mean a million global handles for a million-entry `lru-cache` against upstream's inline SMIs. Nothing is observable: a JS primitive has no identity, so `0 === 0` and `'a' === 'a'` regardless of provenance. `-0` and `NaN` survive verbatim, because only *keys* are normalised. |
| — | **`forEach`'s third callback argument is the `DefaultMap`, not the inner `Map`.** | Upstream delegates to `this.items.forEach(...)`, and a native `Map` passes *itself* to the callback — so upstream's third argument is the internal map, an object this port does not have. The `DefaultMap` is passed instead. Untested upstream (gap 18); the first two arguments, which the original test does use, are exact. |
| — | **`forEach(cb, undefined)` binds `this` to the map.** | Upstream keys off `arguments.length > 1`, which napi's typed signature cannot see: "omitted" and "passed as `undefined`" are the same value. Identical to `SparseSet::for_each` and recorded the same way. The omitted-argument case — the only one the original suite uses — is exact, and passing a real scope object is exact. |
| — | **`inspect()` is not ported.** | It returns the inner `Map`, which does not exist in this port, and nothing asserts on it. |
| — | ~~**A re-entrant factory or `forEach` callback is not supported.**~~ **WITHDRAWN 2026-08-01 — both are now supported.** | This row said fixing it "means interior mutability throughout and that is a decision for the whole bridge, not for one module". That decision was then forced by B-31, which turned out to be the same exposure miscompiling rather than merely aliasing. The bridge now holds `RefCell<Core>`, `forEach` re-borrows per step, and `get` runs the factory between its read and its write exactly as upstream does — so a callback or factory that calls back into the same map behaves as upstream's. Verified differentially in `tests/boundary/reentrancy.js`. |
| — | **The key is stored twice.** | `OrderedMap` keeps a `HashMap<K, usize>` index alongside the entry vector. `indexmap` avoids the second copy with `hashbrown`'s raw-entry API; the core crate is zero-dependency by declaration and `std`'s `HashMap` exposes no equivalent on stable. Mitigated rather than hidden: the bridge's string keys are `Rc<str>`, so the second copy is a refcount, not the text. |
| — | **`undefined` is spelled `None`.** | Core has no JavaScript values, so `DefaultMap<K, V>` stores `Option<V>` and `None` *is* `undefined`. This is what makes B-40 expressible and testable from pure Rust, and it gets `peek` right for free — upstream's `peek` cannot distinguish a missing key from a key holding `undefined` either. |
| — | **The factory is not stored in core.** | Upstream keeps it on the instance; here it is a per-call argument to `get_or_insert_with`, and the bridge holds the `FunctionRef`. The constructor's `typeof factory !== 'function'` check is a JavaScript type test and belongs at the boundary — its message is kept verbatim. A stored `F` would also put a JS callback inside a crate that must not know JavaScript exists. |
| D-06 | **No collection implements `IntoIterator`.** | Unchanged from `sparse-set`: it would hand out a fresh iterator per `for` loop and silently restart. |
| D-07 | **`Symbol.iterator` is installed from Rust, not from the shim.** | Unchanged — but note the table row is `("DefaultMap", "entries")`, not `values`. Upstream's last line aliases `entries`, so spreading a `DefaultMap` yields `[key, value]` pairs. A table that assumed `values` for every module would have been wrong on the second module it met. |
| — | **`MapCursor` is not `crate::cursor::Sequence`.** | The `obliterator` cursor freezes a length at construction and reads elements lazily; a `Map` cursor owns its entry list, skips tombstones, and sees appends. Both are faithful — to different things. One abstraction over both would get one of them wrong, so there are two. |

## Fuzz + bench

### Fuzz

```
module=default-map seed=42       cases=29525 ops=2939502 wall=120.0s divergences=0
module=default-map seed=20260801 cases=14492 ops=1432987 wall=60.0s  divergences=0
```

Two campaigns, two seeds, **4.37 M operations, zero divergences**. Reproduce with
`target/release/difffuzz --module default-map --seed 42 --cases 29525`.

* **Op alphabet:** `get(k)` (weight 5) · `set(k, v)` (4) · `delete(k)` (3) · `peek(k)` (2) ·
  `has(k)` (2) · `clear()` (1) · `$iter("entries"|"keys"|"values")` (2) · `$next()` (4) ·
  `$spread()` (1).
* **Observable state, compared after every op:** `size` **and** `items`, separately. Both are
  public upstream, and separating them is the point — they disagree by design once B-40 fires, so a
  port that made `size` return the entry count agrees on `items` and diverges on `size` within two
  operations. `items` is encoded as a **list** of pairs, so entry order is compared, not just
  membership.
* **Keys:** a pool of eight — `'a'`, `'b'`, `'0'`, `0`, `1`, `-1`, `NaN`, `-0`. Small on purpose:
  collisions, overwrites and delete-then-reinsert have to be constant rather than lucky, and a wide
  key space would spend every program inserting fresh keys. `0` and `'0'` are in it because they
  are two different `Map` keys and a port that stringified would agree on everything else; `NaN`
  and `-0` are the only two places SameValueZero differs from `===`.
* **Values:** `undefined` (weight 2), `null`, small integers, `'v'`. `undefined` is weighted in
  rather than rare because it is the only route to B-40 — and once it fires, every subsequent
  operation in that program is compared against a *drifted* upstream.
* **Constructors:** five named factories — `undefined`, `null`, `autoIncrement`, `key`, `size` —
  built fresh per instance. `autoIncrement` is upstream's own and is the only stateful one; `key`
  and `size` are what make the factory's two arguments observable.
* **Program length:** 1..200 ops. Over eight keys that is enough deletion to force several
  compactions, which is the only way the cursor's id-based relocation is exercised at all.
* **Deliberately excluded:** `forEach`, because the oracle protocol cannot transmit a callback —
  its walk is the same cursor the iterators use, and its callback arguments and `scope` binding are
  covered by the original suite and by the probes. And **`JsKey` itself**: the real key type lives
  in `mnemonist-napi`, a `cdylib` that cannot be linked into a plain Rust binary, so the fuzzer's
  `FuzzKey` *mirrors* its normalisation rather than reusing it. What the fuzzer therefore verifies
  is that the normalisation **rule** is right against a real `Map`; that the bridge **applies** that
  rule is verified by the eight `js_key` tests and by the 27 probes.

**Three additive changes to `fuzz/oracle.js`**, all of which the remaining ten T3 modules need:
`encode` now handles `Map` (a `Map` has no own enumerable properties, so the generic object branch
was encoding a T3 module's whole state as `{}` — an observation that could never disagree with
anything); arguments and constructor arguments are now `decode`d, because JSON has no `undefined`,
no `-0`, no `NaN` and no functions and all four are ordinary inputs here; and factories are named
rather than transmitted as source, so a program stays reproducible from its seed and a repro stays
readable.

**The fuzzer was falsified twice, and this is the headline result for this unit:** *both sabotages
leave the original test file completely green.*

**A — the `size` half.** Sabotage: `get_or_insert_with` resynchronising `size` from `items` instead
of incrementing it — the tidier reading, the one that deletes B-40, and the single most plausible
mis-port of this module.
Original mocha suite: **7 passing, 0 failing.**
Fuzzer: caught in **136 cases (0.1 s)**, shrunk from 200 ops to **two**:

```js
var s = new DefaultMap(function () { return undefined; });
s.get(0);   // port size 1, upstream size 1
s.get(0);   // port size 1, upstream size 2
```

**B — the `Map` half.** Sabotage: `OrderedMap::set` re-inserting an existing key instead of
overwriting it in place, so an overwrite moves the key to the end — the "just delete and re-add"
reading.
Original mocha suite: **7 passing, 0 failing.** (Three of the native tests do catch it.)
Fuzzer: caught in **151 cases (0.1 s)**, shrunk to **three**:

```js
var s = new DefaultMap(function () { return undefined; });
s.get(0);
s.set('a', undefined);
s.get(0);      // port items [a, 0], upstream items [0, a]
```

Both were reverted and both are caught by the single seed committed in
`crates/difffuzz/proptest-regressions/default-map.txt`, which proptest replays before any novel
case on every subsequent run. That the corpus holds one seed rather than three is not an omission:
proptest declines to persist a seed it already holds, and that one seed — itself provenance from
the number-encoding fault above — reaches both sabotages. Verified by re-running sabotage A against
an emptied corpus and watching the same hash come back.

### Bench

`bench/results.json` → `modules["default-map"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`get-or-insert`/`delete` (50/25/25) over the full 1e6-key domain
(`IK = K = V = u32`; the factory always returns `Some`, so B-40's `size` drift never fires in this
workload), xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 62.3 | **44.0** | 1.42× slower |
| p99 ns/op | 176.0 | **153.2** | 1.15× slower |
| RSS delta MB | **67.0** | 220.6 | |
| structure-only RSS delta MB | **1.4** | 9.7 | |
| startup ms | **0.6** | 16.7 | 28× (reported separately; not throughput) |

**This is a loss, on both p50 and p99, stated plainly rather than smoothed into a clean sweep** —
§5.1 says to expect one and report it. Re-checked at 4x domain (`mixed-4e6`, 4e6 keys, same 1e6 ops)
before trusting a single data point:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 55.1 | **47.1** | 1.17× slower |
| p99 ns/op | 156.6 | **123.8** | 1.27× slower |

The loss holds at both sizes — a real, reproducible cost, not a one-run blip (run-to-run p50 noise
on this host is up to ~32% per `methodology.md`; the two `mixed-1e6` measurements taken during this
batch — 1.33× and 1.42× — sit inside that band, but the *direction* of the loss never flipped across
either rerun or the 4x domain probe).

**The cause is not confirmed.** This doc's own prior note speculated about two candidate mechanisms
before any number existed; one of them does not apply to what was actually measured here and the
other is a plausible but unverified explanation, not a confirmed one:

* **`Rc<str>` keys versus V8 strings — does not apply to this benchmark.** That concern is about the
  bridge's string-keyed instantiation; this benchmark links `mnemonist-core` directly with bare
  `u32` keys (per `methodology.md`, never through N-API), so no `Rc<str>` allocation exists on this
  path at all. Ruled out by construction, not by measurement.
* **`get_or_insert_with` on a hit does two hash lookups, not one** — `slot_of` then `entry_at`
  (`DefaultMap::try_get_or_insert_with`), because the borrow from the first has to end before a
  factory that might re-enter can run. This is a plausible explanation for a chunk of the p50 gap —
  `OrderedMap` doing a lookup twice on the 25% of ops that are `get_or_insert_with` is a real,
  structural cost — but it has not been isolated against a metric that would confirm or falsify it
  (e.g. a probe comparing this path to a hypothetical single-lookup one), so it is labelled here as
  **unconfirmed**, not as the cause.

### `$forEach` — the op that was missing (added 2026-08-01, B-31)

`default-map`'s grammar had no `forEach` op at all. That omission is what let B-31 — a `forEach`
callback mutating the collection it is walking — through 4.37 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it.

`$forEach(method, rule, limit)` now walks the instance with a callback that calls back into it.
The compared result is the sequence of callback argument pairs, so the walk's **shape** is checked
and not only the state it leaves behind. This module's mutations:

* `delete(a1)`, `set(a1, a0)` and `clear()`, all uncapped.

This module's walk is the one that differs from every other in the port: a `Map` iteration is
**live in both directions**, so an entry the callback adds *is* visited and one it deletes ahead of
the cursor is *not*. `set` writes back the pair it was handed, so it overwrites rather than adds —
which matters here and nowhere else, because a growing walk here would never terminate. The op is
driven by the same cursor `$next` uses, so a second hand-written walk cannot drift from it.

**What it does not reach, stated so the campaign is not over-read.** `difffuzz` compares
`mnemonist-core` against upstream JS; the napi bridge, where B-31's hoisted read actually lived, is
not in that loop. No op alphabet can catch that class of bug here. The specs that do are
`tests/boundary/reentrancy.js`, which drive the real addon with real JS callbacks — red on the
pre-fix bridges, green after.

One deliberate narrowing, mirrored on both sides: a selected callback argument that is `undefined`
skips the mutation. Feeding it back in reaches upstream's `NaN`-indexed swap, which `usize` cannot
express and the core does not model. Fully disclosed in `fuzz/log.txt`.

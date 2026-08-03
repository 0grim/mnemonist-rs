# default-map — evidence

Gate artifacts for `docs/modules/default-map.md`: test-to-gap tables, probe list, fuzz grammar,
full falsification record, full benchmark tables.

## Test-to-gap mapping

**`crates/mnemonist-core/src/map/mod.rs` — 21 tests** on the `Map` itself, against a plain
`OrderedMap<&str, u32>` rather than through `DefaultMap`:

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
| `a_compaction_under_a_live_cursor_does_not_disturb_the_walk` | 9 — see the falsification-method note in the log |
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

**`crates/mnemonist-napi/src/js_key.rs` — 8 tests** on SameValueZero, closing gap 10:
`nan_is_the_same_key_as_nan`, `every_nan_payload_folds_onto_one_key`,
`negative_zero_is_the_same_key_as_positive_zero`, `negative_zero_is_stored_as_positive_zero`,
`ordinary_numbers_are_distinct_and_integral_forms_coincide`,
`infinities_are_keys_and_are_not_each_other`, `the_primitive_shapes_do_not_collide` (which pins
`0` ≠ `'0'` ≠ `false` ≠ `null` ≠ `undefined` ≠ `''`), `strings_are_compared_by_content`.

## 27 differential probes

Run through the built addon and the vendored `bench/upstream/default-map.js` in one process,
comparing JSON-serialised results. All 27 agree. Coverage: value identity across a round trip, the
B-40 drift and its resynchronisation, delete-then-reinsert order, overwrite position, `NaN` and
`-0` as keys, mixed primitive keys, `null` versus `undefined` as values, all three liveness rules,
`clear`-then-`set` under a cursor, non-restartability next to collection restartability, `forEach`
liveness and both `scope` bindings, `autoIncrement` independence, the factory's two arguments, a
throwing factory, the falsy sweep, spreading the map, a 40-key churn, and a walk across a
compaction.

## Fuzz grammar

* **Op alphabet:** `get(k)` (weight 5) · `set(k, v)` (4) · `delete(k)` (3) · `peek(k)` (2) ·
  `has(k)` (2) · `clear()` (1) · `$iter("entries"|"keys"|"values")` (2) · `$next()` (4) ·
  `$spread()` (1) · `$forEach(method, rule, limit)`.
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
* **`$forEach` mutations:** `delete(a1)`, `set(a1, a0)` and `clear()`, all uncapped. The compared
  result is the sequence of callback argument pairs, so the walk's shape is checked and not only
  the state it leaves behind.
* **Deliberately excluded:** `forEach` as a plain op, because the oracle protocol cannot transmit a
  callback — its walk is the same cursor the iterators use, and its callback arguments and `scope`
  binding are covered by the original suite and by the probes. And **`JsKey` itself**: the real key
  type lives in `mnemonist-napi`, a `cdylib` that cannot be linked into a plain Rust binary, so the
  fuzzer's `FuzzKey` *mirrors* its normalisation rather than reusing it. What the fuzzer therefore
  verifies is that the normalisation **rule** is right against a real `Map`; that the bridge
  **applies** that rule is verified by the eight `js_key` tests and by the 27 probes.

Three additive changes to `fuzz/oracle.js`, all of which the remaining ten T3 modules need:
`encode` now handles `Map` (a `Map` has no own enumerable properties, so the generic object branch
was encoding a T3 module's whole state as `{}` — an observation that could never disagree with
anything); arguments and constructor arguments are now `decode`d, because JSON has no `undefined`,
no `-0`, no `NaN` and no functions and all four are ordinary inputs here; and factories are named
rather than transmitted as source, so a program stays reproducible from its seed and a repro stays
readable.

## Falsification record

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
the number-encoding fault (see log) — reaches both sabotages. Verified by re-running sabotage A
against an emptied corpus and watching the same hash come back.

## Bench tables

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

Re-checked at 4x domain (`mixed-4e6`, 4e6 keys, same 1e6 ops):

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 55.1 | **47.1** | 1.17× slower |
| p99 ns/op | 156.6 | **123.8** | 1.27× slower |

Run-to-run p50 noise on this host is up to ~32% per `methodology.md`; the two `mixed-1e6`
measurements taken during this batch — 1.33× and 1.42× — sit inside that band, but the direction of
the loss never flipped across either rerun or the 4x domain probe.

**`peek` vs `get_or_insert_with` hit-path probe** (`bench-runner --default-map-probe`), over the
same prefilled 1,000,000-key map, same keys, no factory ever invoked:

| variant | p50 ns/call |
|---|---|
| `peek` (one hash lookup, `OrderedMap::get`) | 118.224 |
| `get_or_insert_with` hit path (`slot_of` + `entry_at`) | 117.432 |

Full cause investigation and the structural-fix cost estimate: the log.

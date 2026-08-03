# sparse-map — evidence

Gate artifacts for `docs/modules/sparse-map.md`: test-to-gap table, fuzz grammar, full
falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/sparse_map.rs` — 20 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all nine upstream blocks, both constructor signatures, as a baseline |
| `delete_moves_the_key_but_not_the_value` | **1, 2, 3, 9, 10** — the headline defect, pinned on `get`, on iteration order *and* on `vals` slot by slot |
| `the_stale_value_is_not_an_artefact_of_the_array_store` | 1 — the same defect through a typed store, so it is attributed to the swap and not to the store |
| `deleting_the_last_entry_hides_the_defect` | 11 — and it is named for what it demonstrates: the branch upstream tests is the branch where the bug is invisible |
| `setting_a_present_member_overwrites_in_place` | 5 — asserts both index arrays are unchanged, not just `size` |
| `clear_leaves_stale_entries_that_stay_unreachable` | 12 — asserts the debris is *there*, in `dense` and in `vals`, and unreachable |
| `reads_out_of_range_report_absence` | 16 (the `has`/`get`/`delete` half) |
| `an_out_of_range_set_corrupts_the_map_exactly_as_upstream_does` | 15, 16 — the compound defect, pinned value by value, including the value landing where the key did not |
| `an_array_value_store_outgrows_the_map_it_belongs_to` | **8, 20, 22** — `keys()` gapping while `values()` still yields real data, on the same map, at the same ordinals |
| `a_typed_value_store_drops_the_write_and_gaps_with_the_keys` | 8, 20 — the contrasting store, where both sides gap together |
| `typed_values_truncate_at_their_own_width` | 6, 7 — all three widths, and the independence of the value width from the index width |
| `a_delete_past_capacity_writes_dense_but_not_sparse` | 16 — B-10 on this module's arrays |
| `cursors_do_not_restart_but_the_map_can_be_walked_again` | 18, 19 — both levels of D-07, and all three projections |
| `a_delete_during_iteration_is_visible_and_desynchronises_the_pair` | **17** — B-11 in one assertion: the walk yields `(3, 20)`, a key and a value that were never set together |
| `a_set_during_iteration_is_not_visible_to_the_cursor` | 17 — the frozen-length half |
| `picks_one_pointer_width_for_both_index_arrays` | 14 — five lengths across both width boundaries |
| `rejects_a_length_no_pointer_array_can_index` | 14 (the throw), for both constructors |
| `a_zero_length_map_finds_nothing_but_still_accumulates_values` | 8, 16, 21, 22 — the degenerate end, where the two stores diverge most |
| `fills_to_capacity_without_running_off_the_end` | 13 |
| `projections_do_not_answer_for_each_other` | the projection accessors, so a mis-projected step is a `None` rather than a plausible-looking value |

## Fuzz grammar

* **Op alphabet:** `set(m, v)` (weight 5) · `delete(m)` (2) · `has(m)` (2) · `get(m)` (2) ·
  `clear()` (1) · `$iter("keys")` / `$iter("values")` / `$iter("entries")` (1 each) · `$next()` (3)
  · `$spread()` (1).
* **Observable state, compared after every op:** `size`, `length`, `dense`, `sparse` **and
  `vals`**. `vals` is what makes B-11 checkable rather than inferable: a port that "tidied up" the
  missing value move would still agree on `size`, on both index arrays and on every `has`.
* **Constructors:** both upstream signatures, all four supported value stores. A JS constructor
  cannot travel over JSON, so it goes as `{"$global": "Uint8Array"}` and `fuzz/oracle.js` resolves
  it against the real global — deliberately for `init`'s constructor arguments only, never for an
  op's.
* **Lengths:** `0..=400`, as `sparse-set` and for the same two reasons.
* **Members:** `0..length + 64`, so roughly one in eight is out of range.
* **Values:** `0..=1000`, so an 8-bit value store truncates on a good fraction of its writes.
* **Program length:** 1..200 ops.
* **`$forEach` mutations:** `delete(a1)`, `set(a1, a0)` and `clear()`, all uncapped.
* **Deliberately narrowed: the values.** Integers, not every JS number. The bridge takes `f64` and
  this spec takes `u32`, because the oracle compares JSON and a float that renders `13.0` on one
  side and `13` on the other is a false divergence rather than a finding. Nothing in the op
  alphabet is excluded.

## Falsification record

### Fuzzer falsification

**A — the value half.** Sabotage: `delete` "fixed" to move the value along with the key. Not a
typo — the change a reader makes on purpose, having decided the omission was an oversight. Caught
in **1,474 cases (3.0 s)**, shrunk from 200 ops to three:

```js
var s = new SparseMap(5);
s.set(0, 0);
s.set(1, 1);
s.delete(0);   // port vals [1,1,…], upstream [0,1,…]
```

This is the most instructive seed in the repo, because **the same sabotage leaves the upstream
mocha suite at 9 passing, 0 failing.**

**B — the store half.** Sabotage: the `Array` value store made unable to grow, i.e. given the typed
store's dropped-write behaviour. That is the natural Rust assumption — a `Vec` allocated at
`length` has a length. Caught in **1,147 cases (4.6 s)**, shrunk to a **single** operation:

```js
var s = new SparseMap(0);
s.set(0, 0);   // port vals [], upstream [0]
```

One call past the constructor.

Both sabotages were reverted and both seeds are committed with provenance in
`crates/difffuzz/proptest-regressions/sparse-map.txt`, where proptest replays them before any novel
case on every subsequent run.

### Falsification of the port (gate 6)

Separate from the fuzzer falsifications: gate 6 asks that sabotaging the core turns the *original
mocha suite* red, proving it exercises Rust rather than a JS fallback.

**The assertion the sabotage had to break was named first:**
`should be possible to create an iterator over the map's values` —
`assert.deepStrictEqual(obliterator.take(map.values()), [13, 22, 8])`, at `test/sparse-map.js:127`.
Chosen because it reaches the projection machinery, which is the code this unit adds.

**The sabotage:** the `Values` projection reading `dense` instead of `vals` — the copy-paste error
that three near-identical upstream closures invite, and the one a port writes by adapting `keys()`
into `values()` and forgetting one identifier.

**Confirmed red**, and red in precisely the named place: `8 passing, 1 failing`, the failure being
that assertion, with `actual` `[3, 6, 9]` against `expected` `[13, 22, 8]`. Reverted; **confirmed
green again**: `9 passing`.

**And a second falsification that was expected to stay green, and did.** Gate 6's own lesson is
that a check which cannot fail is a second green light — so it is worth knowing *which* sabotages
this suite cannot catch, not only that some can. Fixing B-11 in the core leaves the suite at
**9 passing, 0 failing** while turning **four** native tests red
(`delete_moves_the_key_but_not_the_value`, `the_stale_value_is_not_an_artefact_of_the_array_store`,
`a_delete_past_capacity_writes_dense_but_not_sparse`,
`a_delete_during_iteration_is_visible_and_desynchronises_the_pair`) and being caught by the
differential fuzzer in 3.0 seconds. Both numbers were measured, not reasoned about.

## Bench table

`bench/results.json` → `modules["sparse-map"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`get`/`delete` (50/25/25) over length 1e6, xorshift32 seed 42,
`set` taking the workload's second operand as the value:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **12.0** | 16.3 | 1.4× faster |
| p99 ns/op | **28.8** | 52.1 | 1.8× faster |
| min ns/op | **6.1** | 8.3 | 1.4× faster |
| RSS delta MB | **13.8** | 79.7 | |
| structure-only RSS delta MB | **1.3** | 9.9 | |
| startup ms | **0.6** | 17.4 | 29× (reported separately; not throughput) |

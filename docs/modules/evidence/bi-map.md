# bi-map — evidence

Gate artifacts for `docs/modules/bi-map.md`: test-to-gap table, fuzz grammar, falsification record,
full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/bi_map.rs` — 12 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all twelve upstream blocks, as a baseline |
| `clear_desyncs_size_from_inverse_size_bug_bi_map_1` | 1 — BUG-BI-MAP-1, both directions, plus the healing-on-next-mutation property |
| `clear_called_on_the_inverse_view_also_empties_the_forward_map` | 1 — the underlying-maps half (both empty regardless of direction) |
| `set_can_rebind_both_sides_of_the_bijection_in_one_call` | 2 |
| `delete_on_a_missing_key_reports_it_and_changes_nothing` | 3 |
| `a_deleted_key_reinserted_moves_to_the_end` | 4 |
| `the_inverse_view_supports_the_full_method_set` | 5 — `set_reverse`/`delete_reverse` exercised directly |
| `rebinding_a_key_releases_its_old_value_from_the_inverse` | — the two-branch case in isolation |
| `rebinding_a_value_releases_its_old_key_from_the_forward_map` | — the other two-branch case |
| `set_is_a_no_op_when_the_exact_relation_already_exists` | — insertion order is untouched on a no-op |
| `an_empty_map_reports_nothing` | 6 |

## Fuzz grammar

* **Op alphabet:** `set(k, v)` (weight 5) · `delete(k)` (3) · `get(k)` (3) · `has(k)` (2) ·
  `clear()` (1).
* **Keys and values share one six-item pool**, mixed strings and numbers, so `set` collides with an
  existing key, an existing value, or both far more often than a wide space would by chance — that
  collision handling is the entire point of the module. `clear`/`delete` are weighted in because
  BUG-BI-MAP-1's reinsert-after-delete-and-clear interactions are easiest to reach right after one.
* **Observable state:** `size`, `items` (the real `Map`), and `inverse` — `{size, items: {$map:
  [...]}, inverse: {$self: true}}`, because `instance.inverse.inverse === instance` and the oracle's
  generic `encode()` special-cases exactly that circular reference. No oracle change was needed.
* **Deliberately excluded:** `instance.inverse.*` called directly (the oracle's `op` dispatch cannot
  reach a nested `instance.inverse.set(...)`, though every forward op still mutates and every
  observation still reads both sides, so the bijection invariant is fully checked); cursor lifecycle
  ops (`bi-map`'s cursor is `default-map`'s `OrderedMap` cursor, already fuzzed there); `forEach`
  (not yet in this alphabet — see `fuzz/log.txt`).

## Falsification record (gate 6)

**Named first:** `clear_desyncs_size_from_inverse_size_bug_bi_map_1`'s assertion
`assert_eq!(forward.inverse_size(), 1, "clear() must NOT resync inverse_size — BUG-BI-MAP-1")`.

**The sabotage:** `BiMap::clear` given back the line round 1 (pre-fix) was missing —
`self.inverse_size = 0;` alongside `self.size = 0;` — reintroducing the exact "more correct than
upstream" defect fuzzing first caught.

**Confirmed red**, at exactly the named assertion: `left: 0, right: 1`. Reverted; **confirmed green
again**: all 11 `bi_map` unit tests pass, `cargo test --workspace` clean.

## Bench table

`bench/results.json` → `modules["bi-map"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`get`/`delete` (50/25/25) over a shared 1e6-value domain for both
key and value (`K = u32`), xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 118.1 (range across 6 runs: 1.14×–1.59× slower; see log) | **102.9** | 1.15× slower (first measurement) |
| p99 ns/op | 322.3 | **288.2** | 1.12× slower |
| RSS delta MB | **60.1** | 212.8 | |
| structure-only RSS delta MB | **1.4** | 9.8 | |
| startup ms | **0.6** | 16.5 | 25× (reported separately; not throughput) |

Doubled-hashing fix, before/after (six alternating runs, isolation spot-check): 169.9 ns → 164.6 ns
(3% gap inside a 10% run-to-run spread — no speedup claimed). Full re-measurement history and the
1.51× figure from the 2026-08-03 pass: the log.

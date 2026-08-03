# bloom-filter — evidence

Gate artifacts for `docs/modules/bloom-filter.md`: test-to-gap table, fuzz campaign table, full
falsification record, full benchmark table.

## Test-to-gap mapping

| gap | test |
|---|---|
| — (upstream's own, transcribed) | `matches_the_upstream_suites_own_settings`, `..._own_bit_arrays`, `..._own_fifty_item_case`, `..._own_membership_case`, `..._own_validation` |
| 1 | `murmurhash3_matches_node_24_18_1` — 23 (seed, data) pairs against real Node, including the exact negative seeds `hashArray` derives; `every_tail_length_is_reached`; `sum32_is_not_an_adder`; `sum32_with_the_swapped_constant_is_the_addition_murmur_wants` |
| 2 | `per_item_bits_match_node_24_18_1` — a **fresh** capacity-10 filter per item, including `U+0000`, `日本` and `😀` (two code units); `elements_above_a_byte_overlap_and_collide` |
| 3 | `b98_the_empty_sequence_is_an_ordinary_item` (core side) + the bridge's `to_units` |
| 4 | `clear_resets_the_bits_and_keeps_the_sizing` |
| 6 | `the_error_rate_check_reads_the_option_and_the_default_reads_the_value` |
| 7 | `b99_an_error_rate_above_one_is_a_range_error_only_sometimes` |
| 8 | `b97_a_filter_with_no_hash_functions_says_yes_to_everything` |
| 9, 10 | `settings_match_node_24_18_1` — 15 (capacity, errorRate) pairs; `an_infinite_capacity_produces_an_empty_filter` |
| 11 | `never_reports_a_false_negative` — 200 items, all found |
| 12 | `the_false_positive_rate_is_roughly_what_was_asked_for` — 500 in, 2,000 out, asserted under 5% against a nominal 0.5% |

## Fuzz campaign table

| module key | seed | cases | ops | wall |
|---|---|---|---|---|
| `bloom-filter` | 42 | 6,788 | 675,049 | 60.0s |
| `bloom-filter` | 20260801 | 7,087 | 710,548 | 60.0s |

## Falsification record

**Gate 6 — falsification, two runs plus one control, each named before it was performed.**

| what | sabotage | must break | result |
|---|---|---|---|
| the original test suite (gate 4) | `hash = sum32(hash, N)` → `hash = hash + N` in `murmurhash3` | `assert.deepStrictEqual(Array.from(filter.data), [128, 0, 86, 65])` in `'should be possible to add items to the filter.'` | **red**: 4 passing, 2 failing, that assertion among them. Reverted: 6 passing. |
| the fuzz spec | an early `if hash_functions == 0 { return false }` in `test`, i.e. "fixing" BUG-BLOOM-FILTER-2 | a return-value divergence on a filter whose `hashFunctions` truncated to zero | **red**, minimised to two lines: `new BloomFilter({capacity: 21, errorRate: 0.98}).test(...)`, divergence on op #0. Reverted: clean. |
| **control**, to check the BUG-BLOOM-FILTER-1 cancellation rather than assert it | `hash = sum32(hash, N)` → `hash = hash + 0xe6546b64`, the **unswapped** constant | **nothing** — if the cancellation is real, all six stay green | **green**, 6 passing. The cancellation is real. |

The first sabotage is worth one more sentence. `'should be possible to test items'` stayed **green**
under it — because it only checks self-consistency (`add x`, then `test x` is true and `test y` is
false), which a completely different hash satisfies just as well.

## Bench table

`bench/results.json` → `modules["bloom-filter"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 2,000 samples/side.

**`mixed-2e5`** — 200,000 mixed `add`/`test`-hit/`test`-miss (50/25/25), hex-encoded keys, capacity
200,000 at upstream's default 0.5% error rate, seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **97.36** | 163.93 | 1.7× faster |
| p99 ns/op | **162.72** | 235.92 | 1.4× faster |
| RSS delta MB | **0.1** | 14.4 | |
| structure-only RSS delta MB | **0.1** | 6.6 | |
| startup ms | **0.6** | 15.3 | 26× (reported separately; not throughput) |

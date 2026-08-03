# suffix-array — evidence

Gate artifacts for `docs/modules/suffix-array.md`: test-to-gap table, fuzz campaign table, full
falsification record, full benchmark table.

## Test-to-gap mapping

18 native tests in `crates/mnemonist-core/src/structures/suffix_array.rs`.

| gap | test |
|---|---|
| — (upstream's own five, transcribed) | `matches_the_upstream_suites_own_arrays`, `..._own_arbitrary_sequence`, `..._own_generalized_arrays`, `..._own_lcs` |
| 1, 2, 3 | `b90_symbols_sharing_a_low_byte_are_mis_sorted`, `b91_lengths_congruent_to_one_mod_three_are_mis_sorted` — both assert upstream's **wrong** answer *and* assert it differs from a naive reference, so a later "tidy-up" fails loudly |
| 1, 3 | `ascii_inputs_off_the_bad_residue_are_exactly_right` — every binary string of length 1..=14 whose length is not `1 (mod 3)`, checked against a naive `O(n² log n)` suffix sort. 23,404 inputs. |
| 3 | `the_bad_residue_is_correct_until_the_recursion_fires` |
| 4 | `empty_sequences`, `the_shortest_sequences` |
| 5 | `to_string_joins_the_array_with_commas` |
| 7 | `a_generalized_array_of_one`, `a_generalized_array_of_three` |
| 8 | `disjoint_members_have_no_common_substring` |
| 9 | `a_generalized_array_of_one` (asserts `firstLength`), `the_separator_occupies_a_position` (asserts `text`) |
| 10 | `the_token_alphabet_is_ordered_as_strings` |
| 11 | `the_separator_occupies_a_position` |
| the port's own edges | `a_generalized_array_of_none_is_refused`, `a_mixed_generalized_array_is_refused` |

## B-90 radix-width measurement table

For a 15-character input, three passes run:

| offset | read past the end? | `j` | `bits` |
|---|---|---|---|
| 2 | yes (index 16, length 15) | `NaN` | 8 |
| 1 | yes | `NaN` | 8 |
| 0 | no | 513 | 16 |

So two of the three passes are 8-bit while the largest symbol needs 10.

## Fuzz campaign table

**Gate 9 — four campaigns, 1,735,060 operations, zero divergences.**

| module key | seed | cases | ops | wall |
|---|---|---|---|---|
| `suffix-array` | 42 | 274,418 | 548,743 | 60.0s |
| `suffix-array` | 20260801 | 257,468 | 515,381 | 60.0s |
| `generalized-suffix-array` | 42 | 169,107 | 343,616 | 60.0s |
| `generalized-suffix-array` | 20260801 | 161,214 | 327,320 | 60.0s |

## Falsification record

**Gate 6 — falsification, three separate runs, each with its target named before it was performed.**

| what | sabotage | must break | result |
|---|---|---|---|
| the original test suite (gate 4) | remove the `.rev()` from the radix gather in `sort()`, making the LSD sort unstable | `assert.deepStrictEqual(sa.array, [5, 3, 1, 0, 4, 2])` in `'SuffixArray should produce the correct array.'` | **red**: 0 passing, 5 failing. Reverted: 5 passing, 1 pending. |
| the `suffix-array` fuzz spec | `sort()`'s `bits` fall-through `8 → 16`, i.e. "fixing" B-90 | a state divergence in `array` / `toJSON` / `toString` | **red** after 400 cases, minimised to a 42-character input mixing U+0141 with U+0100. Reverted: clean. |
| the `generalized-suffix-array` fuzz spec | LCS's second guard `>` → `>=` | a divergence in `longestCommonSubsequence`'s return value | **red**, minimised all the way to a two-member list whose first member is empty — so `firstLength` is 0 and position 0 *is* the boundary the asymmetric guards let through. Reverted: clean. |

Both minimised seeds are committed under `crates/difffuzz/proptest-regressions/` with PROVENANCE
blocks, because an unlabelled `cc` line reads as "a real port defect was found here", which is the
opposite of what happened.

## Bench table

`bench/results.json` → `modules["suffix-array"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, 500 samples/side.

**`build-2e4x50`** — DC3 construction over a freshly-generated 20,000-character random text, 50
passes, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/char | **69.1** | 514.1 | 7.4× faster |
| p99 ns/char | **91.4** | 632.7 | 6.9× faster |
| min ns/char | **66.7** | 469.4 | 7.0× faster |
| RSS delta MB | **1.2** | 200.1 | |
| startup ms | **0.6** | 15.9 | 26× (reported separately; not throughput) |

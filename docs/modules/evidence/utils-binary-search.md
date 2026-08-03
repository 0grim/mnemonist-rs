# utils/binary-search — evidence

Gate artifacts for `docs/modules/utils-binary-search.md`: test-to-gap table, full falsification
record.

## Test-to-gap mapping

| gap | test |
|---|---|
| — (upstream's own cases, transcribed) | `search_matches_the_upstream_suites_own_case`, `search_with_comparator_matches_the_upstream_suites_own_case`, `bounds_match_the_upstream_suites_own_cases`, `comparator_bounds_match_the_upstream_suites_own_cases`, `lower_bound_indices_matches_the_upstream_suites_own_case` |
| 1 | `explicit_bounds_window_the_search` — windows, empty windows, and `hi == 0` |
| 2 | `an_over_long_hi_reports_a_hit_at_a_hole` — the `49`, plus the two bounds' opposite reactions |
| 3 | `lower_bound_indices_defaults_hi_from_the_wrong_array` |
| 4 | `empty_arrays` — all seven functions |
| 5 | `duplicates_pin_the_midpoint_arithmetic` |
| 6 | `nan_is_reported_as_a_match_by_search` |
| 7 | `unsorted_input_is_deterministic_garbage` |
| 8 | `an_antisymmetric_comparator_hides_the_argument_order` and `the_two_comparator_families_take_their_arguments_in_opposite_orders` |
| the underlying property | `bounds_agree_with_a_linear_scan_exhaustively` — every non-decreasing array of length 0..=8 over `{0,1,2}` crossed with every needle in `-1..=3`, all six bound/search functions against a linear scan. 3,280 arrays; upstream checks two. |

## Falsification record (gate 6)

Performed through the native suite, since there is no original test file to turn red. Two
sabotages, each with its target assertion named **before** the run:

| sabotage | must break | result |
|---|---|---|
| `at()` → plain `&array[index as usize]` (delete the `undefined` model) | `an_over_long_hi_reports_a_hit_at_a_hole` | **red** — `index out of bounds: the len is 3 but the index is 49`, and every other test still passed, so the sabotage is precisely targeted |
| `search`'s `else` arm → `return -1` (delete the "neither greater nor less means equal" rule) | `nan_is_reported_as_a_match_by_search` | **red** — that test plus five others, including `search_matches_the_upstream_suites_own_case` |

Both reverted; 15/15 green afterwards. The first is the interesting one: it fails *only* the
assertion it was aimed at, which is what distinguishes a real falsification from a sabotage so broad
that any test would have caught it.

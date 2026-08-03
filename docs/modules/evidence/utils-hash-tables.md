# utils/hash-tables — evidence

Gate artifacts for `docs/modules/utils-hash-tables.md`: test-to-gap table, falsification record.
No log file for this unit — the source document had no dated or chronological material to move (see
the pilot report for why a log file is skipped when there is nothing to put in it).

## Test-to-gap mapping

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

## Falsification record (gate 6)

| sabotage | must break | result |
|---|---|---|
| `jenkinsInt32`'s `(a as i32) >> 19` → `a >> 19` (arithmetic shift becomes logical) | `jenkins_int32_matches_node_24_18_1` | **red**, together with `the_upstream_pairs_land_in_a_known_layout` — which is the point: the layout test is downstream of the hash, so a hash defect moves the data too |
| `linearProbingGet`'s branch order: `c === 0` tested before `c === key` | `the_key_zero_occupies_a_slot_that_still_reads_as_empty` | **red**, and *only* that test — the seven others do not store `0`, so nothing else could see it |

Both reverted; 9/9 green afterwards.

# critbit-tree-map — evidence

Gate artifacts for `docs/modules/critbit-tree-map.md`: test-to-gap table, fuzz grammar self-check
figures.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/critbit_tree_map.rs` — 9 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_set_suite`, `reproduces_the_upstream_delete_suite`, `keys_that_differ_only_in_length_do_not_break`, `for_each_visits_in_sorted_key_order`, `clear_resets_size_and_removes_everything` | the upstream blocks, as a baseline |
| `keys_differing_only_in_the_last_byte_route_correctly`, `a_deep_prefix_chain_is_fully_reachable` | the gate 6 falsification target: deep critical-bit positions and multi-level bubble-up |
| `a_shared_prefix_followed_by_a_nul_byte_still_routes_correctly` | the `0xff`-mask degenerate case |
| `setting_again_after_deleting_back_to_empty_does_not_point_root_at_a_stale_slot` | a port bug this unit's own differential fuzzer found (see the document's "Bugs this found") |

## Fuzz grammar self-check figures

`PREFIX_POOL`: `["a", "ab", "abc", "abcd", "abcda", "abcdb", "b", "ba"]`.

* `pool_self_check_most_entries_are_a_prefix_of_another_entry`: 5/8 entries are themselves a strict
  prefix of another entry.
* `pool_self_check_contains_a_pair_differing_only_in_the_last_byte`: `"abcda"`/`"abcdb"` differ only
  at byte index 4.
* `pool_self_check_generated_programs_revisit_prefix_relationships`: a 2,000-sample draw from the
  real `set` op strategy shows ~65% of generated `set` keys are a strict prefix of another
  generated `set` key across both regimes sampled.

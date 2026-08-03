# fixed-critbit-tree-map — evidence

Gate artifacts for `docs/modules/fixed-critbit-tree-map.md`: test-to-gap table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/fixed_critbit_tree_map.rs` — 10 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_set_suite`, `keys_that_differ_only_in_length_do_not_break`, `for_each_visits_in_sorted_key_order_when_exactly_at_capacity`, `rejects_a_zero_capacity` | the upstream blocks, as a baseline |
| `keys_differing_only_in_the_last_byte_route_correctly_within_capacity` | the gate 6 falsification target, within capacity |
| `exceeding_capacity_silently_corrupts_then_crashes_exactly_as_upstream_does`, `a_capacity_of_one_corrupts_on_the_second_key` | B-261, measured at two different capacities |
| `root_is_a_number_fresh_but_null_right_after_a_clear` | B-260 |
| `clear_empties_the_tree_but_does_not_shrink_the_backing_arrays`, `a_set_right_after_a_clear_reuses_index_zero_instead_of_panicking` | a port bug this unit's own differential fuzzer found (see the document's "Bugs this found") |

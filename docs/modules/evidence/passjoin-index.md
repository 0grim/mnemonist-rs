# passjoin-index — evidence

Gate artifacts for `docs/modules/passjoin-index.md`: test-to-gap table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/passjoin_index.rs` — 12 tests:

| Test | Closes gap |
|---|---|
| `comparator_sorts_by_decreasing_length_then_lexicographically`, `segments_matches_upstreams_pinned_examples`, `segment_pos_matches_upstreams_pinned_examples`, `multi_match_aware_interval_matches_upstreams_pinned_examples`, `multi_match_aware_substrings_matches_upstreams_pinned_groups` | the upstream blocks, transcribed against the exact same pinned literal examples |
| `constructor_rejects_invalid_k`, `reproduces_the_upstream_add_and_search_walkthrough`, `reproduces_the_upstream_sanity_walkthrough`, `for_each_and_values_walk_in_insertion_order`, `clear_resets_the_index` | the remaining upstream blocks, as a baseline |
| `search_results_are_in_upstreams_own_insertion_order` | `search`'s result *order*, not just its membership — see the document's "Bugs this found" for why this is load-bearing beyond what `assert.deepStrictEqual` on two `Set`s can ever catch |
| `try_search_propagates_a_failing_distance_function` | the untested throwing-`levenshtein` path |

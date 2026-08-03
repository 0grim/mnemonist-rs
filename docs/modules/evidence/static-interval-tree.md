# static-interval-tree — evidence

Gate artifacts for `docs/modules/static-interval-tree.md`: test-to-gap table, full falsification
record.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/static_interval_tree.rs` — 9 tests beyond
`reproduces_the_upstream_suite`:

| Test | Closes gap |
|---|---|
| `zero_intervals_is_refused_rather_than_silently_accepted` | 1 — B-100 |
| `a_two_entry_source_gives_a_height_of_two` | — a second, isolated height computation, since the upstream `Map` case bundles it with iterable resolution |
| `getters_resolve_to_the_same_bounds_object_values_survive` | — pins that a resolved `(start, end)` pair, not the getter itself, is what the core crate carries forward (see the bridge divergence below) |
| `a_single_interval_tree_has_one_node` | — the smallest possible tree, verified node-for-node against Node (`tree === [1]`, `augmentations === [0]`) |
| `ties_in_start_are_broken_by_original_insertion_order` | 2 |
| `intervals_are_closed_on_both_ends` | 4 — both boundaries, plus one point just outside each |
| `a_non_overlapping_query_interval_finds_nothing` | 5 |
| `a_larger_tree_answers_every_point_correctly` | 3 — fifty intervals, height beyond what five ever reaches, every point checked |
| `a_length_too_large_to_index_is_refused` | 7 |

## Falsification record (gate 6)

**Named first:** `static_interval_tree_matches_upstream`
(`crates/difffuzz/tests/differential.rs`) should go red, because the fuzz grammar's query points
land exactly on a generated interval's own `start` routinely (starts repeat across up to 40
intervals per tree, and query points are drawn from the same range), and the sabotage removes
exactly the inclusive comparison that admits that case.

**The sabotage:** `intervals_containing_point`'s match condition tightened from
`point >= start && point <= end` to `point > start && point <= end` — excluding a point exactly on
an interval's `start`, which reads as a plausible "shouldn't `start` be exclusive?" cleanup and is
the kind of one-character change a future refactor could make without noticing it flips a
documented, upstream-verified semantic (see `intervals_are_closed_on_both_ends`).

**Confirmed red**, on the campaign's very first case:

```
divergence in return value after op #43: intervalsContainingPoint(28)
  value:
    port:     []
    upstream: [[28,41]]
minimal repro:
var s = new StaticIntervalTree([[7,13],[118,158],[28,41],[77,80]]);
...
s.intervalsContainingPoint(28);
```

Reverted; **confirmed green again**: `static_interval_tree_matches_upstream ... ok`.

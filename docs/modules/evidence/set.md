# set — evidence

Gate artifacts for `docs/modules/set.md`: test-to-gap table, full falsification record, full
benchmark table.

## Test-to-gap mapping

| Gap | Where | What |
|---|---|---|
| 1 | `set.rs::intersection_order_follows_the_smallest_set`, `tests/boundary/set.js` "follow the SMALLEST argument" | Differentially, against vendored upstream |
| 2 | same two | The tie case, `[3, 2, 1]` |
| 3 | `set.rs::symmetric_difference_is_a_then_b`, boundary "A's half before B's" | Both argument orders |
| 4 | `set.rs::re_adding_does_not_move_but_delete_then_add_does`, boundary "not move a member that is re-added" | Both halves |
| 5 | boundary "should be a real Set" | `instanceof Set` and `constructor === Set`, for all four set-returning functions |
| 6 | boundary "fresh set even when difference short-circuits" | `notStrictEqual(result, A)` |
| 7 | boundary "the mutating four", "reach a live iterator through add/delete" | Object identity, and the replay-vs-rebuild distinction |
| 8 | `set.rs::the_two_variadic_functions_need_two_arguments`, boundary "arity", and the fuzz grammar | Both messages, at one argument and at zero |
| 9 | boundary "accept more than four sets" | Eight-way intersection and union |
| 10 | `set.rs::the_mutating_functions_applied_to_their_own_argument`, boundary "defined when applied to their own argument" | All four, differentially |
| 11 | boundary "NaN as one member", "-0 and 0 as one member" | Including that a `Set` stores `-0` as `+0` |
| 12 | boundary "distinguish 1 from \"1\"", and the fuzz member pool | |
| 13 | `set.rs::subset_and_superset_match_the_original_suite` | Empty in both positions, and `∅ ⊆ ∅` |
| 14 | `set.rs::the_ratios_answer_zero_rather_than_nan_when_nothing_is_shared`, boundary "0 rather than NaN" | |
| 15 | `set.rs::intersection_size_matches_the_original_suite`, boundary "argument order for intersectionSize" | Both orders, for four metrics |

## Falsification record (gate 6)

### Fuzzer falsification

Three sabotages were attempted. Two went red; the third stayed green and is recorded in the log as
a withdrawn claim.

Both red ones were chosen so `test/set.js` **stays green** under them — they measure what the
campaign adds over the original suite rather than what it duplicates. Confirmed 16 passing under
each, and green again after revert:

1. **`intersection` iterating the first set instead of the smallest.** Must break
   `intersection_order_follows_the_smallest_set` and the boundary spec's "follow the SMALLEST
   argument". Order-only — the members are identical — so it is invisible to any comparison that
   treats a set as unordered. Found in 109 cases (0.1s), shrunk to one op:
   `m.intersection(new Set([4, 0, "a", 5, "1", 1]), new Set([5, 4]))`, port `{4,5}` against
   upstream `{5,4}`.
2. **`add` inserting `B`'s members in reverse order.** Must break the boundary spec's
   "not move a member that is re-added". Also order-only, and on a function that returns
   `undefined`, so the entire divergence lives in the echoed first argument. Found in 82 cases
   (0.1s), shrunk to `m.add(new Set([]), new Set([0, 1]))`.

Both seeds are committed in `crates/difffuzz/proptest-regressions/set.txt` with a provenance block
saying they came from sabotages and not from real port defects.

### Falsification of the port (gate 6, on the original suite)

Making `disjunct` delete before deciding what to add must break `test/set.js`'s
**"#.disjunct → should properly disjunct the second set to the first."** —
`assert.deepStrictEqual(Array.from(A), [1, 3])`. Confirmed: 16 passing → **15 passing, 1 failing**,
that block and only that block, with
`AssertionError [ERR_ASSERTION]: Expected values to be strictly deep-equal`. Back to 16 after
revert. The other fifteen blocks staying green is the useful part — the sabotage is local to one
function and the suite localised it.

## Bench table

`bench/results.json` → `modules["set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 500 samples/side.

**`union-2e4x50`** — `union(A, B)` of two 20,000-element sets drawn from the SAME `0..20,000`
domain, 50 passes, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **13.2** | 25.0 | 1.9× faster |
| p99 ns/op | **17.6** | 89.2 | 5.1× faster |
| RSS delta MB | **7.8** | 120.9 | |
| structure-only RSS delta MB | **0.1** | 5.7 | |
| startup ms | **0.6** | 16.6 | 28× (reported separately; not throughput) |

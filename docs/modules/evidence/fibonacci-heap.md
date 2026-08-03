# fibonacci-heap — evidence

Gate artifacts for `docs/modules/fibonacci-heap.md`: test-to-gap table, fuzz grammar, full
falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/fibonacci_heap.rs` — 12 tests:

| Test | Closes gap |
|---|---|
| `push_increments_size`, `peek_reads_the_minimum_without_removing_it`, `pop_drains_in_ascending_order`, `a_max_heap_drains_in_descending_order`, `a_custom_comparator_orders_by_a_projected_field`, `from_iter_builds_a_heap_from_an_iterable` | the upstream blocks, as a baseline |
| `consolidation_merges_trees_across_many_pushes_and_pops` | 1 — 64 pushes then a full drain, asserting both the sorted output and a measured floor (`merges() >= 6`) on how many links one `pop` forces |
| `interleaved_push_and_pop_stays_sorted_and_merges_repeatedly` | 2 — a 400-step xorshift32-seeded interleaving of push and pop, checked against a reference `Vec` sort at every third step, plus a measured floor on total merges |
| `push_favours_the_most_recently_pushed_node_on_a_tie` | 3 — pins that a tie really does take the `<=` branch (see the falsification record for the sharper way this rule is actually pinned) |
| `a_comparator_may_re_enter_and_push` | 4 (growing) |
| `a_comparator_that_clears_the_heap_mid_pop_does_not_panic` | 4 (resetting) — and pins BUG-FIBONACCI-HEAP-1's exact `-1`, not merely "doesn't crash" |
| `a_pop_after_b_220s_negative_size_panics_matching_upstreams_null_dereference` (`#[should_panic]`) | the follow-on half of BUG-FIBONACCI-HEAP-1: the *next* `pop` after the corruption |

## Fuzz grammar

* **Op alphabet:** `push(v)` (weight 7, `v` drawn from `0..24` — small and repetitive, so ties are
  frequent rather than incidental) · `pop()` (5) · `clear()` (1). `program_len` is widened to
  `1..400` (`heap`'s own grammar uses `1..200`): this structure's whole point is `consolidate`'s
  degree-merging, which needs a population built up across many pushes before a `pop`'s degree
  bucketing has real work to do, not a handful of elements.
* **Constructor alphabet:** six comparator factories, mirrored name for name between
  `fuzz/oracle.js` and `crates/difffuzz/src/modules/fibonacci_heap.rs`. `ascending`/`descending`/
  `boom` are reused **verbatim** from `heap`'s own table — none of the three touches `.items`, so
  none needed a fibonacci-heap-specific version. Three are new, because this structure has no public
  backing array to mutate through: `fibPushy` (`instance.push(99)`), `fibPopper` (a **nested**
  `instance.pop()` — the shape that found this port's own arena defect), `fibClearer`
  (`instance.clear()` — BUG-FIBONACCI-HEAP-1/BUG-FIBONACCI-HEAP-3's trigger).
* **Observable state, compared after every op:** `size` and `peek`. There is no `.items` to compare
  against — `push`/`peek`/`pop`/`clear` are this structure's entire public surface. `size` is
  compared as the signed `i64` both sides can produce (DIV-FIBONACCI-HEAP-2); a campaign that clamped or ignored
  negative values here would have missed BUG-FIBONACCI-HEAP-1 entirely.

**Measured evidence that `consolidate` actually merges trees, repeatedly — not inferred from op
weights.** `grammar_self_check` (`crates/difffuzz/src/modules/fibonacci_heap.rs`, no oracle, no
`node`) runs 400 generated programs directly against the core structure and counts
[`FibonacciHeap::merges`](../../crates/mnemonist-core/src/structures/fibonacci_heap.rs), a
diagnostic counter incremented once per `link` call (not part of upstream's API):

```
fibonacci-heap grammar: 81477 ops, 16815 tree merges across 400 programs (369 of them saw at least one)
```

92% of generated programs (369/400) triggered at least one degree-merge, and the total is over 16.8
thousand links across those programs.

## Falsification record (gate 6)

Two attempts, targeting `consolidate` directly — the sharp target for this structure, not
`push`/`peek` — with contrasting outcomes, both reported honestly.

**Attempt 1 — confirmed red, dramatically.**

**The assertion named before running:** `interleaved_push_and_pop_stays_sorted_and_merges_repeatedly`'s
final `assert_eq!(popped, reference);`, plus `consolidation_merges_trees_across_many_pushes_and_pops`'s
sorted-drain assertion and, at the bridge, `test/fibonacci-heap.js`'s `should be possible to pop the
heap.` (`assert.strictEqual(heap.pop(), 1)`).

**The sabotage:** `consolidate`'s swap condition, `if (heap.comparator(x.item, y.item) > 0) { swap
}`, flipped from `> 0.0` to `< 0.0` — one character, in the branch that decides which of two
same-degree trees becomes the parent.

**Confirmed red, at every level checked:**

* Rust: **5 of 12 native tests failed**, both named targets included, plus two more
  (`a_max_heap_drains_in_descending_order`, `a_comparator_may_re_enter_and_push`) that depend on
  correct consolidation incidentally.
* The bridge: **the Node process aborted outright** —
  `thread '<unnamed>' panicked at ...: Cannot read properties of null (reading 'child') ... fatal
  runtime error: failed to initiate panic, error 5, aborting`. The sabotage corrupted the tree
  structure badly enough to reach BUG-FIBONACCI-HEAP-1's own panic site through completely ordinary (non-re-entrant)
  operation, and a Rust panic crossing the N-API boundary is not a catchable JS exception — it takes
  the whole process down. About as red as a falsification gets.

**Reverted; confirmed green again** at both levels: `12/12` native tests, `6/6` original suite.

**Attempt 2 — stayed green in this unit's own gates, caught by the fuzzer and by a sibling unit.**

**The assertion named before running:** none could be named with confidence in advance — the honest
prediction, stated before running, was "this may not be observable within this unit's own tests,
because ties between identical values carry no information."

**The sabotage:** `push`'s tie-break, `<= 0.0` (favour the just-pushed node on an exact tie) flipped
to `< 0.0` (favour the existing `min`) — BUG-FIBONACCI-HEAP-1/BUG-FIBONACCI-HEAP-2's neighbouring line, and the exact rule DIV-UTILS-2's
own fix in `utils/merge.rs` depends on.

**Result, confirmed and reported honestly:**

* This unit's 12 native tests: **all still pass**, `push_favours_the_most_recently_pushed_node_on_a_tie`
  included — both pushed values in that test are literally `5`, so which physical node "wins" a tie
  is unobservable through `peek()`.
* The original suite: **6/6 still pass** — no block in `test/fibonacci-heap.js` ever pushes two equal
  values.
* **The differential fuzzer caught it**, inside 425 generated cases:
  `new FibonacciHeap({"$factory":"fibClearer"}); push(10); push(10);` diverges on `peek()` after the
  very first `push` (`port: undefined`, `upstream: 10`) — a different comparator factory than the one
  being sabotaged exposed it, because `fibClearer`'s own re-entrant `clear()` interacts with the
  tie-break rule in a way two plain pushes do not.
* **The sibling unit's own regression caught it too:** `mnemonist_core::utils::merge`'s
  `merge_k_matches_upstreams_real_heap_on_the_case_that_found_div_utils_2` failed — DIV-UTILS-2's fix depends on
  this exact rule.

**Reverted; confirmed green again** at all four: `12/12`, `6/6`, a clean 3,000-case fuzz replay, and
`cargo test` workspace-wide (606 `mnemonist-core` tests passing).

## Bench table

`bench/results.json` → `modules["fibonacci-heap"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 2,000 samples/side.

**`mixed-2e5`** — 200,000 mixed `push`/`pop`/`peek` (50/25/25), default numeric comparator, `size`
200,000, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **1251.72** | 31260.54 | 25× faster |
| p99 ns/op | **2667.66** | 64227.06 | 24× faster |
| RSS delta MB | **5.6** | 291.8 | |
| structure-only RSS delta MB | **0.1** | 6.7 | |
| startup ms | **0.6** | 15.3 | 26× (reported separately; not throughput) |

`FibonacciHeap::merges` measured directly at **195,920 merges over 50,000 `pop` calls** for this
exact op mix — ~3.9 merges per `pop`. At the 1e6-op scale used to sanity-check this workload's size,
the same ratio held: 985,004 merges over 250,000 pops.

Checksum `5003154165`, identical on both sides.

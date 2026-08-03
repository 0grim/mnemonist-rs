# _utils — evidence

Gate artifacts for `docs/modules/_utils.md`: test list, fuzz grammar exclusions, full falsification
record.

## Native test list

`crates/mnemonist-core/src/utils/merge.rs`'s native tests — the upstream suite's own cases,
transcribed, plus: `NaN` in a union's dedup check
(`nan_is_never_deduplicated_by_the_union_dedup_check`), ties across k-way arrays that do *not*
change the merged multiset (`ties_across_arrays_do_not_affect_the_merged_multiset`), BUG-UTILS-1 isolated
at its sharpest (`merge_k_reproduces_b_180_when_filtering_drops_the_length`,
`union_unique_k_reproduces_b_180_when_filtering_drops_the_length`), the boundary where filtering
down to two-or-fewer arrays takes the early-return path *before* the stale-length bug could ever
fire (`filtering_down_to_two_or_fewer_never_reaches_the_bug`), and `intersection_unique_k`'s
structural immunity to BUG-UTILS-1 (`intersection_unique_k_is_immune_to_bug_utils_1`).

`crates/mnemonist-core/src/utils/typed_arrays.rs`'s new tests for `getNumberType`/
`getMinimalRepresentation`/`concat` — every priority-table boundary and `-0` taking the
non-negative branch.

## Fuzz grammar

Two 60-second campaigns, seeds `42` and `20260801`: **508,729 + 503,372 = 1,012,101 operations,
zero divergences** on the final grammar.

* The grammar manufactures BUG-UTILS-1 deliberately: a 0-length array alongside two-or-more non-empty
  ones in a three-to-five-array group.
* The k-way (three-or-more array) generator draws **globally distinct** values across the whole
  group, so array-head ties do not confound the value comparison — this removes the only condition
  (a tie) under which the FibonacciHeap tie-break question was ever unreachable; BUG-UTILS-1 stays fully
  reachable regardless, since it depends only on array counts, never on value content.
* The two-array generator keeps duplicates and `NaN` freely, because `merge_two`/`union_unique_two`
  have no tie-break step to disagree about (verified: `merge([-5, NaN], [-1, 3])` matches upstream
  with `NaN` present).

**Deliberately excluded from the grammar**, each for a stated, structural reason:

* **`getPointerArray`/`getMinimalRepresentation`.** Both return a real JS *constructor*, which
  `fuzz/oracle.js`'s `encode` has no case for — it falls through unmodified and `JSON.stringify`
  drops the property outright. Nothing to compare through this protocol; covered by native Rust
  tests pinned against Node and by the real bridge integration run instead.
* **The three `WithComparator` binary-search variants.** The comparator is a JavaScript function
  called from inside the search loop; comparing against it here would mean rendering an equivalent
  comparator as JS source per generated case — real re-entrant-callback machinery this unit was
  scoped to avoid at the fuzz-grammar level (it is still exercised, once, at the bridge). Covered
  instead by `crate::utils::binary_search`'s pre-existing exhaustive native tests.
* **`iterables`.** No core-side pure function exists to fuzz (`docs/modules/utils-iterables.md`).
* **All of `hash-tables.js`.** Its two exports (`hashes`, `linearProbing`) are both plain objects —
  upstream never exports a bare top-level function at all — and the oracle's free-function protocol
  dispatches with a single property lookup, `instance[name](...)`. There is no
  `instance.linearProbingSet`, only `instance.linearProbing.set`; extending the oracle to walk a
  dotted path would be a structural change to a shared file (`fuzz/oracle.js`) that this project's
  convention reserves for additive edits only. Covered instead by `crate::utils::hash_tables`'s own
  extensive native tests and the real bridge run.

## Falsification record (gate 6)

Two attempts named a real assertion and stayed **green**:

1. Relaxed `k_way_scan`'s tie-break (`<` to `<=`, favouring the latest array on a tie). Target:
   `'should properly merge k arrays.'`. The test's own tie resolves to identical values regardless
   of which array supplies them — unobservable there.
2. Reversed `merge_two`'s swap condition (`a[0] > b[0]` to `a[0] < b[0]`). Target:
   `'should properly merge two arrays.'`, the `[4, 5, 6]`/`[1, 2, 3]` case. The swap is a
   fast-path optimisation, not a correctness requirement of the (side-symmetric) two-pointer walk —
   unobservable.

Third attempt, **confirmed red**: reversed the overlap loop's comparison (`a_head <= b_head` to
`a_head >= b_head`). Target: `'should properly merge two arrays.'`, case
`[[1, 2, 2, 3], [2, 3, 3, 4], [1, 2, 2, 2, 3, 3, 3, 4]]`. Result: **25 passing / 1 failing**, exactly
that assertion, actual `[1, 2, 2, 3, 2, 3, 3, 4]` (unsorted) against expected
`[1, 2, 2, 2, 3, 3, 3, 4]`. Reverted; **confirmed green again**, 26/26.

The two green attempts are not filler: they establish, empirically rather than by assertion alone,
that the two-array functions are tie-order- and swap-side-invariant — which is exactly the property
that stops holding once a third array can interleave.

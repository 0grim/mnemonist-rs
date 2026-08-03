# _utils — working log

Chronological. See `docs/modules/_utils.md` for the current-state document and
`docs/modules/evidence/_utils.md` for the gate artifacts.

## Porting history for this unit

Four of the five files in this require-closure (`typed-arrays`, `binary-search`, `hash-tables`,
`iterables`) were already ported as standing infrastructure by earlier work, each carrying its own
"this is not a unit yet" note in its module docs pointing at this file. This series ported the
fifth, `merge.js`, which is what turned that infrastructure into gate 4 evidence for `test/_utils.js`
for the first time.

This was not quite "the cheapest large unit remaining," which is what it looked like going in.
Four of the five files are pure numeric functions with no surprises. `merge.js`'s k-way algorithms
drive a `FibonacciHeap` internally, and two of `binary-search.js`'s seven functions take a
JavaScript comparator — a small, one-shot instance of the re-entrant-callback work the unit was
expected to have none of. Both turned out tractable (at first, a linear-scan substitute for the
heap; later, a real `FibonacciHeap` once that unit existed — see below; and a "sticky error"
wrapper for the comparator, reusing `crate::vector`'s existing shape for a fallible callback inside
an infallible core signature) without requiring a new unit, but neither was free.

## DIV-UTILS-2 — the k-way tie-break was a linear scan's, not a real FibonacciHeap's (CLOSED)

At the time this unit was first ported, `fibonacci-heap` did not yet exist as a ported unit, so the
k-way merge/union's tie-break was approximated with a linear scan rather than driven by a real
`FibonacciHeap`. This was recorded as divergence DIV-UTILS-2, with the gap stated plainly: three-or-more
array ties were untested upstream and the port's linear-scan approximation disagreed with a real
heap's tie-break, observably, on both `merge`'s element order and `unionUnique`'s deduplication.

Once `fibonacci-heap` became a ported unit, DIV-UTILS-2 was closed: `k_way_scan` now drives a real
`FibonacciHeap<usize, KWayKeyComparator, Thrown>` — upstream's own inline comparator closure,
translated directly, over array indices with `pointers` read fresh per comparison. The fuzz grammar
was widened back to a tie-producing, `NaN`-including pool for `merge`/`unionUnique` accordingly. The
exact case that had exposed the disagreement (`merge([3], [2, -5], [2])`) is pinned as
`merge_k_matches_upstreams_real_heap_on_the_case_that_found_div_utils_2`, run against the real heap's
actual output.

DIV-UTILS-2 no longer appears in the current document's divergences table, since the port no longer
diverges from upstream on this point. DIV-UTILS-3 (`intersectionUnique`'s separate, still-open `NaN`
gap) was never part of DIV-UTILS-2 and remains open, described in the current document.

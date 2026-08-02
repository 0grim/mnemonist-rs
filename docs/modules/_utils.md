# _utils

Upstream: `utils/typed-arrays.js` (169 LOC), `utils/binary-search.js` (216 LOC),
`utils/hash-tables.js` (107 LOC), `utils/iterables.js` (93 LOC), `utils/merge.js` (563 LOC) — **five
files, ~1,166 LOC** · `test/_utils.js` — **389 lines, 20 distinct `it` blocks (26 executions across
one dynamically-repeated group), roughly 60 assertion statements**.

Port: `crates/mnemonist-core/src/utils/{typed_arrays,binary_search,hash_tables,merge}.rs` (pure
computation); `crates/mnemonist-napi/src/iterables.rs` (the one JS-value member — see
`docs/modules/utils-iterables.md`). Bridge: `crates/mnemonist-napi/src/{typed_arrays,binary_search,
hash_tables,merge}.rs`. Shim: `tests/bridge/_utils.js` (the hub, assembling all five files' export
shape, same convention `tests/bridge/sort.js` established) plus its five
`tests/bridge/utils/*.js` leaves. Fuzz spec: `crates/difffuzz/src/modules/_utils.rs`.

This unit is named specifically as one whose require-closure spans several standing files:
`test/_utils.js`'s require-closure is
`typed-arrays` + `binary-search` + `hash-tables` + `iterables` + `merge`, and — despite the
underscore — this is a real upstream test file, not a helper, so all five must exist before one
assertion in it can run. Four of the five were already ported as standing infrastructure by earlier
work (each carries its own "this is not a unit yet" note in its module docs, pointing at this file);
this pass ported the fifth, `merge.js`, and is what turns that infrastructure into gate 4 evidence
for the first time.

This was **not** quite "the cheapest large unit remaining" as expected going in. Four of the five
files are pure numeric functions with no surprises. `merge.js`'s k-way algorithms drive a
`FibonacciHeap` (an unported, separate T2-tier unit) internally, and two of `binary-search.js`'s
seven functions take a JavaScript comparator — a small, one-shot instance of the re-entrant-callback
work the unit was expected to have none of. Both turned out tractable (a linear-scan substitute for
the heap; a "sticky error" wrapper for the comparator, reusing `crate::vector`'s existing shape for
a fallible callback inside an infallible core signature) without requiring a new unit, but neither
was free, and the heap substitution left a real, stated gap — see "Deliberate divergences".

---

## What upstream tests

Twenty `it` blocks (one, `#.getPointerArray`, is a factory called three times, for 26 executions
total), organised by file:

* **`typed-arrays`** — `getPointerArray` at the three width boundaries (nine cases, all from one
  helper), a throw past `2^32`, `getMinimalRepresentation` over one three-case array (`Uint8Array`,
  `Int8Array`, `Float64Array`), and `concat` over three real `Uint8Array`s (pairwise and out of
  order).
* **`binary-search`** — `search`/`searchWithComparator` over one five-element ascending array (and
  its descending, comparator-inverted twin); `lowerBound`/`lowerBoundWithComparator` and
  `upperBound`/`upperBoundWithComparator` over one nine-element array with two duplicate runs, seven
  probe points each; `lowerBoundIndices` over one seven-element array and its argsort, checked
  against `lowerBound` on the materialised sorted array.
* **`hash-tables`** — one `it()`: eight `(key, value)` pairs into an eight-slot table via
  `jenkinsInt32`, read back, membership-checked, a ninth insert that must throw `/full/`, and one
  miss each for `get`/`has`.
* **`merge`** — `merge`/`unionUnique`/`intersectionUnique`, each with a two-array block (six to
  eight small hand-picked pairs) and a k-array block (four arrays, always either all-empty or
  all-non-empty).
* **`iterables`** — one `it()`: `toArrayWithIndices` over an array, a `Set`, and a bare iterator.

Every array, in every block, is already sorted (where sortedness matters), already internally
unique (where uniqueness matters), and every k-array case is either all-empty or has no empty
member at all. Nothing here supplies a malformed, unsorted, or partially-empty input to anything.

## What upstream does NOT test

**`merge.js`, the newly-ported file, has the sharpest gaps:**

1. **B-180.** A k-way `merge`/`unionUnique` call where some but not all arrays are empty and
   three-or-more remain live throws a `TypeError` upstream never catches or documents — see "Bugs
   this found". Not one k-array test case here mixes an empty array with two-or-more non-empty
   ones.
2. **Unsorted, duplicate-heavy, and internally non-unique inputs.** Every algorithm here is
   duck-typed and unchecked; upstream's own suite never feeds it anything but well-formed sorted
   (and, for the unique variants, deduplicated) arrays. What actually happens on a malformed input
   is exactly where this port and upstream first disagreed — twice (see "Bugs this found").
3. **Three-or-more-array ties.** No k-array test case has any array head tie with another, so the
   `FibonacciHeap`'s tie-break behaviour is completely untested upstream, not merely lightly tested.
   **Update: this port now reproduces it rather than disagreeing with it** — D-105 is closed (see
   "Deliberate divergences" and `docs/modules/fibonacci-heap.md`); `k_way_scan` drives a real
   `FibonacciHeap`, the same one upstream's own `kWayMergeArrays`/`kWayUnionUniqueArrays` build.
4. **`NaN` anywhere in `merge`/`unionUnique`/`intersectionUnique`.** Every value in every test case
   is a plain finite integer. The differential fuzz grammar now covers `NaN` in three-or-more-array
   groups for `merge`/`unionUnique` (widened alongside D-105's closure); `intersectionUnique`'s own
   k-way `NaN` handling is a *separate*, still-open, pre-existing gap — D-105 never touched it, since
   `kWayIntersectionUniqueArrays` has no heap at all. See "Deliberate divergences".

**Elsewhere:**

5. **`binary-search`'s out-of-range `lo`/`hi`.** Every probe in the suite stays within
   `0..array.length`; an inverted window (`hi < lo`) or one extending past the array is untested,
   even though neither function validates its bounds (`crate::utils::binary_search`'s own module
   docs cover this at length, independently of this unit).
6. **`hash-tables`'s key `0`, a non-power-of-two table, and a zero-length one.** The suite's single
   example uses an eight-slot (power-of-two) table and none of its eight keys is `0`
   (`crate::utils::hash_tables`'s own docs, B-92/B-94).
7. **`getMinimalRepresentation`'s optional `getter` argument.** Never supplied; not ported (helpers
   land as callers reach them, same policy `indices` already established).
8. **A custom hash function** passed to `linearProbing.*` — the suite only ever passes
   `hashes.jenkinsInt32`.

## What we test in addition

* **This unit's own two real bugs**, both found by differential fuzzing inside the first fuzz
  campaign ever run against this grammar — see "Bugs this found" below.
* **`crates/mnemonist-core/src/utils/merge.rs`'s native tests** — the upstream suite's own cases,
  transcribed, plus: `NaN` in a union's dedup check (`nan_is_never_deduplicated_by_the_union_dedup_check`),
  ties across k-way arrays that do *not* change the merged multiset
  (`ties_across_arrays_do_not_affect_the_merged_multiset`), B-180 isolated at its sharpest
  (`merge_k_reproduces_b_180_when_filtering_drops_the_length`,
  `union_unique_k_reproduces_b_180_when_filtering_drops_the_length`), the boundary where filtering
  down to two-or-fewer arrays takes the early-return path *before* the stale-length bug could ever
  fire (`filtering_down_to_two_or_fewer_never_reaches_the_bug`), and `intersection_unique_k`'s
  structural immunity to B-180 (`intersection_unique_k_is_immune_to_b_180`).
* **`crates/mnemonist-core/src/utils/typed_arrays.rs`'s new tests** for `getNumberType`/
  `getMinimalRepresentation`/`concat` — every priority-table boundary (including the sharp,
  counter-intuitive one where `Uint32Array` tops out at `i32::MAX`, not `u32::MAX`, because
  `value === (value | 0)` fails for anything ToInt32 would make negative — verified against Node
  24.18.1) and `-0` taking the non-negative branch exactly as `Math.sign(-0) !== -1` does.
* **`tests/boundary/iterables.js`** (pre-existing, 19 specs) and `crate::utils::binary_search`'s own
  exhaustive agreement-with-a-linear-scan test (pre-existing) both already cover the members this
  pass did not touch; not re-described here.
* **The differential-fuzz campaign** (`crates/difffuzz/src/modules/_utils.rs`) — see "Fuzz + bench".

## Bugs this found

**B-180 — the k-way `merge`/`unionUnique` throw a `TypeError` whenever filtering an empty array out
leaves three-or-more arrays live.** `status: VERIFIED against Node 24.18.1`. `kWayMergeArrays` and
`kWayUnionUniqueArrays` both capture `l = arrays.length` *before* filtering empty inputs out into
`filtered`, then reassign `arrays = filtered` and seed a `FibonacciHeap` with `l` indices — more
than `filtered.length` whenever anything was filtered out. The first `heap.pop()` that touches one
of the extra indices reads `arrays[p]` (`undefined`) and indexes it, throwing
`TypeError: Cannot read properties of undefined (reading 'undefined')`. Confirmed directly against
a real `pm-recon/mnemonist` v0.40.4 checkout:

```js
merge.merge([], [1, 2, 3], [4, 5, 6], [4, 7])        // throws
merge.unionUnique([1, 2], [], [3, 4], [5, 6])        // throws
merge.merge([1, 2], [], [3, 4])                       // OK -- filtered.length is 2, early return
merge.intersectionUnique([], [1, 2, 3], [4, 5, 6])   // OK -- returns [] before any heap exists
```

`kWayIntersectionUniqueArrays` has no heap at all (a sequential binary-search fold) and returns `[]`
on the first empty array it scans, before the stale-`l` code path is reachable — structurally
immune, not merely untested. Reproduced in the port as `KWayError::StaleLengthMismatch` (D-104).

Two further findings are **port defects, not upstream's**, and get no B-number ("do not
overclaim causation" cuts the other way too):

1. `union_unique_two`'s prefix loop deduplicated an internally non-unique input where upstream's own
   prefix loop pushes unconditionally (only its overlap and filling loops dedup). Found by
   differential fuzzing inside the first 300 generated cases; fixed.
2. The k-way linear scan's tie-break disagreed with `FibonacciHeap`'s own, observably, on both
   `merge`'s element order and `unionUnique`'s deduplication. **Fixed — D-105 is now closed**: see
   "Deliberate divergences" and `docs/modules/fibonacci-heap.md`. The exact case that found this
   (`merge([3], [2, -5], [2])`) is pinned as a Rust test,
   `merge_k_matches_upstreams_real_heap_on_the_case_that_found_d_105`
   (`crates/mnemonist-core/src/utils/merge.rs`), against the real heap's actual output.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-104 | **B-180 is reproduced as `Err(KWayError::StaleLengthMismatch)`, not a panic.** | `mnemonist-core` has no exceptions and forbids `unsafe`, so the actual out-of-bounds mechanism cannot be reproduced; the outcome (the k-way call fails, with upstream's message available at the boundary) is. Same convention as D-44 (`hash_tables::TABLE_IS_FULL`). |
| D-105 | **CLOSED.** Was: the k-way merge/union's tie-break was a linear scan's, not a real `FibonacciHeap`'s. | `fibonacci-heap` is now a ported unit. `k_way_scan` drives a real `FibonacciHeap<usize, KWayKeyComparator, Thrown>` — upstream's own inline comparator closure, translated directly, over array indices with `pointers` read fresh per comparison. The fuzz grammar (`crates/difffuzz/src/modules/_utils.rs`) is widened back to a tie-producing, `NaN`-including pool for `merge`/`unionUnique`. |
| D-106 | **`intersectionUnique`'s k-way `NaN` handling is a separate, still-open gap.** | `kWayIntersectionUniqueArrays`/`intersection_unique_k` never used a heap — D-105 never applied to it. Upstream seeds its running bounds from JS's `-Infinity`/`Infinity` sentinels; this port seeds from `Option<T>`, so the *first* array scanned always sets the accumulator, `NaN` included, where upstream's sentinel can survive past a `NaN`-headed array. Reachable only once `NaN` participates in a three-or-more-array group; the fuzz grammar's `k_way_arrays_op` takes an `allow_nan` flag that stays `false` for `intersectionUnique` specifically so D-105's widening does not silently paper over this different gap. See `crates/mnemonist-core/src/utils/merge.rs`'s `intersection_unique_k` module docs for the mechanism. |
| — | **`concat` supports `Uint8Array` only.** | `test/_utils.js`'s own case never constructs anything else; upstream is generic over any typed-array class via `arguments[0].constructor`. Same "helpers land as callers reach them" policy as `indices`. |
| — | **`getMinimalRepresentation`'s optional `getter` argument is not ported.** | Never supplied by any test in scope; same policy. |
| — | **A custom `linearProbing` hash function is supported at the bridge (a real JS callback), but never fuzzed.** | `test/_utils.js` only ever passes `jenkinsInt32`; fuzzing an arbitrary hash would need the same re-entrant-callback machinery as the comparator exclusion below, for a capability nothing in scope exercises. |

## Fuzz + bench

### Fuzz

Two 60-second campaigns logged (`fuzz/log.txt`, `module=_utils`), seeds `42` and `20260801`:
**508,729 + 503,372 = 1,012,101 operations, zero divergences** on the final grammar. The path there
was not clean on the first attempt, which is the point of fuzzing this unit at all — three real
findings surfaced inside the first few hundred cases of the very first run, all reported above
rather than smoothed over:

1. B-180 was already known from reading; the campaign's grammar deliberately manufactures it (a
   0-length array alongside two-or-more non-empty ones in a three-to-five-array group) rather than
   relying on luck.
2. The `union_unique_two` prefix-loop defect (a real port bug) surfaced inside the first ~300 cases
   and was fixed before any campaign was logged.
3. The `FibonacciHeap`-tie-break gap (D-105) surfaced immediately after, on a three-array case with
   a tied value. Rather than fix the unfixable-without-a-new-unit, the k-way (three-or-more array)
   generator was changed to draw **globally distinct** values across the whole group — which does
   not hide the gap, it removes the *only* condition (a tie) under which it is reachable. B-180
   stays fully reachable, since it depends only on array counts, never on value content. The
   two-array generator keeps duplicates and `NaN` freely, because `merge_two`/`union_unique_two`
   have no tie-break step to disagree about (verified: `merge([-5, NaN], [-1, 3])` matches upstream
   with `NaN` present).

**Deliberately excluded from the grammar**, each for a stated, structural reason rather than
convenience:

* **`getPointerArray`/`getMinimalRepresentation`.** Both return a real JS *constructor*, which
  `fuzz/oracle.js`'s `encode` has no case for — it falls through unmodified and `JSON.stringify`
  drops the property outright. Nothing to compare through this protocol; covered by native Rust
  tests pinned against Node and by the real bridge integration run instead.
* **The three `WithComparator` binary-search variants.** The comparator is a JavaScript function
  called from inside the search loop; comparing against it here would mean rendering an equivalent
  comparator as JS source per generated case — real re-entrant-callback machinery this unit was
  scoped to avoid at the fuzz-grammar level (it is still exercised, once, at the bridge — see
  below). Covered instead by `crate::utils::binary_search`'s pre-existing exhaustive native tests.
* **`iterables`.** No core-side pure function exists to fuzz (`docs/modules/utils-iterables.md`).
* **All of `hash-tables.js`.** Its two exports (`hashes`, `linearProbing`) are both plain objects —
  upstream never exports a bare top-level function at all — and the oracle's free-function protocol
  dispatches with a single property lookup, `instance[name](...)`. There is no
  `instance.linearProbingSet`, only `instance.linearProbing.set`; extending the oracle to walk a
  dotted path would be a structural change to a shared file (`fuzz/oracle.js`) that this project's
  convention reserves for additive edits only. Covered instead by `crate::utils::hash_tables`'s own extensive native
  tests and the real bridge run.

### Falsification (gate 6)

Two attempts named a real assertion and stayed **green** — reported rather than discarded, because
each is itself a finding about this unit's structure:

1. Relaxed `k_way_scan`'s tie-break (`<` to `<=`, favouring the latest array on a tie). Target:
   `'should properly merge k arrays.'`. The test's own tie resolves to identical values regardless
   of which array supplies them — unobservable there, consistent with D-105's analysis.
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
that stops holding once a third array can interleave (D-105).

### Bench

**Excluded, deliberately, and not merely deferred** — considered directly during the final gate-10
batch (the last fourteen units) and not benchmarked, for a reason specific to what `_utils` actually
is rather than for lack of an idle machine.

Every other module in this project's gate-10 harness (`bench/runner/src/harness.rs`'s
`ModuleEntry`) benchmarks **one structure**: either a mixed op-stream against one persistent
instance, or — for the handful of function-only units already in scope (`sort`, `suffix-array`,
`set`) — one representative function, called once per timed sample, standing in for that unit
because the unit genuinely *is* one function (or a tightly related pair, like
`inplaceQuickSortIndices`/`quickSort`). `_utils` is not that shape: its require-closure is **five
unrelated files** — `typed-arrays`, `binary-search`, `hash-tables`, `merge`, `iterables` — with no
shared instance, no shared complexity class, and (per the unit definition used throughout this
port) they are one gate-10 unit only because one upstream test file, `test/_utils.js`, happens to
require all five.

Two shapes were considered and both rejected:

1. **Pick one function to stand in for all five**, the way `set`'s bench picked `union` out of
   `set.js`'s fourteen free functions. That precedent works because `set.js` is fourteen operations
   over the *same* kind of value (native `Set`s) — any one of them says something representative
   about the file. `_utils`'s five files share nothing: a k-way `merge` call, a `linearProbing.set`
   into a fixed-size table, and a `lowerBound` binary search are different algorithms over different
   data shapes with different complexity classes. Reporting `bench/results.json`'s `_utils` key
   against, say, `merge` alone would silently omit `binary-search`, `hash-tables`, and `typed-arrays`
   entirely while a reader has no way to tell that from the key name — the exact "a number nobody
   can trust" failure mode described above, just at the level of *which function ran* rather
   than *which parameter was chosen*.
2. **One workload per file, several rows under one `_utils` module entry.** `bench/drive.js`'s
   `WORKLOADS` table already supports several named rows per module (`static-disjoint-set`'s
   `mixed-1e6`/`mixed-4e6` is exactly this). What it cannot support is what those five rows would
   need: `harness.rs`'s `ModuleEntry` wires exactly one Rust `MixedFn` *or* one `DrainFn` per module
   name, so every row for a given module calls the *same* function at different parameters — the
   registry has no notion of "row N calls a different underlying function than row N+1." Five
   genuinely different operations would need five different module *names* (`utils-merge`,
   `utils-binary-search`, …), which would then not be `_utils` in `results.json` at all, contradicting
   `tests/verify.sh`'s gate 10 check (`modules["_utils"].workloads`) and the one-unit
   framing established for this require-closure.

Two of the five files also carry reasons of their own not to force into any bench shape, echoing
this unit's own fuzz-grammar exclusions above: **`iterables`** has no core-side pure function at
all (`docs/modules/utils-iterables.md`) — there is nothing to *put* in a Rust `run_mixed`, only a
bridge-side JS-value concern, the same reason it was excluded from the differential fuzz grammar.
**The `WithComparator` binary-search variants** take a JavaScript comparator called from inside the
search loop — timing them honestly would mean rendering an equivalent closure per call, real
re-entrant-callback machinery this unit was scoped to avoid at the fuzz-grammar level for the exact
same reason (see "Fuzz" above).

`typed-arrays`' three functions (`getPointerArray`, `getMinimalRepresentation`, `concat`) and
`hash-tables`' `linearProbing` are more benchmarkable in isolation — each is a single, pure,
non-comparator function over a plain array — but benchmarking them alone while `merge` and
`binary-search` sit out would be exactly the first rejected shape above with extra steps: a `_utils`
key that quietly represents 40% of the unit's own surface. Nothing here rules out a **future,
separate** gate-10 extension that gives each of `typed-arrays`/`binary-search`/`hash-tables`/`merge`
its own module name (sidestepping objection 2 by not calling itself `_utils` at all); that is a
harness change out of scope here, stated so the reasoning does not need to be rediscovered later.

`_utils` is consequently **not** added to `tests/scope.txt` by this pass — consistent with
`default-weak-map`'s own precedent (`docs/modules/default-weak-map.md`'s "### Bench"): a stated
exclusion, backed by a structural reason specific to the module, rather than a number nobody could
trust.

## Gate 10 exemption

This unit carries an explicit benchmark exemption, recorded in `bench/results.json` and enforced
by `tests/verify.sh`: the gate accepts an exemption only when a reason is present and this section
exists.

A require-closure of five unrelated pure-function files (typed-arrays, binary-search, hash-tables, iterables, merge) with no shared instance. The benchmark harness keys one workload per module name, so a single entry would misrepresent all five, and splitting them would no longer describe the unit named in the scope manifest. Correctness is covered by gates 1-9.

The distinction being drawn is between a gate that was not *satisfied* and one that is not
*applicable*. Gates 1 through 9 apply to this unit in full and all pass.

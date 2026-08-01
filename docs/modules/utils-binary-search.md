# utils/binary-search

Upstream: `utils/binary-search.js` (216 LOC) · **no test file of its own.**

Port: `crates/mnemonist-core/src/utils/binary_search.rs`. No bridge: nothing in the upstream test
corpus reaches these functions through a structure yet, and `test/_utils.js` — the only file that
requires them — cannot run.

---

## Scope note: this is not a "unit" by DESIGN.md §1.1

A unit is the require-closure of one upstream *test file*. `utils/binary-search.js` has none. The
only assertions that touch it live in `test/_utils.js`, whose closure is
`typed-arrays` + `binary-search` + `hash-tables` + `iterables` + `merge` — and `iterables` and
`merge` are not ported, so **not one of that file's 389 lines can execute today.** A single missing
sibling makes the whole file fail with zero partial credit; that is the rule §1.1 exists to state.

So this file gets gates **1** (ported), **2** (`forbid(unsafe_code)`, zero deps), **7** (native
tests) and **8** (this document), exactly as `utils/bitwise` did. It will never appear in
`tests/scope.txt` on its own. It is a member of the eventual `_utils` unit, and a hard dependency of
`utils/merge` and `vp-tree`.

## What upstream tests

`test/_utils.js` has a `describe('binary-search')` block with four `it()`s:

* **`#.search` / `#.searchWithComparator`** — one ascending array of five distinct integers, each
  element looked up by value, plus one miss. The comparator case is the same shape on a *descending*
  array with a comparator that inverts it.
* **`#.lowerBound` / `#.lowerBoundWithComparator`** — one nine-element array with runs of length 3
  and 2, probed at seven points: below the range, above it, and at the start of each run. The
  comparator variant repeats it over number *names* resolved through a lookup object.
* **`#.lowerBoundIndices`** — one seven-element array with an argsort, checked at seven needles
  against `lowerBound` on the materialised sorted array. This is the only place the indirect
  variant is exercised at all.
* **`#.upperBound` / `#.upperBoundWithComparator`** — the same nine-element array as `lowerBound`,
  the same seven probes.

Coverage is therefore: **one array shape per function, all distinct-or-run integers, no explicit
`lo`/`hi`, no empty input.** Every assertion is a hand-written constant.

## What upstream does NOT test

1. **Explicit `lo` and `hi`.** Four of the seven functions take them; not one assertion passes
   either. The entire windowing behaviour is unexercised — including that `search` decrements `hi`
   before the loop and the bound functions do not.
2. **Out-of-range bounds.** The consequence is not a thrown error, it is a wrong answer:
   `search([1,2,3], 9, 0, 100)` returns `49`, because `undefined` loses both `>` and `<` and the
   `else` arm reports a match. Verified on Node 24.18.1.
3. **`lowerBoundIndices` with a short `indices` array.** Its `hi` defaults to `array.length`, not
   `indices.length` — the one place in the file the default bound is read off the wrong array.
   `lowerBoundIndices([0..7], [0,1], 1)` is `8`, a position that does not exist in `indices`.
4. **Empty haystacks.** Zero coverage. This is where `search`'s `hi--` produces `-1` before the
   loop runs, the only input that exercises that path.
5. **Which index `search` returns inside a run of equal values.** Upstream's arrays are distinct, so
   the midpoint arithmetic itself is never pinned. `search([7 × 9], 7)` is `4`, and nothing upstream
   would notice if it became `0` or `8`.
6. **`NaN` in the haystack.** `NaN` fails both comparisons, so `search` reports it as a *match*:
   `search([NaN, NaN, NaN], 1)` is `1`.
7. **Unsorted input.** Not meaningful, but deterministic; nothing pins it, so a changed branch order
   would go unnoticed.
8. **The comparator argument-order difference** between `searchWithComparator` and the two bound
   functions. Upstream's own comparators are antisymmetric, which hides it completely.

## What we test in addition

Mapped 1:1 to the list above.

| gap | test |
|---|---|
| — (upstream's own cases, transcribed) | `search_matches_the_upstream_suites_own_case`, `search_with_comparator_matches_the_upstream_suites_own_case`, `bounds_match_the_upstream_suites_own_cases`, `comparator_bounds_match_the_upstream_suites_own_cases`, `lower_bound_indices_matches_the_upstream_suites_own_case` |
| 1 | `explicit_bounds_window_the_search` — windows, empty windows, and `hi == 0` |
| 2 | `an_over_long_hi_reports_a_hit_at_a_hole` — the `49`, plus the two bounds' opposite reactions |
| 3 | `lower_bound_indices_defaults_hi_from_the_wrong_array` |
| 4 | `empty_arrays` — all seven functions |
| 5 | `duplicates_pin_the_midpoint_arithmetic` |
| 6 | `nan_is_reported_as_a_match_by_search` |
| 7 | `unsorted_input_is_deterministic_garbage` |
| 8 | `an_antisymmetric_comparator_hides_the_argument_order` and `the_two_comparator_families_take_their_arguments_in_opposite_orders` |
| the underlying property | `bounds_agree_with_a_linear_scan_exhaustively` — every non-decreasing array of length 0..=8 over `{0,1,2}` crossed with every needle in `-1..=3`, all six bound/search functions against a linear scan. 3,280 arrays; upstream checks two. |

The exhaustive test was also run *against upstream on Node* before being written here, with zero
mismatches — so it pins agreement with the original, not merely internal consistency.

## Bugs this found

**B-95 — `lowerBoundIndices` defaults its upper bound from the wrong array.** `hi` falls back to
`array.length` where every other reference in the function is to `indices`. When `indices` is shorter
than `array` — the normal case for a partial argsort, which is exactly what an index array is for —
the search runs past the end of `indices`, reads `undefined`, indexes `array[undefined]`, gets
`undefined` again, fails the `value <= …` test, and moves right. The returned value is then a
position in neither array.

```js
> require('mnemonist/utils/binary-search').lowerBoundIndices([0,1,2,3,4,5,6,7], [0,1], 1)
8                                  // len(indices) is 2
> require('mnemonist/utils/binary-search').lowerBoundIndices([0,1,2,3,4,5,6,7], [0,1], 1, 0, 2)
1                                  // the answer the caller wanted
```

The library's one caller, `vp-tree.js`, always passes an `indices` array the same length as `array`,
so the defect is latent there. Verified on Node 24.18.1. Reproduced.

**B-96 — an out-of-range `hi` makes `search` report a match at a hole.** Not a crash and not a
`-1`: `undefined` loses both comparisons, so the `else` branch — which means "equal" — is taken.
`search([1,2,3], 9, 0, 100)` is `49`. The two bound functions react in *opposite* directions to the
same `undefined` (`lowerBound` walks right to `100`, `upperBound` walks left to `3`), which is worth
stating because it means there is no single "undefined sorts high/low" rule to reason from. Verified
on Node 24.18.1. Reproduced.

Neither is reachable from the shipped library today; both are reachable from the public API, which
`utils/` is (it is `require`-able from the published package and `_utils.js` treats it as such).

## Deliberate divergences

**D-40 — comparators return `Ordering`, not a number.** Upstream branches on `comparison > 0`,
`comparison < 0`, else. Those three arms are `Greater`, `Less`, `Equal`, so the mapping is exact for
every comparator that returns a real number. It is *not* exact for one that returns `NaN`: in
JavaScript that fails both tests and lands in the "equal" arm, and `Ordering` has no way to say so.
No comparator in mnemonist returns `NaN`; `utils/comparators.js` returns `-1`, `0`, `1`.

**D-41 — `search` returns `isize` with `-1` for "absent"**, not `Option<usize>`. `Option` would be
the idiomatic choice, but every upstream caller tests `!== -1`, and a port of those callers reads
more obviously against the same sentinel than against a `None`. The information content is identical.

**D-42 — `(lo + hi) >>> 1` is computed as `(lo + hi) / 2`.** The `>>> 1` also truncates the sum to
32 bits. That can only matter for an array of 2^31 or more elements, which JavaScript cannot
construct, so the truncation is unreachable and is not reproduced.

**D-43 — out-of-range reads are modelled, not bounds-checked.** `array[i]` past the end is `None`,
and every comparison against `None` is `false`. That is what JavaScript does. A `debug_assert` or a
panic here would be *more correct than upstream*, which DESIGN.md's porting rule explicitly calls a
bug in the port.

## Fuzz + bench

**Neither applies, and that is a scope statement rather than an omission.**

Gate 9 (differential fuzz) compares a *structure* op-by-op against a live upstream instance through
`fuzz/oracle.js`, which constructs `new Ctor(...)`. These are free functions with no instance and no
observable state, so there is nothing for the oracle protocol to hold. The equivalent evidence is
`bounds_agree_with_a_linear_scan_exhaustively`, whose 3,280-array cross-product was run against
upstream on Node before being frozen into the native test — an exhaustive differential check over
the domain upstream's own suite samples twice.

Gate 10 (benchmark) is keyed per unit in `bench/results.json` and will be recorded against the
`_utils` unit when `merge` and `iterables` land and that unit becomes real.

Gate 6 (falsification) likewise has no original test file to turn red, so it was performed through
the native suite. Two sabotages, each with its target assertion named **before** the run:

| sabotage | must break | result |
|---|---|---|
| `at()` → plain `&array[index as usize]` (delete the `undefined` model) | `an_over_long_hi_reports_a_hit_at_a_hole` | **red** — `index out of bounds: the len is 3 but the index is 49`, and every other test still passed, so the sabotage is precisely targeted |
| `search`'s `else` arm → `return -1` (delete the "neither greater nor less means equal" rule) | `nan_is_reported_as_a_match_by_search` | **red** — that test plus five others, including `search_matches_the_upstream_suites_own_case` |

Both reverted; 15/15 green afterwards. The first is the interesting one: it fails *only* the
assertion it was aimed at, which is what distinguishes a real falsification from a sabotage so broad
that any test would have caught it.

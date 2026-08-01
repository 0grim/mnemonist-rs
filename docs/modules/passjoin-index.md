# passjoin-index

Upstream: `passjoin-index.js` (519 LOC), no require-closure beyond `obliterator/iterator` and
`obliterator/foreach` (runtime dependencies in JS; `mnemonist_core` needs no equivalent) ·
`test/passjoin-index.js` — **270 lines, 12 `it` blocks** — also requires the `leven` npm package as
the distance function every add/search example uses.

Port: `crates/mnemonist-core/src/structures/passjoin_index.rs` — `PassjoinIndex`, a
Levenshtein-distance similarity index built on the "passjoin" partition scheme (Jiang et al. 2013;
Li, Deng & Feng 2013): every string is split into `k + 1` disjoint segments, and a query is answered
by generating the substrings a matching string's segment could have shifted to (bounded by `k`) and
looking those up, rather than scanning the whole corpus. Bridge:
`crates/mnemonist-napi/src/passjoin_index.rs`. Shim: `tests/bridge/passjoin-index.js`. Fuzz spec:
`crates/difffuzz/src/modules/passjoin_index.rs`.

---

## What upstream tests

* **`PassjoinIndex.comparator`** — sorting by decreasing length then lexicographically (the "4.2
  Effective Indexing Strategy" ordering from the paper).
* **`PassjoinIndex.segments`/`segmentPos`** — pinned literal examples for `k=3` over two strings of
  different lengths, checked segment-by-segment.
* **`PassjoinIndex.multiMatchAwareInterval`** — four pinned `(delta, i, pi, li) -> (start, stop)`
  examples.
* **`PassjoinIndex.multiMatchAwareSubstrings`** — seven groups of pinned examples across strings of
  length 7 through 13, plus a duplicate-letter case that must collapse consecutive identical
  substrings.
* **Constructor validation**: a `null` `levenshtein` (`/levenshtein/i`) and a negative `k`
  (`/number > 0/`) each throw — and specifically in that order, since `new PassjoinIndex(null)`
  omits `k` entirely and must still fail on the `levenshtein` message, not on `k`'s.
* **`add`/`search`** across three thresholds (`k=1,2,3`) over a shared ten-string pool, including
  the empty string as a valid added value.
* **A "should remain sane" regression** — apostrophes and a specific `failed`/`flailed` pair at
  `k=1`.
* **`PassjoinIndex.from`**, `forEach`, the `values()` iterator, `for...of` (`Symbol.iterator`), and
  `clear()`.

## What upstream does NOT test

**A `levenshtein` function that throws.** Every example uses the real `leven` package, which never
throws for two strings. This port's fallible path (`try_search`, the re-entrancy guard in the
bridge) is exercised by native tests and by construction, not by the original suite.

**An array-like of characters as an added/searched value.** Upstream's own string operations
(`.length`, `.slice`, `+`) work identically on a plain array of characters, so `add`/`search` accept
either in principle; `test/passjoin-index.js` only ever passes strings. See "Deliberate divergences"
(D-453).

**The partition/segment-generation arithmetic under a differential fuzz campaign spanning many
random `k`/length combinations** — the original suite pins a fixed set of literal examples at
`k=3`, which this port also reproduces exactly (see "What we test in addition"), but does not by
itself prove the arithmetic generalises. That generalisation is what the differential fuzzer (varying
`k` across `1..=3` against a controlled-edit-distance word pool) is for.

## What we test in addition

**Rust native tests** — `crates/mnemonist-core/src/structures/passjoin_index.rs` (12):

| Test | Closes gap |
|---|---|
| `comparator_sorts_by_decreasing_length_then_lexicographically`, `segments_matches_upstreams_pinned_examples`, `segment_pos_matches_upstreams_pinned_examples`, `multi_match_aware_interval_matches_upstreams_pinned_examples`, `multi_match_aware_substrings_matches_upstreams_pinned_groups` | the upstream blocks, transcribed against the exact same pinned literal examples |
| `constructor_rejects_invalid_k`, `reproduces_the_upstream_add_and_search_walkthrough`, `reproduces_the_upstream_sanity_walkthrough`, `for_each_and_values_walk_in_insertion_order`, `clear_resets_the_index` | the remaining upstream blocks, as a baseline |
| `search_results_are_in_upstreams_own_insertion_order` | `search`'s result *order*, not just its membership — see "Bugs this found" for why this is load-bearing beyond what `assert.deepStrictEqual` on two `Set`s can ever catch |
| `try_search_propagates_a_failing_distance_function` | the untested throwing-`levenshtein` path above |

**Differential fuzzer** — a controlled-edit-distance word pool, `k` varied across `1..=3` (the same
range `test/passjoin-index.js` itself uses), the real `leven` npm package as the oracle-side distance
function via `fuzz/oracle.js`'s `pjLeven` factory; see "Fuzz + bench" for measured numbers, including
a `grammar_self_check` counting non-empty searches and exact-`k`-distance hits directly.

**Still untested, stated rather than glossed:** the array-like-of-characters input form (D-453), and
astral-character input (D-452).

## Bugs this found

No upstream defect found in this unit. In particular, the partitioning arithmetic — the place
CLAUDE.md's guidance for this unit specifically calls out as "the failure mode least likely to be
noticed" if a correct implementation were made to look wrong by an off-by-one — was checked directly
against `test/passjoin-index.js`'s own pinned segment/position/interval/substring examples (all
passing, see "What we test in addition") and exercised over ~1.77M differential-fuzz operations
against real upstream on Node 24.18.1 with zero divergences. No case was found where upstream's own
partition scheme misses a candidate a correct implementation would find.

Two defects were found in **this port's own** code before it was committed as complete, neither an
upstream bug:

1. `search`'s match accumulator was a `HashSet<String>`, correct for membership but with no
   relationship to insertion order. `assert.deepStrictEqual` on two JS `Set`s is order-independent,
   so `test/passjoin-index.js` could not have caught this, but the differential fuzzer's plain
   JSON-array comparison is order-sensitive: a `HashSet`-backed result would have reported false
   divergences purely from Rust's hash-bucket iteration order disagreeing with upstream's genuine
   first-insertion order. Caught before any campaign was logged (during construction of the fuzz
   spec, by reasoning about what the comparison actually does — not by a failing run) and fixed with
   `OrderedStringSet`, verified against real upstream for a concrete query
   (`search_results_are_in_upstreams_own_insertion_order`).
2. The bridge constructor checked `k`'s validity before `levenshtein`'s type, the opposite of
   upstream's own order. `new PassjoinIndex(null)` therefore failed with napi's own "missing
   argument" message instead of the upstream-matching `/levenshtein/i` one — caught by
   `test/passjoin-index.js`'s own "should throw if given wrong arguments" block (gate 4), not by
   fuzzing. Fixed by making `k` optional at the bridge and checking `levenshtein` first.

See `planning/NOTES.md`'s "multi-array, symspell, passjoin-index" entry for the fuller account of
the sibling units' own defects found the same way.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-452 | **String indexing (`segments`/`segmentPos`/`multiMatchAwareSubstrings`) is over Rust `char`s (Unicode scalar values), not upstream's UTF-16 code units.** | Agrees exactly for every Basic-Multilingual-Plane codepoint, the only kind `test/passjoin-index.js` and this port's fuzz grammar use; diverges only for astral characters, which neither exercises. |
| D-453 | **`add`/`search` accept only a JS `String`, not upstream's array-like-of-characters form.** | Untested upstream; accepting the array form would mean threading a second representation through every core helper for a case nothing observes. |
| D-454 | **The inverted index keys on a `(String, i64)` tuple, not upstream's string concatenation `segment + i`.** | A strictly safer key (upstream's concatenation could in principle collide between two distinct `(segment, i)` pairs; the tuple cannot) that is unreached on every tested and fuzzed input either way, so it can only ever make the port's candidate set a superset on an input nobody has observed, never a subset. Recorded because this is the one unit where "more correct than upstream" is the stated risk to watch for. |

## Fuzz + bench

### Fuzz

```
module=passjoin-index  seed=42        cases=8870 ops=888923 wall=60.0s divergences=0
module=passjoin-index  seed=20260802  cases=8857 ops=880275 wall=60.0s divergences=0
```

Two campaigns, two seeds, **1.77M operations, zero divergences**. Reproduce with e.g.
`target/release/difffuzz --module passjoin-index --seed 42 --cases 8870`.

* **Op alphabet:** `add` (weight 5) and `search` (weight 5) equally weighted; `clear` (weight 1).
* **Word pool:** fifteen words at controlled edit distances (`"benjamin"/"benjomon"/"benja"/"benjo"`,
  `"paule"/"paul"/"pa"/"pat"`, `"ab"/"a"/"b"/""`, `"failed"/"flailed"/"railed"`) — the
  controlled-distance construction CLAUDE.md's guidance requires, not random strings that would all
  be far apart and return empty candidate sets trivially.
* **Constructor:** `k` uniform over `1..=3`, the exact range `test/passjoin-index.js` itself uses
  (`k1`/`k2`/`k3`); `levenshtein` is `fuzz/oracle.js`'s `pjLeven` factory, the real `leven` package —
  not a simplified stand-in, since `test/passjoin-index.js` itself uses exactly this function.
* **Observable state:** `size`, `k`. `search`'s own return value (rendered `{"$set": [...]}`, in
  upstream's genuine insertion order — see "Bugs this found") is compared per-op.

**Direct evidence the grammar reaches the states this campaign is for** — `grammar_self_check`
(`crates/difffuzz/src/modules/passjoin_index.rs`, no oracle, no `node`), 400 generated programs, up
to 200 ops each:

```
passjoin-index grammar: 9778/19208 searches non-empty, 5161 pulled in a candidate at exactly distance k
```

51% of searches return at least one candidate, and 5,161 pull in a candidate at exactly the `k`
threshold — both floors (`> 100` non-empty, `> 30` boundary hits) are asserted in the test itself.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:**
`crates/mnemonist-core/src/structures/passjoin_index.rs`'s
`multi_match_aware_interval_matches_upstreams_pinned_examples` test, specifically the pinned example
`multi_match_aware_interval(3, delta=1, i=1, s=10, pi=2, li=2) == (1, 3)` — transcribed directly from
`test/passjoin-index.js`'s own "should be possible to compute the multi-match aware interval." block.

**The sabotage:** `multi_match_aware_interval`'s `let o = k - i;` was changed to
`let o = k - i - 1;` — an off-by-one in the candidate-generation window's width, the exact class of
defect CLAUDE.md's guidance for this unit calls out as producing "a subtly smaller candidate set
that still contains most correct answers."

**Confirmed red:** the named assertion failed (`(2, 3)` instead of `(1, 3)`), six of twelve native
tests failed downstream (including `add`/`search` walkthroughs losing real matches — `{"paul"}`
instead of `{"paul", "paule"}`), and the real upstream mocha suite failed the identical assertion at
`test/passjoin-index.js:136` plus the same shrunk-candidate-set failures in `add`/`search`.

**Reverted; confirmed green again:** 12/12 native tests (13 after this session's additions),
13/13 upstream `it` blocks.

**Nothing was found to be blind.** The sabotage produced exactly the failure mode CLAUDE.md warns
is hardest to notice — a smaller, still-plausible candidate set — and both instruments caught it
immediately rather than needing a wider probe.

### Bench

**Not run.** Gate 10 is deferred to a serial pass on an idle machine (DESIGN.md §7.3).
`passjoin-index` is therefore complete except gate 10, and deliberately not in `tests/scope.txt`.

# symspell

Upstream: `symspell.js` (548 LOC), no require-closure beyond `obliterator/foreach` (a runtime
dependency in JS, ported code here — `mnemonist_core` needs no equivalent, since `add`/`from` just
call a plain function on an already-materialised iterable) · `test/symspell.js` — **131 lines, 6
`it` blocks** — also requires the `damerau-levenshtein` npm package, but only to *validate* results
against a second implementation, never to compute anything the port itself needs to reproduce.

Port: `crates/mnemonist-core/src/structures/symspell.rs` — `SymSpell`, a Symmetric Delete spelling
index: every added word's reachable one-to-`maxDistance` character deletions are indexed, so a
query only has to generate its own deletes and look them up directly. Bridge:
`crates/mnemonist-napi/src/symspell.rs`. Shim: `tests/bridge/symspell.js`. Fuzz spec:
`crates/difffuzz/src/modules/symspell.rs`.

---

## What upstream tests

* **Constructor validation**: a negative `maxDistance` and an out-of-range `verbosity` (`45`) each
  throw, matching `/maxDistance/` and `/verbosity/`.
* **Basic indexing and search**: ten words added (with one repeat, `'Hello'` twice, checking `size`
  counts every `add` call including the repeat), then `search('ello')` against a **specific,
  ordered** five-suggestion list — `deepStrictEqual` against the whole array, so both membership and
  order are pinned. Every returned `distance` is also cross-checked against the real
  `damerau-levenshtein` npm package.
* **A wider `maxDistance` (4, not the default 2)** finds more, and different, suggestions —
  the exact input needed to exercise the far branches at all.
* **All three `verbosity` levels** (`0`, `1`, `2`) against the same data, checking that a lower
  verbosity returns a strict prefix of a higher one's suggestions for the same query.
* **`clear()`** resets `size` to `0`.
* **`SymSpell.from(iterable)`**, checked by running the same `search('ello')` query afterward.

## What upstream does NOT test

**A word that is both a real dictionary entry and another word's delete-form at the same time**
via a single dictionary key serving both roles simultaneously — `'Hell'` appears in `DATA` and is
*also* `'Hello'`'s length-1 delete, but the test file never isolates that interaction from the rest
of `DATA`. This port's own test
(`a_word_can_be_both_a_real_entry_and_another_words_delete_form`) isolates it directly.

**A dictionary entry still in its "compact" form (a bare word index) at the moment a query reaches
it** — every query in `test/symspell.js` happens to run against a dictionary where the relevant
entries have already been promoted to full objects by cross-references among `DATA`'s own ten
words. See "Bugs this found": this is exactly the gap that let a real defect in this port ship
undetected by the original suite.

**A fractional `maxDistance`** (e.g. `2.5`). Upstream's guard is `typeof maxDistance !== 'number' ||
maxDistance <= 0` — no integrality check despite the error message's wording — so a fractional value
is accepted and used exactly as given in every downstream comparison; untested either way.

**A `NaN` `maxDistance`.** `NaN <= 0` is `false` in both languages, so upstream's own guard lets it
through uncaught, and it then propagates through every later comparison as `false` (matching IEEE
754 in Rust too, verified for the one place this port casts a distance to compare against it — see
`SymSpell::new`'s docs). This is fidelity restored during development, not a disclosed divergence:
see "Bugs this found" for the direction this was initially wrong in.

## What we test in addition

`crates/mnemonist-core/src/structures/symspell.rs` — 8 tests: a baseline reproduction of the
upstream blocks transcribed with the exact same `DATA` and expected ordered suggestion lists, the
untested dual-role dictionary key case, the real defect this port shipped and the differential
fuzzer caught (a compact dictionary entry still contributing its suggestion — see "Bugs this
found"), and the internal Damerau-Levenshtein distance function checked directly against known
distances, independent of the indexing machinery around it.

**Differential fuzzer** — a controlled-edit-distance word pool (every word within
Damerau-Levenshtein distance 1-2 of at least one other, plus one deliberately distant word), varying
`maxDistance` across `1..=4` and `verbosity` across all three values; see "Fuzz + bench" for measured
numbers, including a `grammar_self_check` counting non-empty searches and exact-threshold hits
directly.

**Still untested, stated rather than glossed:** a fuzzed pool wide enough to hit every verbosity-2
early-termination path (the current pool is tuned for controlled distances, not exhaustive branch
coverage), and astral-character input (see "Deliberate divergences").

## Bugs this found

No upstream defect found in this unit.

**One real defect in this port's own `lookup`**, found by the differential fuzzer's very first short
smoke run (before any full campaign was logged) and fixed before this unit was committed as
complete: `add("jello")` then `search("hello")` at `{maxDistance: 1, verbosity: 0}` returned `[]`
from the port against `[{"term":"jello","distance":1,"count":1}]` from real upstream (verified
directly against `symspell.js` on Node 24.18.1, not merely inferred from the fuzz comparison).
Cause: this port's dictionary entry, mirroring upstream's own two-shaped representation (a bare
word index versus a promoted `{suggestions, count}` object), skipped a "compact" entry's
suggestions entirely rather than reproducing upstream's *local, non-persisted* promotion of it
during a read (`item = createDictionaryItem(item)` in `lookup`, which upstream never writes back to
the dictionary). Because most dictionary entries in `test/symspell.js`'s own ten-word `DATA` set get
cross-promoted to full objects by the data's own overlapping delete-forms, the original suite never
reached an unpromoted entry at query time — the gap in "What upstream does NOT test," directly.
Fixed by `Entry::suggestions()`, which reproduces the same local promotion for both entry shapes.
See `symspell.rs`'s `a_compact_dictionary_entry_still_contributes_its_suggestion`.

A second, smaller defect caught during a `cargo clippy` pass rather than by fuzzing: an earlier draft
rejected a `NaN` `maxDistance` at construction, the opposite of upstream's own (surprising) behaviour
— see "What upstream does NOT test." Fixed to match upstream rather than the more-defensive reading.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| DIV-SYMSPELL-1 | **String indexing throughout (`edits`, `addLowestDistance`, `damerauLevenshtein`, `lookup`) is over Rust `char`s (Unicode scalar values), not upstream's UTF-16 code units.** | The two agree exactly for every codepoint in the Basic Multilingual Plane, which includes every alphabet `test/symspell.js` and this port's fuzz grammar use, and diverge only for astral characters (outside the BMP) — not exercised by either. |

## Fuzz + bench

### Fuzz

Two campaigns, two seeds, **1.56M operations, zero divergences** — the campaigns logged are the
clean, post-fix runs; see "Bugs this found" for what the very first short smoke run (before these)
caught:

```
module=symspell  seed=42        cases=7942 ops=791368 wall=60.0s divergences=0
module=symspell  seed=20260802  cases=7693 ops=771891 wall=60.0s divergences=0
```

Reproduce with e.g. `target/release/difffuzz --module symspell --seed 42 --cases 7942`.

The op alphabet weights `add` and `search` equally, since both sides of the symmetric-delete scheme
need exercising; `clear` is much lighter. The word pool is fourteen words, chosen so every one sits
at Damerau-Levenshtein distance 1-2 of at least one other, plus one deliberately distant word so an
empty-result search is reachable too. The constructor draws `maxDistance` uniform over `1..=4` (the
same range `test/symspell.js` itself uses) and `verbosity` uniform over `0..=2`. Observable state is
`size`, `maxDistance`, `verbosity`; `search`'s own return value (the ordered suggestion list) is
compared per-op, not just folded into state. Full grammar: evidence file.

**Direct evidence the grammar reaches the states this campaign is for** — `grammar_self_check`, 400
generated programs, up to 200 ops each: 7,290 of 18,531 searches non-empty (39%), 1,438 pulling in a
suggestion at exactly the threshold boundary. Both floors (`> 100` non-empty, `> 30` boundary hits)
are asserted in the test itself, so a future pool change that regresses this back toward "every
query is too far from every word" fails loudly.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:** `test/symspell.js`'s "should correctly
index & perform basic search queries." — `assert.deepStrictEqual(index.search('ello'), [...5
specific suggestions...])`.

**The sabotage:** `edits`'s deletion loop, `for i in 0..length`, was changed to `for i in
1..length` — skipping the deletion that removes a word's *first* character, the exact delete-form
(`"Hello"` → `"ello"`) the whole basic-search example depends on.

**Confirmed red:** the named assertion failed — `search('ello')` returned `[]` instead of the
five expected suggestions — four of eight native tests failed, and the real upstream mocha suite
failed the same two assertions (basic search and `SymSpell.from`) with the identical empty result.

**Reverted; confirmed green again:** 8/8 native tests, 6/6 upstream `it` blocks.

**Nothing was found to be blind.** The sabotage broke exactly the indexing path it targeted, and
every downstream query that depended on a first-character delete failed identically in both
instruments.

### Bench

`bench/results.json` → `modules["symspell"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 2,000 samples/side.

**`mixed-4e3`** — 200,000 mixed `add`/`search` (50/50), `maxDistance` 2, `verbosity` 2 (upstream's
own defaults), dictionary prefilled to a stated 50% fill ratio before timing. A random vocabulary
defeats this structure entirely: `search` only finds anything by matching deletes of the query
against deletes of added words, and if nothing is within `maxDistance`, every search returns empty.
The vocabulary generator applies a multiplicative scramble (`Math.imul`-matched) before encoding the
domain value in a fixed base, which spreads the domain across the suffix space so only the
deliberate one-character query perturbation makes a query findable — see the log for the two
designs tried and rejected before this one. Measured: **98.4% of `search` calls return at least one
suggestion**, averaging 1.40 suggestions per call — genuine, non-degenerate hits. xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **4434.60** | 5901.76 | 1.3× faster |
| p99 ns/op | **5972.41** | 8682.54 | 1.5× faster |
| RSS delta MB | **31.1** | 96.9 | |
| structure-only RSS delta MB | 0.1 | **0.4** | |
| startup ms | **0.6** | 15.4 | 26× (reported separately; not throughput) |

**No regressions.** Checksum `471173`, identical on both sides — both sides generated the same
deletes, found the same candidates, and computed the same edit distances.

# suffix-array

Upstream: `suffix-array.js` · 353 LOC · `test/suffix-array.js` (113 lines, 20 assertions, one
`it.skip`)

Require-closure: `suffix-array.js` alone. **Zero dependencies** — no `obliterator`, no `utils/`.
So the unit is one file, and it is the smallest closure in the queue.

Port: `crates/mnemonist-core/src/structures/suffix_array.rs` ·
bridge `crates/mnemonist-napi/src/suffix_array.rs` · shim `tests/bridge/suffix-array.js`

Gates 1–9 green. **Gate 10 (benchmark) deliberately not run**: five agents were working this tree,
and DESIGN.md §7.3 batches benchmarks into a quiet serial pass because a contended run inflated both
sides 2–3× here. `tests/scope.txt` is therefore untouched, and `tests/verify.sh` correctly reports
this unit as not-in-scope. That is the intended end state, not an omission.

---

## What upstream tests

Five `it()`s and one `it.skip`:

* **`SuffixArray` should produce the correct array** — `'banana'` (length 6) and
  `'This is a long string.'` (length 22), each against a hardcoded array.
* **…with arbitrary sequences** — `'banana'.split('')`, i.e. the same six characters as an array of
  one-character strings, against the same hardcoded array.
* **`GeneralizedSuffixArray` should produce the correct array** — `['banana', 'ananas']` and the
  same pair split into characters.
* **…longest common subsequence** — three cases: `['banana', 'ananas'] → 'anana'`,
  `['abcd', 'cdef'] → 'cd'`, and one word-token pair → `['the', 'mouse']`.
* **`it.skip('should work with int values (issue #196)')`** — the only acknowledgement anywhere in
  the repo that this module has a known defect, and it is disabled.

So: **four distinct inputs, all ASCII, all lowercase-Latin except one sentence, none longer than 22
characters, and every expectation a hardcoded constant.** Not one assertion compares the result
against an independently computed suffix array.

## What upstream does NOT test

1. **That the answer is *correct*.** Every expected array is a frozen copy of what the code
   produced. If the algorithm is wrong on an input, the test records the wrong answer as the
   expectation. This is not hypothetical — see "Bugs this found", where two of the four tested
   inputs happen to sit off the failure modes by luck rather than by design.
2. **Any character above U+007F.** The whole alphabet exercised is ASCII, so the entire high half of
   `charCodeAt`'s range is untested — which is exactly where B-90 lives.
3. **Lengths.** Four inputs, of lengths 6, 22, 13 and 13. Two of the three residues of `l % 3` are
   represented (0 and 1); the one input at the residue that fails (22) survives only because its
   triples are all distinct and the recursion never fires.
4. **The empty sequence, and length 1.** Neither is built. Length 0 is the input that drives `build`
   through `al == 0`, where `a[0]` is `undefined` and upstream's `(undefined / 3) | 0` silently
   becomes `0`; length 1 is the recursion's base case.
5. **`toString` and `toJSON`.** Defined upstream, called by nothing.
6. **`hasArbitrarySequence`,** which is a public property and is never read.
7. **`GeneralizedSuffixArray` with one member, or with more than two.** Only pairs are built.
8. **Disjoint members** — an LCS that is genuinely empty is never asserted.
9. **`firstLength` and `text`,** both public, both unread.
10. **Tokens whose string order differs from any natural order** — the token cases use words whose
    lexicographic order is the intended one, so the fact that the alphabet is built from
    `Object.keys(...).sort()` (i.e. *string* order, so `"10" < "2"`) is never visible.
11. **Anything about the separator.** `''` is spliced between members and occupies a real
    position; nothing checks that, and nothing checks what happens when a member contains one.

## What we test in addition

18 native tests in `crates/mnemonist-core/src/structures/suffix_array.rs`.

| gap | test |
|---|---|
| — (upstream's own five, transcribed) | `matches_the_upstream_suites_own_arrays`, `..._own_arbitrary_sequence`, `..._own_generalized_arrays`, `..._own_lcs` |
| 1, 2, 3 | `b90_symbols_sharing_a_low_byte_are_mis_sorted`, `b91_lengths_congruent_to_one_mod_three_are_mis_sorted` — both assert upstream's **wrong** answer *and* assert it differs from a naive reference, so a later "tidy-up" fails loudly |
| 1, 3 | `ascii_inputs_off_the_bad_residue_are_exactly_right` — every binary string of length 1..=14 whose length is not `1 (mod 3)`, checked against a naive `O(n² log n)` suffix sort. 23,404 inputs. |
| 3 | `the_bad_residue_is_correct_until_the_recursion_fires` |
| 4 | `empty_sequences`, `the_shortest_sequences` |
| 5 | `to_string_joins_the_array_with_commas` |
| 7 | `a_generalized_array_of_one`, `a_generalized_array_of_three` |
| 8 | `disjoint_members_have_no_common_substring` |
| 9 | `a_generalized_array_of_one` (asserts `firstLength`), `the_separator_occupies_a_position` (asserts `text`) |
| 10 | `the_token_alphabet_is_ordered_as_strings` |
| 11 | `the_separator_occupies_a_position` |
| the port's own edges | `a_generalized_array_of_none_is_refused`, `a_mixed_generalized_array_is_refused` |

The naive reference is the load-bearing piece. It is what turns "matches the frozen constant" into
"matches an independently computed answer", and it is what found both defects below.

## Bugs this found

Both were found by running **upstream** — not the port — against the naive reference on Node
24.18.1, over random inputs. Both are reachable from the documented public API. Both are reproduced.

### B-90 — the radix sort silently narrows to 8 bits

`sort()` picks its radix width by scanning for the largest symbol:

```js
while (i--)
  j = Math.max(string[array[i] + offset], j);

bits = j >> 24 && 32 || j >> 16 && 24 || j >> 8 && 16 || 8;
```

The scan reads `string[array[i] + offset]`. `convert()` pads the sequence with `length % 3` zeros,
which is **not enough**: for `offset` 1 and 2 the index routinely runs one or two past the end. In
JavaScript that read is `undefined`, `Math.max(undefined, j)` is `NaN`, every shift of `NaN` is `0`,
every `&&` clause is therefore falsy, and `bits` falls through to `8`. The sort then compares only
the **low byte** of each 16-bit symbol.

Mechanism confirmed by instrumenting upstream's own `sort`, not inferred. For the 15-character input
below, three passes run:

| offset | read past the end? | `j` | `bits` |
|---|---|---|---|
| 2 | yes (index 16, length 15) | `NaN` | 8 |
| 1 | yes | `NaN` | 8 |
| 0 | no | 513 | 16 |

So two of the three passes are 8-bit while the largest symbol needs 10.

Any alphabet where two symbols share a low byte is then mis-sorted — including every character at or
above U+0100 whose low byte collides with the `0` padding:

```js
> new (require('mnemonist/suffix-array'))('ĀĀĀĀȁĀĀȁȁȁȁȁĀȁȁ').array
[ 0, 1, 2, 5, 3, 12, 6,  4, 14, 11, 10, 13, 9, 8, 7 ]
//                       ^^^^^^      ^^^^^^
// correct:
[ 0, 1, 2, 5, 3, 12, 6, 14,  4, 11, 13, 10, 9, 8, 7 ]
```

Two transpositions, not a scrambling — which is what makes it dangerous. A caller spot-checking the
first few entries sees a plausible answer.

Measured incidence over 10,000 random inputs of length 1..30 with a two-symbol alphabet whose
members share a low byte (`'A'` / `'Ł'`): **81% wrong** at `length % 3 == 0`. Pure-ASCII alphabets
are unaffected, because ASCII fits in the low byte and the padding `0` is not a letter.

### B-91 — the reduced string has no separator when `l % 3 == 1`

DC3 splits positions into three classes, ranks the ≡1 and ≡2 groups, concatenates those two rank
arrays into a reduced string, and recurses. The concatenation is only sound if the first group ends
in a symbol that nothing else can equal — otherwise a suffix of the reduced string can run out of
the first group and into the second, comparing positions that are not adjacent in the original.

Upstream sizes the groups with `al = (2 * l / 3) | 0`. For `l % 3 == 1` that omits the ≡1 position
that would have carried the sentinel. The two halves then run together, and once the recursion
actually fires — which needs a repeated triple — the answer is wrong:

```js
> new (require('mnemonist/suffix-array'))('aaaaaaa').array
[ 6, 5, 3, 0, 2, 4, 1 ]        // correct: [ 6, 5, 4, 3, 2, 1, 0 ]
```

Exhaustively over binary strings, failures occur at lengths **7, 10, 13, 16** and at no other length
up to 16 — all ≡ 1 (mod 3), and 4 is clean because it is too short for the recursion. The rule
applies at every recursion level, which is why an occasional length ≡ 2 (mod 3) also fails: its
`al` is itself ≡ 1 (mod 3). Measured incidence over 10,000 random 3-letter inputs of length 1..30:
**12% wrong** at `length % 3 == 1`, 0% elsewhere.

Upstream's own suite contains a length-22 input, which *is* ≡ 1 (mod 3), and passes — because
`'This is a long string.'` has no repeated triple, so `j == al` and the recursion never runs. The
test suite is one repeated trigram away from having caught this.

### Relationship to upstream's own `it.skip`

`test/suffix-array.js` ends with a skipped test for issue #196 and the comment
`// TODO: fix sentinel to be lower than anything else in the token case`. That is a *third*,
separately-known problem — the token alphabet numbers symbols from 1 while the separator `''`
is itself a token and gets an ordinary alphabet number rather than a guaranteed-smallest one. It is
disabled, so it is not asserted here either way; the port reproduces whatever upstream does, and the
skipped test stays skipped (`tests/run.sh` reports "1 pending").

Neither B-90 nor B-91 is the same defect: both fire on plain string input where issue #196 needs
tokens.

## Deliberate divergences

**D-48 — tokens are `String`, and their identity is their string form.** Upstream builds the token
alphabet by using each token as a *property key*, i.e. through `ToString`, but compares tokens inside
`longestCommonSubsequence` with `!==`, i.e. by identity. For two distinct objects with the same
`toString` those disagree: same alphabet symbol, different `!==`. Representing a token as a `String`
collapses the two. Every token in upstream's suite, and every token any sane caller passes, is
already a string or a number, where the two coincide. The bridge coerces with `String(x)`, which is
exactly what the alphabet does.

**D-49 — a mixed member list is refused.** `new GeneralizedSuffixArray(['ab', ['c', 'd']])` upstream
takes `strings[0]`'s kind and applies it to everything, so the array member is `join`ed into
`"c,d"`; the reverse case spreads a string into its characters via `push.apply`. Neither is a
behaviour any caller wants and neither is documented. The port returns an error. Stated rather than
silently supported, because "upstream would have done something" is not the same as "upstream
defined something".

**D-50 — an empty member list is an error, not a `TypeError` from a property read.** Upstream reads
`strings[0].length` unguarded. The core returns `Err`; the bridge surfaces it as a JS error. A panic
would cross the FFI boundary and take the host process down.

**D-51 — `SuffixArray.GeneralizedSuffixArray` is aliased in the shim, not in the addon.** Upstream's
last-but-one line is `SuffixArray.GeneralizedSuffixArray = GeneralizedSuffixArray`. That is
CommonJS *namespacing*, not behaviour: the addon exports both classes at top level, so
`require('@port/addon').GeneralizedSuffixArray` works and nothing is missing from the addon. Compare
`Stack.of`, which *is* behaviour and therefore lives in the addon (`crates/mnemonist-napi/src/statics.rs`)
precisely so a shim is not load-bearing.

The alternative was to do the alias inside the addon's single `#[napi(module_exports)]` hook in
`crates/mnemonist-napi/src/cursor.rs`. That file is being edited concurrently by several agents, and
a merge conflict boundary landing inside a function tail has already broken this tree three times.
Recording the trade here rather than burying it: this is a merge-safety decision, and if the hook
ever becomes safe to extend, moving the alias into it would be strictly better.

**D-52 — `inspect` is not ported.** A Node display convenience (`util.inspect.custom`) with no
upstream assertion and no Rust equivalent.

**D-53 — the core reads every sequence through a `Sparse`, not a slice.** Both defects above are
consequences of reading past the end of an array, and B-90 turns on the difference between reading a
*zero* and reading *`undefined`* — the first still sorts correctly, the second poisons `Math.max`.
So a port that indexed with `[]` would panic where upstream computes an answer, and one that clamped
to `0` would be quietly more correct than the library it ports. `compare()` returns `f64` for the
same reason: upstream's `||` chain treats `NaN` as falsy and its caller tests `< 0`, which `NaN`
fails.

## Fuzz + bench

**Gate 9 — four campaigns, 1,735,060 operations, zero divergences.**

| module key | seed | cases | ops | wall |
|---|---|---|---|---|
| `suffix-array` | 42 | 274,418 | 548,743 | 60.0s |
| `suffix-array` | 20260801 | 257,468 | 515,381 | 60.0s |
| `generalized-suffix-array` | 42 | 169,107 | 343,616 | 60.0s |
| `generalized-suffix-array` | 20260801 | 161,214 | 327,320 | 60.0s |

Spec: `crates/difffuzz/src/modules/suffix_array.rs`. This is the first module in the fuzzer with no
mutating method, so the constructor strategy carries the entropy and programs are 1..4 ops long.
The op alphabet is **complete** for both classes — `toString`, `toJSON` and, for the generalized
variant, `longestCommonSubsequence` — so no method is omitted from the grammar.

The input alphabet is picked for collisions rather than realism: `U+0000` equals `convert`'s padding
value, `U+0001` equals the generalized separator, and `U+0100` / `U+0141` / `U+0201` have low bytes
`0x00` / `0x41` / `0x01`, colliding respectively with the padding, with `'A'` and with the separator
under B-90's 8-bit radix. Lengths run to 45, covering all three residues of `l % 3`, several
recursion depths, and the point where a reduced string's ranks exceed 255.

Deliberately excluded, per DESIGN.md §3.7: an **empty** member list (upstream throws from the
constructor, which the oracle protocol classifies as apparatus failure, and the port refuses it —
see D-50) and **mixed** member kinds (D-49). Both are documented divergences, so fuzzing them would
only re-report a known decision.

**Gate 6 — falsification, three separate runs, each with its target named before it was performed.**

| what | sabotage | must break | result |
|---|---|---|---|
| the original test suite (gate 4) | remove the `.rev()` from the radix gather in `sort()`, making the LSD sort unstable | `assert.deepStrictEqual(sa.array, [5, 3, 1, 0, 4, 2])` in `'SuffixArray should produce the correct array.'` | **red**: 0 passing, 5 failing. Reverted: 5 passing, 1 pending. |
| the `suffix-array` fuzz spec | `sort()`'s `bits` fall-through `8 → 16`, i.e. "fixing" B-90 | a state divergence in `array` / `toJSON` / `toString` | **red** after 400 cases, minimised to a 42-character input mixing U+0141 with U+0100. Reverted: clean. |
| the `generalized-suffix-array` fuzz spec | LCS's second guard `>` → `>=` | a divergence in `longestCommonSubsequence`'s return value | **red**, minimised all the way to a two-member list whose first member is empty — so `firstLength` is 0 and position 0 *is* the boundary the asymmetric guards let through. Reverted: clean. |

Both minimised seeds are committed under `crates/difffuzz/proptest-regressions/` with PROVENANCE
blocks, because an unlabelled `cc` line reads as "a real port defect was found here", which is the
opposite of what happened.

**Gate 10 — not run, on purpose.** See the header. Benchmarks belong to the quiet serial pass; a
number measured while five agents compile is worthless, and publishing one would be exactly the
"do not overclaim about performance" failure CLAUDE.md warns about.

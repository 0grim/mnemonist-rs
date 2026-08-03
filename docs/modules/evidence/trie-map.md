# trie-map — evidence

Gate artifacts for `docs/modules/trie-map.md`: test-to-gap table, fuzz grammar, full falsification
record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/trie_map.rs` (15 tests, plus 8 more in `trie.rs`):

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite`, `setting_the_same_prefix_again_does_not_increase_size`, `the_null_sequence_is_a_valid_prefix`, `update_calls_back_with_the_old_value_and_creates_when_absent`, `delete_removes_and_prunes_singleton_chains`, `find_returns_the_suffix_beyond_the_given_prefix`, `clear_resets_size_and_removes_everything` | the upstream blocks, as a baseline |
| `delete_does_not_prune_an_ancestor_that_is_itself_a_stored_word` | a sharper version of the file's own pruning check: the word one level up must survive |
| `has_distinguishes_a_stored_word_from_a_mere_prefix_of_one` | the gate 6 falsification target, pinned directly |
| `walk_over_a_prefix_that_does_not_exist_is_empty` | gap 1 |
| `walk_visits_every_word_in_the_same_order_as_find` | cross-checks the lazy walk against the eager `find` DFS, which upstream never does explicitly (they are two separate code paths upstream too) |
| `an_addition_inside_an_already_queued_branch_is_visible_to_an_open_walk` | the *matching* half of the DIV-TRIE-MAP-2 story: a live addition (not a prune) to a node an open walk has already queued IS seen, on both sides |
| `a_token_equal_to_the_sentinel_character_is_an_ordinary_token` | gap 3 / BUG-TRIE-MAP-1 — pins that the port does **not** reproduce the corruption |
| `root_exposes_entries_in_insertion_order` | the shared value/child enumeration order `NodeView` exposes, which is what makes `find`'s DFS order correct in the first place |
| `values_mut_reaches_every_stored_value` | bridge plumbing: every stored value must be reachable to release a JS reference on `clear`/finalize |

## Fuzz grammar

`crates/difffuzz/src/modules/trie_map.rs`. `PREFIX_POOL`: `a, ab, abc, abcd, b, ba, bc, bad`.
Measured, not asserted by eye: `pool_self_check_most_entries_are_a_prefix_of_another_entry`
confirms **5 of the 8 pool entries are themselves a strict prefix of another entry**
(`a`⊂`ab`⊂`abc`⊂`abcd`, `b`⊂`ba`⊂`bad`, `b`⊂`bc`); `pool_self_check_generated_programs_revisit_prefix_relationships`
draws 2,000 samples from the real `set` op strategy across both regimes and confirms the *actual
generated stream* — not just the pool in principle — mostly revisits these relationships. Both are
plain `cargo test` assertions, no oracle, no `node`.

**The regime split (DIV-TRIE-MAP-2).** `ctor_strategy` generates one internal flag (not a real `Token`
argument — see the module's own docs) deciding whether a program exercises `delete`/`clear` or a
persistent `$iter`/`$next` cursor, never both in the same program. This exists because the campaign
run *without* the split diverged inside a few hundred operations, independently rediscovering BUG-TRIE-MAP-2:

```
divergence in return value after op #4: $next()
  done:  port: true   upstream: false
  value: port: undefined   upstream: "a"
```

`$spread` (`Array.from`) is exempt from the split in both regimes: it opens and fully drains a fresh
cursor within a single op, so nothing is ever left queued across a later mutation.

**Observed state:** `size` and `root`, the latter rebuilt from
`mnemonist_core::structures::trie_map::NodeView` into the identical nested JSON shape upstream's
plain object already is. `root` is compared as a JSON object (order-independent by construction),
so it verifies structure; `find`/`$spread`/`$next` sequences (JSON arrays) are what verify DFS
*order*, which is where a wrong enumeration order would actually show up.

**What this grammar deliberately does not cover**, each stated rather than left to be assumed found:

* **Array mode**, entirely. `ToPropertyKey` coercion is a bridge-only concern (core has no notion of
  it); fuzzing it here would mean a third, independent reimplementation of the same coercion rule
  purely to compare against itself. Covered by the original suite's custom-tokens block and by
  `mnemonist_napi::trie_map`'s own reasoning instead.
* **Digit tokens** — see DIV-TRIE-MAP-3.
* **A starting sub-prefix on `values`/`keys`/`entries`** — every walk in this grammar starts at the
  root. Covered by `mnemonist_core::structures::trie_map::tests::walk_visits_every_word_in_the_same_order_as_find`
  and by gate 4 (`test/trie-map.js` exercises `keys('rate')` directly).
* **`delete`/`clear` interleaved with an open cursor** — DIV-TRIE-MAP-2, above. Excluded by construction
  rather than by luck, after the campaign showed it was reachable in practice, not merely in theory.

## Falsification record (gate 6)

**The assertion the sabotage had to break was named first:** the "should be possible to check the
existence of a sequence" blocks in both `test/trie.js` and `test/trie-map.js`
(`assert.strictEqual(trie.has('roman'), false)` after `trie.add('romanesque')`/
`trie.set('romanesque', 1)` — `'roman'` is a stored prefix of `'romanesque'` but never itself a
word), and the equivalent Rust assertion,
`has_distinguishes_a_stored_word_from_a_mere_prefix_of_one` in both `trie_map.rs` and `trie.rs`
(the same core method backs both, so one sabotage exercises both units at once).

**The sabotage:** `TrieMap::has` changed from checking word presence
(`node.word_index().is_some()`) to merely checking that the prefix path resolves at all
(`self.navigate(prefix).is_some()`) — collapsing exactly the distinction the falsification brief
named.

**Confirmed red, in all three places:**

* The named Rust assertions, in both modules: `assertion failed: !trie.has(tokens("roman"))`,
  plus incidental fallout in `reproduces_the_upstream_suite` and
  `delete_removes_and_prunes_singleton_chains` (both also assert a mere-prefix `has` at some point) —
  5 of 30 core tests failed.
* The original suite: 6 of the 23 `it` blocks failed across both files (17 passing, 6 failing), at
  exactly the named lines (`test/trie.js:143`, `test/trie-map.js:154`, plus the custom-tokens
  blocks, which assert the same distinction on a different prefix, and incidental fallout in the
  delete blocks, which also read `has` on a mere prefix at some point).
* **The differential fuzzer**, on both modules, minimised to two operations:

  ```
  var s = new TrieMap(false);
  s.set("abcd", {"$undefined":true});
  s.has("a");   // port: true, upstream: false
  ```

  and, for `trie`, `s.add("ba"); s.has("b");` with the same shape.

**Reverted; confirmed green again** at all three: 30/30 core tests, 23/23 original `it` blocks
(across both files), and a 200-case replay of each fuzz module comes back clean.

**Nothing was found to be blind.** Every instrument caught the sabotage independently.

## Bench table

`bench/results.json` → `modules["trie-map"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-2e5`** — 1e6 mixed `set`/`get`/`delete` (50/25/25) over hex-encoded keys, `size` 200,000,
xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **146.96** | 406.22 | 2.8× faster |
| p99 ns/op | **215.70** | 695.65 | 3.2× faster |
| RSS delta MB | **30.8** | 224.6 | |
| structure-only RSS delta MB | **0.1** | 6.7 | |
| startup ms | **0.6** | 15.9 | 26× (reported separately; not throughput) |

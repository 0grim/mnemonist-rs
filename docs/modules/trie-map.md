# trie-map

Upstream: `trie-map.js` (477 LOC) · `test/trie-map.js` (305 lines, 13 `it` blocks, 56 assertion
statements).

Port: `crates/mnemonist-core/src/structures/trie_map.rs` — `TrieMap<T, V>`, a plain-nested-object
trie generic over the token type `T` and the stored value `V`. Bridge:
`crates/mnemonist-napi/src/trie_map.rs`. Shim: `tests/bridge/trie-map.js`. Fuzz spec:
`crates/difffuzz/src/modules/trie_map.rs`.

`trie.js` is upstream's own `TrieMap.prototype` copy-and-delete — it copies every method onto
`Trie.prototype`, deletes four (`set`, `get`, `values`, `entries`), and defines its own `add` and
`find`. So `docs/modules/trie.md` and this file share almost everything; that file cross-references
here rather than repeating it, and states only what actually differs.

## What upstream tests

* **The whole `set`/`get`/`has`/`delete` contract** on plain string prefixes: setting, overwriting
  the same prefix without growing `size`, the empty-string prefix (`''`/`[]`), and asserting the
  exact `root` shape after each mutation — `test/trie-map.js` checks the raw nested object directly,
  not just `get`/`has`, for the two set/delete blocks.
* **`update`**: creates on a missing prefix, receives the old value on an existing one, and its
  result becomes the new value in one call.
* **`delete`'s pruning**, walked through by hand in the assertions: deleting a leaf removes it;
  deleting the last word under a shared prefix removes the whole now-empty chain, checked against
  the exact resulting `root`; deleting an absent prefix (including the empty string) is `false` and
  changes nothing.
* **`find`**, at three depths: matches at the exact prefix, matches under a shorter prefix, and the
  now-empty-result case for a prefix nothing extends.
* **Custom tokens** (`new TrieMap(Array)`): sequences of whole strings as one array, rather than
  characters of one string — set, `root`, `has`, `delete`, `find` all re-asserted in this mode.
* **The four lazy iterators** (`values`, `prefixes`, `entries`, and `keys` as an alias of
  `prefixes`), each with and without a starting sub-prefix argument, plus `for...of` over the
  collection itself (`Symbol.iterator`, aliased to `entries`).
* **`TrieMap.from`**, from a plain object (`{roman: 1, romanesque: 2}` — a `forEach` branch-5
  iterable, not an array or `Map`).

## What upstream does NOT test

Everything below is reachable through the public API and never exercised by `test/trie-map.js`.

**A sub-prefix argument that does not resolve to a node.** `values('nope')` and friends must return
an *empty* iterator (`Iterator.empty()` upstream), not throw and not iterate the whole trie. No test
supplies a starting prefix that fails to resolve — every sub-prefix example in the file (`'rate'`)
is itself a stored word. Pinned by
`mnemonist_core::structures::trie_map::tests::walk_over_a_prefix_that_does_not_exist_is_empty`.

**`delete` or `clear` while a lazy iterator is still open over the affected region.** This is the
single biggest gap, and it is where B-201 lives — see "Bugs this found". No test in the file ever
holds a `values()`/`keys()`/`entries()` iterator across a mutation.

**A token equal to `SENTINEL`** (`String.fromCharCode(0)`). Neither this file nor `test/trie.js`
ever embeds the reserved sentinel character in a real key. See B-200.

**Digit-shaped tokens.** `Object.keys`' integer-index-sorts-first rule never has anything to apply
to — every prefix in the file is letters.

**Two array-mode tokens that coerce to the same property-key string but are not `===`** (e.g. the
number `1` and the string `"1"`). The file's one custom-tokens block uses only distinct plain
strings, so the collision `ToPropertyKey` creates between different *raw* JS types sharing one
coerced form is never exercised, and neither is the fact that `find`/`keys`/`entries` echo the raw
argument's own leading element(s) verbatim while every newly-discovered token is the coerced string
form (verified by hand against Node — see `mnemonist_napi::trie_map`'s module docs, part 2).

**A stored value of `undefined`, held explicitly.** Every value in the test file is a real number.
`update(prefix, () => undefined)` and `has` staying `true` afterward (word presence, not value
truthiness) is untested by the original suite.

## What we test in addition

**Rust native tests** (`crates/mnemonist-core/src/structures/trie_map.rs`, 15, plus 8 more in
`trie.rs` for the wrapper):

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite`, `setting_the_same_prefix_again_does_not_increase_size`, `the_null_sequence_is_a_valid_prefix`, `update_calls_back_with_the_old_value_and_creates_when_absent`, `delete_removes_and_prunes_singleton_chains`, `find_returns_the_suffix_beyond_the_given_prefix`, `clear_resets_size_and_removes_everything` | the upstream blocks, as a baseline |
| `delete_does_not_prune_an_ancestor_that_is_itself_a_stored_word` | a sharper version of the file's own pruning check: the word one level up must survive |
| `has_distinguishes_a_stored_word_from_a_mere_prefix_of_one` | the gate 6 falsification target, pinned directly |
| `walk_over_a_prefix_that_does_not_exist_is_empty` | gap 1 |
| `walk_visits_every_word_in_the_same_order_as_find` | cross-checks the lazy walk against the eager `find` DFS, which upstream never does explicitly (they are two separate code paths upstream too) |
| `an_addition_inside_an_already_queued_branch_is_visible_to_an_open_walk` | the *matching* half of the D-201 story: a live addition (not a prune) to a node an open walk has already queued IS seen, on both sides |
| `a_token_equal_to_the_sentinel_character_is_an_ordinary_token` | gap 3 / B-200 — pins that the port does **not** reproduce the corruption |
| `root_exposes_entries_in_insertion_order` | the shared value/child enumeration order `NodeView` exposes, which is what makes `find`'s DFS order correct in the first place |
| `values_mut_reaches_every_stored_value` | bridge plumbing: every stored value must be reachable to release a JS reference on `clear`/finalize |

**Differential fuzzer** — see "Fuzz + bench". Its grammar closes gap 6 (the value alphabet includes
`undefined`) and, through `PREFIX_POOL`'s deliberate self-prefixing, exercises the shared-prefix and
prefix-of-a-word states far more densely than the original suite's hand-picked examples do.

**Still untested, stated rather than glossed:** gap 5 (the raw-vs-coerced echo distinction — proven
by hand against Node in `mnemonist_napi::trie_map`'s module docs, not by an automated test on either
side, since it requires an array-mode starting prefix the fuzz grammar deliberately excludes — see
"Fuzz + bench"). No `tests/boundary/trie-map.js` was written for this unit.

## Bugs this found

Two upstream defects, both confirmed by direct execution against Node 24.18.1, neither reachable by
`test/trie.js` or `test/trie-map.js`.

### B-200 — a token equal to `SENTINEL` corrupts the trie

`node[SENTINEL]` (the value slot) and `node[token]` (each child) are properties of the *same* plain
object. A real token equal to the sentinel string collides with the value slot:

```js
var t = new TrieMap();
t.set('a', 'word-a');
t.set('a' + TrieMap.SENTINEL + 'b', 'word-a0b');
t.size;                              // 2 -- incremented for an ORPHAN, not for anything stored
t.get('a' + TrieMap.SENTINEL + 'b'); // undefined -- unreachable through any public method
t.root;                              // { a: { '\x00': 'word-a' } } -- no trace of the second set
```

The mechanism: `node = node[token] || (node[token] = {})`. Once `node` becomes the *value* `1` (a
primitive, since the walk just read `node[SENTINEL]` where `SENTINEL` collided with the real
token), every later `node[token] = {}` is a silent no-op in sloppy mode — the assignment
*expression* still evaluates to the discarded `{}`, so the loop's local `node` keeps rebinding to a
chain of fresh, unlinked objects that vanish with the call, while `size` increments because the
final orphan genuinely has no `SENTINEL` property of its own.

**Not reproduced.** `mnemonist_core::structures::trie_map::Node` keeps the value and the children in
two separate fields, never a shared keyspace (D-200), so a token equal to whatever the bridge treats
as reserved is an entirely ordinary token here. Reproducing the corruption would mean modelling
JavaScript's primitive/object duality purely to recreate one write silently going nowhere, for a
path no test anywhere in the port reaches.

### B-201 — a `delete`/`clear` that prunes a queued cursor frame leaves it still yielding

`values`/`prefixes`/`keys`/`entries` close over live JS object references already discovered but not
yet visited. `delete`'s pruning removes a *parent's* reference to a node without necessarily
touching the node's own `SENTINEL` property; `clear` replaces `this.root` outright. Either way, an
open cursor holding the old object directly keeps reporting its stale content:

```js
var t = new TrieMap();
t.set('a', 1); t.set('ab', 2);
var it = t.entries(); it.next();   // {value: ['a', 1]}
t.delete('ab');
it.next();                          // {value: ['ab', 2]} -- the just-deleted entry, still yielded
```

Neither original test file interleaves a mutation with an open walk over the region being mutated.
**Found twice independently**: first by reading (recorded before
this unit's fuzz spec existed), then again by the differential fuzzer's own first, ungated campaign
for each unit — see "Fuzz + bench" for both minimised repros. **Not reproduced**:
`mnemonist_core::structures::trie_map::Walk` re-navigates from the root by token path rather than
holding a reference, which is required for it to be resumable from a fresh `&TrieMap` at the FFI
boundary. See D-201.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-200 | **A token equal to `SENTINEL` does not corrupt the trie.** | See B-200. Reproducing it needs JS primitive/object duality this port has no other use for, and no test reaches the path. |
| D-201 | **The lazy walk re-navigates by token path, not a live reference**, so it can disagree with upstream when a `delete`/`clear` prunes something an already-open cursor has queued. | Required regardless of B-201: the walk must be resumable at the FFI boundary, which a live Rust borrow cannot express across calls. Confirmed narrow enough that no original test reaches it; the fuzz grammar reached it on contact and now excludes the interaction explicitly (see "Fuzz + bench"). |
| D-202 | **`Object.keys`' integer-like-keys-sort-first rule is not implemented** — every node enumerates in plain insertion order. | No token in either original suite is ever a digit. The fuzz alphabet is built entirely from letters for the same reason. |
| — | **Array-mode tokens are coerced with `String(value)`, not full `ToPropertyKey`** (a `Symbol` is not accepted unchanged). | Mirrors D-91's precedent (`lru-cache`'s object keys): no test in either suite ever supplies anything but a plain string as an array-mode token. |
| — | **`find`/`values`/`keys`/`entries` return only the *suffix* beyond a given prefix from core**; the bridge concatenates it with the caller's own, uncoerced starting argument. | Verified against real Node: upstream's own `find`/iterators echo the caller's raw prefix value and only ever coerce newly-discovered tokens. Splitting the two at the core/bridge boundary is what lets core stay JS-agnostic while the bridge reproduces this exactly — see `mnemonist_napi::trie_map`'s module docs. |

## Fuzz + bench

### Fuzz

```
module=trie-map   seed=42       cases=6315  ops=635470  wall=60.0s  divergences=0
module=trie-map   seed=20260801 cases=6351  ops=628731  wall=60.0s  divergences=0
```

**Grammar:** `crates/difffuzz/src/modules/trie_map.rs`. Every prefix comes from `PREFIX_POOL`, a
small, hand-built alphabet — `a, ab, abc, abcd, b, ba, bc, bad` — chosen so prefix relationships
exist *before a single op runs*, rather than hoping random long strings collide by luck. Measured,
not asserted by eye: `pool_self_check_most_entries_are_a_prefix_of_another_entry` confirms **5 of
the 8 pool entries are themselves a strict prefix of another entry** (`a`⊂`ab`⊂`abc`⊂`abcd`,
`b`⊂`ba`⊂`bad`, `b`⊂`bc`); `pool_self_check_generated_programs_revisit_prefix_relationships` draws
2,000 samples from the real `set` op strategy across both regimes (below) and confirms the *actual
generated stream* — not just the pool in principle — mostly revisits these relationships. Both are
plain `cargo test` assertions, no oracle, no `node`.

Values are a small alphabet (`undefined`, `null`, small integers, one string) so equal values recur
constantly, matching every T3-style spec in this crate. `update` uses one named factory
(`trieIncrement`, `fuzz/oracle.js`) that increments a stored number, treating anything else as `0`.

**The regime split (D-201).** `ctor_strategy` generates one internal flag (not a real `Token`
argument — see the module's own docs) deciding whether a program exercises `delete`/`clear` or a
persistent `$iter`/`$next` cursor, never both in the same program. This exists because the campaign
run *without* the split diverged inside a few hundred operations, independently rediscovering B-201:

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
* **Digit tokens** — see D-202.
* **A starting sub-prefix on `values`/`keys`/`entries`** — every walk in this grammar starts at the
  root. Covered by `mnemonist_core::structures::trie_map::tests::walk_visits_every_word_in_the_same_order_as_find`
  and by gate 4 (`test/trie-map.js` exercises `keys('rate')` directly).
* **`delete`/`clear` interleaved with an open cursor** — D-201, above. Excluded by construction
  rather than by luck, after the campaign showed it was reachable in practice, not merely in theory.

### Falsification (gate 6)

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

### Bench

`bench/results.json` → `modules["trie-map"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-2e5`** — 1e6 mixed `set`/`get`/`delete` (50/25/25) over hex-encoded keys
(`format!("{value:x}")` / `value.toString(16)` — byte-identical, no second matched generator; see
`bench/runner/src/trie_map.rs`), `size` 200,000 kept an order of magnitude below the flat-structure
modules' 1e6 so genuine prefix-sharing (every value under 0x1000 shares its leading digits with
thousands of others) stays the dominant cost rather than sheer key count — same reasoning
`trie.rs`'s own workload already established, reused rather than re-derived. `delete`'s checksum
contribution is upstream's own plain boolean, not the `Option<V>` core's richer API exposes, so the
two sides are proven to compute the *same* answer rather than merely the same count. xorshift32
seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **146.96** | 406.22 | 2.8× faster |
| p99 ns/op | **215.70** | 695.65 | 3.2× faster |
| RSS delta MB | **30.8** | 224.6 | |
| structure-only RSS delta MB | **0.1** | 6.7 | |
| startup ms | **0.6** | 15.9 | 26× (reported separately; not throughput) |

**No regressions.** Checksum `12349076899`, identical on both sides — the shared workload walked
the same prefix-sharing tree and both implementations computed the same answer at every step,
including upstream's own `delete` return shape.

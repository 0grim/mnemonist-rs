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

`crates/mnemonist-core/src/structures/trie_map.rs` — 15 tests, plus 8 more in `trie.rs` for the
wrapper, closing every gap above except 5: a baseline reproduction of every upstream block, a
sharper pruning check (the word one level up survives), the gate-6 falsification target
(`has` distinguishing a stored word from a mere prefix of one), an empty walk over a non-existent
prefix, the lazy walk cross-checked against the eager `find` DFS, an addition inside an
already-queued branch staying visible to an open walk, a token equal to the sentinel character
proven ordinary, insertion-order enumeration, and every stored value reachable for bridge release.
Full test-to-gap mapping: evidence file.

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
Found by reading and confirmed independently by the differential fuzzer's own first campaign for
this unit — see "Fuzz + bench" for the minimised repro. **Not reproduced**:
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

Two campaigns, two seeds:

```
module=trie-map   seed=42       cases=6315  ops=635470  wall=60.0s  divergences=0
module=trie-map   seed=20260801 cases=6351  ops=628731  wall=60.0s  divergences=0
```

Every prefix comes from `PREFIX_POOL`, a small, hand-built alphabet — `a, ab, abc, abcd, b, ba, bc,
bad` — chosen so prefix relationships exist *before a single op runs*, rather than hoping random
long strings collide by luck: 5 of the 8 pool entries are themselves a strict prefix of another
entry, and a self-check on the actual generated stream confirms these relationships are revisited
constantly rather than only present in principle. Values are a small alphabet (`undefined`, `null`,
small integers, one string) so equal values recur constantly, matching every T3-style spec in this
crate. `update` uses one named factory (`trieIncrement`) that increments a stored number, treating
anything else as `0`.

**The regime split (D-201).** The constructor strategy generates one internal flag deciding whether
a program exercises `delete`/`clear` or a persistent `$iter`/`$next` cursor, never both in the same
program — because the campaign run *without* the split diverged inside a few hundred operations,
independently rediscovering B-201. `$spread` (`Array.from`) is exempt from the split in both
regimes: it opens and fully drains a fresh cursor within a single op, so nothing is ever left queued
across a later mutation.

Observed state is `size` and `root`, the latter rebuilt into the identical nested JSON shape
upstream's plain object already is; `root` is compared as an order-independent JSON object, so it
verifies structure, while `find`/`$spread`/`$next` sequences (JSON arrays) verify DFS *order*.

**What this grammar deliberately does not cover:** array mode entirely (`ToPropertyKey` coercion is
a bridge-only concern; covered by the original suite's custom-tokens block instead), digit tokens
(D-202), a starting sub-prefix on the lazy iterators (every walk in this grammar starts at the
root; covered by a dedicated Rust test and by gate 4), and `delete`/`clear` interleaved with an open
cursor (D-201, excluded by construction after the campaign showed it was reachable in practice, not
merely in theory). Full grammar: evidence file.

### Falsification (gate 6)

**The assertion the sabotage had to break was named first:** the "should be possible to check the
existence of a sequence" blocks in both `test/trie.js` and `test/trie-map.js` — `'roman'` is a
stored prefix of `'romanesque'` but never itself a word — and the equivalent Rust assertion,
`has_distinguishes_a_stored_word_from_a_mere_prefix_of_one` in both `trie_map.rs` and `trie.rs` (the
same core method backs both, so one sabotage exercises both units at once).

**The sabotage:** `TrieMap::has` changed from checking word presence to merely checking that the
prefix path resolves at all — collapsing exactly the distinction the falsification brief named.

**Confirmed red, in all three places:** the named Rust assertions in both modules (5 of 30 core
tests failed, including incidental fallout in tests that assert the same distinction elsewhere); the
original suite (6 of the 23 `it` blocks failed across both files, 17 passing, 6 failing, at exactly
the named lines and their custom-tokens/delete-block counterparts); and the differential fuzzer, on
both modules, minimised to two operations. Reverted; confirmed green again at all three: 30/30 core
tests, 23/23 original `it` blocks, and a 200-case replay of each fuzz module comes back clean.
Nothing was found to be blind — every instrument caught the sabotage independently. Full record:
evidence file.

### Bench

`bench/results.json` → `modules["trie-map"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-2e5`** — 1e6 mixed `set`/`get`/`delete` (50/25/25) over hex-encoded keys, `size` 200,000
kept an order of magnitude below the flat-structure modules' 1e6 so genuine prefix-sharing (every
value under 0x1000 shares its leading digits with thousands of others) stays the dominant cost
rather than sheer key count — same reasoning `trie.rs`'s own workload already established, reused
rather than re-derived: the port is 2.8× faster at p50 (146.96 vs 406.22 ns/op), 3.2× faster at p99.
No regressions. Full table: evidence file.

`delete`'s checksum contribution is upstream's own plain boolean, not the `Option<V>` core's richer
API exposes, so the two sides are proven to compute the *same* answer rather than merely the same
count. Checksum `12349076899`, identical on both sides — the shared workload walked the same
prefix-sharing tree and both implementations computed the same answer at every step, including
upstream's own `delete` return shape.

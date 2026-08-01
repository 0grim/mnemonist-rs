# inverted-index

Upstream: `inverted-index.js` (249 LOC) · `test/inverted-index.js` — **126 lines, 8 `it` blocks,
~20 assertion statements** (several via `deepStrictEqual` on arrays/iterators).

Port: `crates/mnemonist-core/src/structures/inverted_index.rs`. Bridge:
`crates/mnemonist-napi/src/inverted_index.rs`. Shim: `tests/bridge/inverted-index.js`. Fuzz spec:
`crates/difffuzz/src/modules/inverted_index.rs`.

A document store plus a token → posting-list index: `add(doc)` tokenizes `doc` and records it
under every distinct token it contains; `get(query)` tokenizes the query and returns every
document containing **all** of its tokens (a boolean AND, via repeated sorted-array intersection).
Read `crates/mnemonist-core/src/structures/inverted_index.rs`'s own module docs first — they carry
the cursor-capture reasoning in full; this file adds the six required sections.

---

## What upstream tests

Eight blocks:

```js
new InvertedIndex({hello: 'world'});           // throws /tokenizer/
index.add(OBJECT_DOCS[0]);                     // (no tokenizer given) throws /array/
index.add(doc); // x3, with a real tokenizer (lodash/words + a stopword filter + a stemmer)
assert.strictEqual(index.size, 3);
assert.strictEqual(index.dimension, 7);
InvertedIndex.from(OBJECT_DOCS, documentTokenizer);
index.get('A mouse.');           // AND query, several shapes including "matches nothing"
index.forEach(function (doc, i, instance) { ... });     // asserts args, never counts calls
index.documents();               // drained iterator, deepStrictEqual per step
index.tokens();                  // drained iterator, all seven tokens in order
```

Characterising the shape of that coverage:

* **Every document in the suite shares tokens with at least one other document.** The three
  `DOCS` strings (`'The cat eats the mouse.'`, `'The mouse likes cheese.'`, `'Cheese is something
  mouses really like to eat.'`) collide constantly after stemming/stopwords: `mouse`, `cheese`,
  `eat` each appear in two or more. This is precisely the shape CLAUDE.md's brief for this batch
  asked to be reached, and the original suite already reaches it — just narrowly (three documents,
  seven tokens total).
* **`forEach`'s block asserts the callback's arguments on every invocation, but never counts how
  many invocations happened.** This is exactly the gap B-240 (below) hides in, and it means gate 4
  cannot catch B-240 on its own.
* **No document is ever added after `documents()`/`tokens()` is called.** Both iterators in the
  suite are opened on an already-final index and drained immediately; neither cursor's *liveness*
  is exercised at all.
* **`clear()` is never called.** The port defect this unit's own first fuzz campaign found (below)
  is entirely inside `clear()`'s interaction with an open cursor, and the original suite has zero
  coverage of `clear()` in any form.
* **The two `/tokenizer/`/`/array/` throw blocks are the only two that ever reach the identity
  fallback** — `new InvertedIndex()` with no descriptor at all, then `.add()` on a document that
  is not an array, which is exactly what triggers upstream's `identity` default and the
  `Array.isArray` guard together.

## What upstream does NOT test

**`clear()` — the whole territory this unit's own first fuzz campaign found**

1. **`clear()` while a `documents()`/`tokens()` cursor is open is never done.** This is not a
   narrow gap; it is a real port defect the campaign hit inside a few hundred generated cases (see
   "Bugs this found").
2. **`clear()`'s own effect on `size`/`dimension`/a subsequent `get`/`add` is never checked at
   all** — the method is never called in the original suite.

**`forEach`'s invocation count — B-240's whole territory**

3. **The number of times `forEach`'s callback runs is never counted.** Gap 3 alone is why gate 4
   cannot find B-240; see "Bugs this found."

**Cursor liveness, the other half**

4. **A document `add`ed after a `documents()`/`tokens()` cursor opens, but before the cursor
   finishes, is never checked** for whether it is visible (it is not — both cursors freeze a
   length/capture the array or map object at open time).
5. **`Array.from(index)` (the collection's own `Symbol.iterator`, aliased to `documents`) is never
   used**, so the *factory* half of D-07 has zero coverage here, same gap default-map's own docs
   describe for that module.

**The tokenizer's fallback and error paths, beyond the two blocks that use them**

6. **A falsy-but-non-`undefined` descriptor** (`0`, `''`, `false`, `null`, `NaN`) is never passed;
   only an entirely omitted argument reaches the `identity` fallback in the original suite, so
   upstream's `!this.documentTokenizer` *truthiness* test (not `typeof … === 'undefined'`) is
   exercised only in its one narrowest form.
7. **A descriptor that is an array with fewer than two elements** (`[fn]`, giving
   `queryTokenizer = undefined` → `identity`) is never passed.
8. **A tokenizer that throws** is never used, so nothing pins whether a document is partially
   indexed or an error propagates cleanly.

**Never called at all**

9. `inspect()` and the `nodejs.util.inspect.custom` symbol.

## What we test in addition

**`crates/mnemonist-core/src/structures/inverted_index.rs` — 19 tests:**

| Test | Closes gap |
|---|---|
| `adding_documents_updates_size_and_dimension`, `querying_returns_the_and_intersection_of_matching_documents`, `from_iter_builds_the_same_index_as_repeated_add`, `documents_iterates_in_insertion_order`, `tokens_iterates_in_first_seen_order` | the eight blocks, as a baseline (tokenizer replaced by plain whitespace-split — see the fuzz spec's own docs on why) |
| `b_240_for_each_never_visits_a_single_document`, `b_240_holds_on_an_empty_index_too` | 3 — B-240, pinned directly |
| `a_clear_between_two_steps_of_an_open_documents_cursor_does_not_panic_and_finishes_the_old_array`, `a_clear_between_two_steps_of_an_open_for_each_walk_does_not_panic`, `a_clear_between_two_steps_of_an_open_tokens_cursor_finishes_the_old_mapping` | 1 — the port defect the fuzzer found, both cursors |
| `documents_cursor_is_not_restartable_and_does_not_grow_after_it_reports_done`, `a_document_added_after_a_cursor_opens_is_not_visible_because_the_length_is_frozen` | 4 |
| `a_query_before_any_document_is_added_returns_nothing`, `an_empty_query_returns_nothing`, `a_repeated_token_within_one_document_is_recorded_once`, `clear_resets_everything`, `get_only_matches_documents_containing_every_query_token` | general correctness |

**The differential fuzzer's `grammar_self_check`** measures, rather than asserts from op weights,
how often generated documents actually collide on tokens: over 400 generated programs (up to 300
`add`s each), **58,643 documents added, 1,993 posting lists, 1,985 of them (99.6%) spanning more
than one document.** This is the direct answer to CLAUDE.md's brief for this unit: the grammar
does not merely prove the index can store words.

**Still untested, stated rather than glossed:** gap 5 (`$spread`/`Array.from` *is* fuzzed — see
"Fuzz" — but the original suite's own gap remains, since gate 4 never uses it), gaps 6-8 (the
tokenizer's fallback/error paths beyond the identity default — this port's fuzz grammar always
uses the identity tokenizer by construction, see the fuzz spec's own docs, so a real callback's
truthiness fallback and throwing behaviour are exercised only by the two original-suite blocks and
by reading the bridge's own `resolve_tokenizer`/`tokenize`, not by fuzzing), gap 9 (`inspect`, not
bridged).

## Bugs this found

**B-240 — `forEach` never calls its callback, regardless of how many documents are stored.**
`status: verified against Node 24.18.1`.

```js
InvertedIndex.prototype.forEach = function(callback, scope) {
  scope = arguments.length > 1 ? scope : this;
  for (var i = 0, l = this.documents.length; i < l; i++)
    callback.call(scope, this.documents[i], i, this);
};
```

`this.documents` is the **method** `InvertedIndex.prototype.documents`, defined a few lines above
— not `this.items`, the property that actually holds the document array. A JavaScript function's
`.length` is its declared parameter count, and `documents` takes none, so `this.documents.length`
is `0`: not "usually 0," not "0 past some threshold" — the literal, permanent arity of a
zero-argument function. The loop condition `i < l` is `0 < 0` on the very first check, for every
call, on an index with one document or with a thousand:

```text
var index = InvertedIndex.from(['a b', 'b c'], s => s.split(' '));
var times = 0;
index.forEach(function () { times++; });
times;   // 0
```

The original suite's own `forEach` block asserts properties of each invocation but never counts
how many happened (gap 3), so it passes identically whether the callback runs zero times or *n*
times — gate 4 structurally cannot find this. Reproduced rather than "fixed": a walk that visited
every document would be the *correct*, useful behaviour, and is exactly what a careful porter
would write without reading this file line by line — which is precisely why it would be a defect
under CLAUDE.md's bug-for-bug mandate. `InvertedIndex::for_each` hands back a cursor frozen at
length **zero**, unconditionally, so the loop bound really is zero here, not a value merely
rendered as if it were. Confirmed by the differential fuzzer's own `$forEach` op, which asserts
`seen: []` on every single generated case regardless of index size — positive, repeated evidence
rather than the original suite's one hand-picked call.

**A real port defect, found by this unit's own first fuzz campaign, fixed before any campaign was
logged in `fuzz/log.txt`.** An earlier cut of `documents()`'s cursor re-read `self.items` (a plain
`Vec`) against a length frozen at the cursor's own open time. Upstream's `clear()`,

```js
InvertedIndex.prototype.clear = function() {
  this.items = [];
  this.mapping = new Map();
  this.size = 0;
  this.dimension = 0;
};
```

**rebinds** `this.items`/`this.mapping` to fresh containers — it does not empty them in place, the
opposite of `default-map.js`'s own `clear` (`this.items.clear()`, the same native `Map` object).
Confirmed against Node 24.18.1: opening a `documents()`/`tokens()` cursor, calling `clear()`, then
stepping the cursor again still yields the **pre-clear** documents/tokens — the cursor is entirely
invisible to the rebind, because it captured the array/map *object*, not a promise to keep
re-reading `this.items`/`this.mapping`. The port's first cut re-read the live field instead, which
is indistinguishable right up until a `clear()` happens between two steps of an open cursor — where
it read past the end of the now-empty `Vec` and **panicked**:

```
index out of bounds: the len is 0 but the index is 0
```

Found by `crates/difffuzz/src/modules/inverted_index.rs`'s very first campaign, inside its first
few hundred generated cases. Fixed by making both `items` and `mapping` `Rc<RefCell<_>>` — the
exact shape `crate::structures::queue::Queue`'s own `Items<T>` already uses for the identical
reason — with `DocumentsCursor`/`TokensCursor` each capturing an `Rc` clone at open time, so a
later `clear()` rebinding the field never touches what an already-open cursor holds. `tokens()`'s
half of this (the `mapping` side) was found by re-deriving the same reasoning under Node rather
than by the fuzzer catching it directly — its own campaign never happened to generate the precise
`tokens()`-then-`clear()`-then-`step()` interleaving in the time available, which is itself a
finding worth stating rather than glossing: a clean campaign is evidence of absence only up to the
grammar's actual coverage of the state space in the time it ran, not a proof of absence.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **Tokenizing is entirely the bridge's job.** `mnemonist-core`'s `add`/`get` both take an already-tokenized `Vec<Tok>`; the `Array.isArray` guard and the constructor's truthiness-based tokenizer resolution are JavaScript questions through and through, and live in `crates/mnemonist-napi/src/inverted_index.rs`'s `resolve_tokenizer`/`tokens_from_unknown`. Identical reasoning to `default_map`'s factory and `default_weak_map`'s. |
| — | **Tokens are `crate::js_key::JsKey`, not a bespoke type.** `mapping` is a real `Map`, and its keys compare with SameValueZero — the same T3 reasoning `default_map.rs` documents, reused rather than reinvented. |
| — | **The `identity` fallback is modelled as `Option::None`, not a materialised JS closure.** Upstream's `function identity(x) { return x; }` is a real function object; this port's `resolve_tokenizer` returns `None` for the falsy-descriptor case and `JsInvertedIndex::tokenize` applies the identical `Array.isArray`-then-convert rule directly to the input, without ever constructing or calling a JS function. Observationally identical — the input is handed back and validated exactly as calling `identity` and validating its return value would be — and avoids the `Function`-lifetime-casting machinery a real closure would need for no behavioural gain. |
| — | **`inspect()` is not ported.** It returns `this.items.slice()` with a constructor-name trick for Node's REPL; nothing asserts on it. |
| D-06 | No collection implements `IntoIterator`; unchanged from every other module in this port. |
| D-07 | `Symbol.iterator` is installed from Rust via `ITERATOR_FACTORIES`, aliased to `documents` — upstream's own last line, matching the table's existing precedent of not assuming `values` for every module (`default-map` aliases `entries`, `Trie` aliases `keys`). |

## Fuzz + bench

### Fuzz

```
module=inverted-index  seed=42       cases=9600 ops=968510 wall=60.0s divergences=0
module=inverted-index  seed=20260801 cases=9632 ops=967012 wall=60.0s divergences=0
```

Two campaigns, two seeds, **1.94M operations, zero divergences** — against a build that already
carries the `clear()`/cursor-capture fix from "Bugs this found." Both this fix and B-240 predate
these logged campaigns; B-240 is reproduced by construction (see above) rather than found by
fuzzing, and the `clear()` defect was found by an earlier, unlogged run before the fix landed.
Reproduce with `target/release/difffuzz --module inverted-index --seed 42 --cases 9600`.

* **Grammar: identity tokenizer, documents ARE token arrays.** Every `InvertedIndex` in this
  grammar is constructed with `descriptor` omitted, so both sides fall back to upstream's own
  `identity`. Documents (and queries) are generated directly as **arrays of tokens drawn from a
  five-word pool** (`a`..`e`) rather than natural-language strings run through a real tokenizer:
  `identity(doc) === doc` and `Array.isArray(doc)` holds by construction, so the constructor's and
  `add`'s own guards are satisfied for free, and a 1–4-token document over a five-word pool
  collides with earlier documents constantly (measured: 99.6%, see "What we test in addition") —
  reaching real natural-language overlap would mean porting or mirroring a real tokenizer
  (stemming, stopwords, `lodash/words`) into the fuzz harness for no gain the module itself needs.
* **Op alphabet:** `add` (5, 1–4 tokens), `get` (4, 0–3 tokens — upstream's own `if
  (!tokens.length) return [];` branch is reachable), `clear` (1), `$iter` over
  `documents`/`tokens` (2), `$next` (4), `$spread` (1), `$forEach` (1 — always the "plain walk"
  shape, since the mutation table `for_each_strategy` takes is empty; see below).
* **Two cursor shapes, both fuzzed, tagged by `FuzzCursor`:** `documents()` (a frozen length over
  a captured array) and `tokens()` (a real `Map` cursor over a captured map) are genuinely
  different walks, matching the core module's own two-cursor design.
* **`$forEach` is included specifically as an invariant, not a mutation vector.** Because
  `InvertedIndex::for_each` always drives a cursor frozen at length zero, `seen` is `[]` on every
  single generated case regardless of `size` — this op's whole purpose is to be *positive,
  repeated* evidence that the port's brokenness matches upstream's across thousands of index
  states, not merely the original suite's one hand-picked call.
* **Observable state, compared after every op:** `size`, `dimension`, `items` (the full document
  list, in order) and `mapping` (the full token → posting-list index, as an order-sensitive
  `$map` — entry order is part of what `tokens()` promises, same reasoning as `default-map`'s
  `items`).

### Falsification (gate 6)

**The assertion named first:** `b_240_for_each_never_visits_a_single_document`'s
`assert_eq!(cursor.step(), None, "the loop bound is zero, unconditionally")`.

**The sabotage:** `InvertedIndex::for_each` changed from `DocumentsCursor::open_at_zero(...)` to
`DocumentsCursor::open(...)` — i.e., made it actually walk the documents. The *correct*, useful
behaviour, and therefore a bug per CLAUDE.md's mandate.

**Confirmed red, in two of the three places this could be caught:**

* The named Rust assertion, plus one more it took down with it
  (`a_clear_between_two_steps_of_an_open_for_each_walk_does_not_panic`, which also assumed a
  zero-length walk): `2 failed, 15 passed`.
* **The differential fuzzer caught it immediately**: 148 cases, 58 operations, 0.2 seconds,
  minimised to two lines:

  ```js
  var s = new InvertedIndex();
  s.add(["a"]);
  s.forEach(function (a, b) {});
  // port saw one callback invocation ([["a"]]); upstream saw none ([])
  ```

**Confirmed green, correctly, for a stated reason.** The original mocha suite stayed green (`8
passing`) — expected, and the point of gap 3: it never counts `forEach` invocations, so it cannot
distinguish "ran once" from "ran zero times" either way.

**Reverted; confirmed green again** at both instruments: the two Rust assertions pass (`17
passing`), and a 500-case replay of the same seed comes back `0 divergences`.

**Nothing was found to be blind here that was not already known and stated**: the original suite's
blindness to this exact class of bug (gap 3) is precisely why B-240 needed the differential
fuzzer's `$forEach` op to be pinned as *continuous* evidence in the first place, rather than a
single hand-picked assertion.

### Bench

**Not run.** Gate 10 is deferred to the batched quiet pass (DESIGN.md §7.3). `inverted-index` is
therefore **complete except gate 10** and correctly absent from `tests/scope.txt` until that pass
lands.

One thing to watch when it does: `get`'s repeated `intersection_unique_two` fold is left-to-right
over `query_tokens`, matching upstream's own `helpers.intersectionUniqueArrays(results, c)` loop
rather than the k-way form `mnemonist-core::utils::merge` also provides (which upstream itself
never uses here) — whether that ordering has a measurable cost relative to a k-way scan at
realistic query lengths (rarely more than a handful of tokens) is a question for the measurement,
not for this document.

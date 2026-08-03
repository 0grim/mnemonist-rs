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
  `eat` each appear in two or more. This is precisely the shape this unit's own porting notes
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

`crates/mnemonist-core/src/structures/inverted_index.rs` — 19 tests, closing every gap above except
5–9: a baseline reproduction of all eight blocks (tokenizer replaced by plain whitespace-split — see
the fuzz spec's own docs on why), B-240 pinned directly on an index of any size and on an empty
index, `clear` between two steps of an open cursor for all three walks not panicking and finishing
the pre-clear data, a document added mid-walk staying invisible because the length is frozen, and
general correctness (empty query, repeated token dedup, `clear` resetting everything). Full
test-to-gap mapping: evidence file.

**The differential fuzzer's `grammar_self_check`** measures, rather than asserts from op weights,
how often generated documents actually collide on tokens: over 400 generated programs (up to 300
`add`s each), **58,643 documents added, 1,993 posting lists, 1,985 of them (99.6%) spanning more
than one document.** This is the direct answer to the porting brief for this unit: the grammar
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
Verified against Node 24.18.1.

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
under this port's bug-for-bug fidelity rule. `InvertedIndex::for_each` hands back a cursor frozen at
length **zero**, unconditionally, so the loop bound really is zero here, not a value merely
rendered as if it were. Confirmed by the differential fuzzer's own `$forEach` op, which asserts
`seen: []` on every single generated case regardless of index size — positive, repeated evidence
rather than the original suite's one hand-picked call.

**A real port defect, found by this unit's own first fuzz campaign, fixed before any campaign was
logged.** An earlier cut of `documents()`'s cursor re-read `self.items` (a plain
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

Found inside the first few hundred cases of this unit's own first fuzz campaign. Fixed by making
both `items` and `mapping` `Rc<RefCell<_>>` — the exact shape `crate::structures::queue::Queue`'s
own `Items<T>` already uses for the identical reason — with `DocumentsCursor`/`TokensCursor` each
capturing an `Rc` clone at open time, so a later `clear()` rebinding the field never touches what an
already-open cursor holds. `tokens()`'s half of this (the `mapping` side) was found by re-deriving
the same reasoning under Node rather than by the fuzzer catching it directly — its own campaign
never happened to generate the precise `tokens()`-then-`clear()`-then-`step()` interleaving in the
time available, which is itself worth stating: a clean campaign is evidence of absence only up to
the grammar's actual coverage of the state space in the time it ran, not a proof of absence.

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

Two campaigns, two seeds, **1.94M operations, zero divergences** — against a build that already
carries the `clear()`/cursor-capture fix from "Bugs this found":

```
module=inverted-index  seed=42       cases=9600 ops=968510 wall=60.0s divergences=0
module=inverted-index  seed=20260801 cases=9632 ops=967012 wall=60.0s divergences=0
```

Reproduce with `target/release/difffuzz --module inverted-index --seed 42 --cases 9600`. Both this
fix and B-240 predate these logged campaigns: B-240 is reproduced by construction (see above) rather
than found by fuzzing, and the `clear()` defect was found by an earlier, unlogged run before the fix
landed.

**Grammar: identity tokenizer, documents ARE token arrays.** Every `InvertedIndex` in this grammar
is constructed with `descriptor` omitted, so both sides fall back to upstream's own `identity`.
Documents (and queries) are generated directly as **arrays of tokens drawn from a five-word pool**
rather than natural-language strings run through a real tokenizer, so a 1–4-token document over a
five-word pool collides with earlier documents constantly (measured: 99.6%, see "What we test in
addition") — reaching real natural-language overlap would mean porting or mirroring a real tokenizer
into the fuzz harness for no gain the module itself needs. The op alphabet covers `add`/`get`/`clear`
plus the cursor ops over both cursor shapes (`documents()`, a frozen length over a captured array,
and `tokens()`, a real `Map` cursor over a captured map) and `$forEach` — included specifically as
an *invariant*, not a mutation vector: because `InvertedIndex::for_each` always drives a cursor
frozen at length zero, `seen` is `[]` on every single generated case regardless of `size`, so this
op's whole purpose is to be positive, repeated evidence that the port's brokenness matches
upstream's across thousands of index states, not merely the original suite's one hand-picked call.
Observable state is `size`, `dimension`, `items` (the full document list, in order) and `mapping`
(the full token → posting-list index, order-sensitive, since entry order is part of what `tokens()`
promises). Full grammar: evidence file.

**Falsification (gate 6):** the assertion named first was
`b_240_for_each_never_visits_a_single_document`'s
`assert_eq!(cursor.step(), None, "the loop bound is zero, unconditionally")`. The sabotage —
`InvertedIndex::for_each` changed from `DocumentsCursor::open_at_zero(...)` to
`DocumentsCursor::open(...)`, i.e. made it actually walk the documents, the *correct*, useful
behaviour and therefore a bug per this port's bug-for-bug fidelity rule — is confirmed red in two of
the three places this could be caught: the named Rust assertion plus one more it took down with it
(2 failed, 15 passed), and the differential fuzzer, which caught it immediately (148 cases, 58
operations, 0.2 seconds, minimised to two lines). The original mocha suite stayed green (8 passing),
correctly: it never counts `forEach` invocations, so it cannot distinguish "ran once" from "ran zero
times" either way — precisely why B-240 needed the fuzzer's `$forEach` op as continuous evidence
rather than a single hand-picked assertion. Reverted; confirmed green again (17 passing, 0
divergences on a 500-case replay). Full record: evidence file.

### Bench

`bench/results.json` → `modules["inverted-index"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 2,000 samples/side.

**`mixed-2e5`** — 200,000 (deliberately smaller than this group's usual 1e6 — see
`bench/runner/src/inverted_index.rs`'s own module docs for the sizing check) mixed
`add`(2-token doc)/`get`(1-token query)/`get`(2-token AND query) (50/25/25) over a 1,000-word
vocabulary, identity tokenizer on both sides, with ~200 documents per posting list on average by the
run's end (the load-bearing multi-container parameter, exercising both a plain posting-list read and
a real two-list AND intersection): the port is 1.7× faster at p50 (249.5 vs 424.4 ns/op), 2.0×
faster at p99 (539.3 vs 1104.4). No regressions. Full table: evidence file.

This has the largest RSS-delta ratio in this group (upstream's ~118 MB against the port's ~2 MB) —
most of that gap is `add`'s per-document allocation cost: every `add` upstream makes allocates a
fresh `Set()` for its dedup pass and grows a plain-object-backed `Map`, while the port's `add`
allocates a `HashSet` scoped to one call and writes into an already-hashed `OrderedMap`. One thing
still open, unconfirmed either way: `get`'s repeated `intersection_unique_two` fold is left-to-right
over `query_tokens`, matching upstream's own `helpers.intersectionUniqueArrays(results, c)` loop
rather than the k-way form `mnemonist-core::utils::merge` also provides — this workload's queries
are at most two tokens, so the two folds are identical at this length, and whether a k-way scan
would matter at longer queries remains a question for a future measurement, not settled by this one.

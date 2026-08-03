# inverted-index — evidence

Gate artifacts for `docs/modules/inverted-index.md`: test-to-gap table, fuzz grammar, full
falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/inverted_index.rs` — 19 tests:

| Test | Closes gap |
|---|---|
| `adding_documents_updates_size_and_dimension`, `querying_returns_the_and_intersection_of_matching_documents`, `from_iter_builds_the_same_index_as_repeated_add`, `documents_iterates_in_insertion_order`, `tokens_iterates_in_first_seen_order` | the eight blocks, as a baseline (tokenizer replaced by plain whitespace-split — see the fuzz spec's own docs on why) |
| `b_240_for_each_never_visits_a_single_document`, `b_240_holds_on_an_empty_index_too` | 3 — BUG-INVERTED-INDEX-1, pinned directly |
| `a_clear_between_two_steps_of_an_open_documents_cursor_does_not_panic_and_finishes_the_old_array`, `a_clear_between_two_steps_of_an_open_for_each_walk_does_not_panic`, `a_clear_between_two_steps_of_an_open_tokens_cursor_finishes_the_old_mapping` | 1 — the port defect the fuzzer found, both cursors |
| `documents_cursor_is_not_restartable_and_does_not_grow_after_it_reports_done`, `a_document_added_after_a_cursor_opens_is_not_visible_because_the_length_is_frozen` | 4 |
| `a_query_before_any_document_is_added_returns_nothing`, `an_empty_query_returns_nothing`, `a_repeated_token_within_one_document_is_recorded_once`, `clear_resets_everything`, `get_only_matches_documents_containing_every_query_token` | general correctness |

## Fuzz grammar

* **Grammar: identity tokenizer, documents ARE token arrays.** Every `InvertedIndex` in this
  grammar is constructed with `descriptor` omitted, so both sides fall back to upstream's own
  `identity`. Documents (and queries) are generated directly as **arrays of tokens drawn from a
  five-word pool** (`a`..`e`) rather than natural-language strings run through a real tokenizer:
  `identity(doc) === doc` and `Array.isArray(doc)` holds by construction, so the constructor's and
  `add`'s own guards are satisfied for free, and a 1–4-token document over a five-word pool
  collides with earlier documents constantly (measured: 99.6%).
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

## Falsification record (gate 6)

**The assertion named first:** `b_240_for_each_never_visits_a_single_document`'s
`assert_eq!(cursor.step(), None, "the loop bound is zero, unconditionally")`.

**The sabotage:** `InvertedIndex::for_each` changed from `DocumentsCursor::open_at_zero(...)` to
`DocumentsCursor::open(...)` — i.e., made it actually walk the documents. The *correct*, useful
behaviour, and therefore a bug per this port's bug-for-bug fidelity rule.

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

## Bench table

`bench/results.json` → `modules["inverted-index"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 2,000 samples/side.

**`mixed-2e5`** — 200,000 mixed `add`(2-token doc)/`get`(1-token query)/`get`(2-token AND query)
(50/25/25) over a 1,000-word vocabulary, identity tokenizer on both sides, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **249.5** | 424.4 | 1.7× faster |
| p99 ns/op | **539.3** | 1104.4 | 2.0× faster |
| RSS delta MB | **2.1** | 118.2 | |
| structure-only RSS delta MB | **0.1** | 0.2 | |
| startup ms | **0.6** | 16.9 | 28× (reported separately; not throughput) |

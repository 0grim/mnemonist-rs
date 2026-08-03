# multi-map — evidence

Gate artifacts for `docs/modules/multi-map.md`: test-to-gap table, fuzz grammar, full benchmark
table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/multi_map.rs` — 9 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_walkthrough`, `remove_matches_upstream_size_and_deletion_bookkeeping`, `delete_removes_the_whole_bucket`, `clear_resets_size_and_dimension` | the upstream blocks, as a baseline |
| `set_kind_deduplicates_by_the_supplied_equality`, `list_kind_never_deduplicates` | the `Set`/`Array` write-path contrast directly, over a `V` with no `Hash`/`Eq` at all |
| `remove_on_a_set_kind_bucket_drops_the_key_once_it_empties` | the drain-to-zero path, `Set`-kind |
| `a_key_deleted_ahead_of_a_live_cursor_is_skipped` | the flattened cursor's outer liveness |
| `set_with_hands_a_rejected_duplicate_back_instead_of_dropping_it` | the resource-leak contract `fuzzy_multi_map`'s bridge depends on (see the document's "Bugs this found") |
| `fallible_equality_short_circuits_on_the_first_error` | the fallible `set_with`/`remove_with` machinery, over a comparator that returns `Err` |

## Fuzz grammar

* **Op alphabet:** `set` (weight 5, the only op that grows a bucket), `remove` (3), `delete`/`has`/
  `get`/`multiplicity` (2 each), `clear` (1). Cursor-lifecycle ops (`$iter`/`$next`/`$spread`) are
  deliberately not in this alphabet.
* **Key pool:** three keys (`"a"`, `"b"`, `"c"`), small enough that `set`/`remove` collide
  constantly rather than spreading across a wide, mostly-empty map.
* **Value pool:** four values, two strings and two numbers, wide enough that a `Set`-kind bucket
  sees genuine duplicates and genuine distinct members both.
* **Constructor:** alternates between the default container and `{"$global": "Set"}`, so both
  bucket kinds get their own campaign share.
* **Observable state:** `size`, `dimension`, and `items` rendered exactly as `fuzz/oracle.js`'s
  `encode()` renders the real per-key `Array`/`Set`.

## Bench table

`bench/results.json` → `modules["multi-map"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`get`/`remove` (50/25/25), over a 20,000-key domain, xorshift32
seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **25.9** | 36.4 | 1.4× faster |
| p99 ns/op | **46.1** | 89.8 | 1.9× faster |
| RSS delta MB | **11.6** | 79.3 | |
| structure-only RSS delta MB | **0.1** | 5.8 | |
| startup ms | **0.6** | 16.8 | 28× (reported separately; not throughput) |

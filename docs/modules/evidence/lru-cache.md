# lru-cache — evidence

Gate artifacts for `docs/modules/lru-cache.md`: test-to-gap table, fuzz grammar, full campaign log,
full falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/lru_cache.rs` — 13 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_walkthrough`, `setpop_reports_none_overwritten_and_evicted`, `capacity_of_one_evicts_on_every_new_key`, `delete_and_remove_maintain_lru_order`, `peek_does_not_disturb_order`, `clear_resets_bookkeeping_but_a_stale_slot_is_never_reachable`, `keys_and_values_project_the_same_walk_differently` | the upstream blocks, as a baseline |
| `zero_capacity_is_refused` | the numeric half of the invalid-capacity guard |
| `a_deleted_slot_is_reused_by_the_next_insert` | the hole-reuse path the two "healthy workout" blocks exercise indirectly |
| `eviction_re_derives_the_index_key_from_the_stored_key_and_can_leave_it_stale` | gap 7 |
| `a_delete_of_the_walks_next_unvisited_pointer_yields_stale_data_not_a_panic` | gap 2 — the port defect this pins |
| `a_remove_of_the_walks_next_unvisited_pointer_yields_stale_data_not_a_panic` | the `remove` half of the same defect |
| `a_freed_pointer_reused_before_a_stale_walk_reaches_it_surfaces_the_new_occupant` | gap 3 |

## Fuzz campaign log

```
module=lru-cache               seed=42       cases=4665 ops=711010  wall=60.0s divergences=0
module=lru-cache               seed=20260801 cases=4869 ops=731558  wall=60.0s divergences=0
module=lru-cache-with-delete   seed=42       cases=5027 ops=755972  wall=60.0s divergences=0
module=lru-cache-with-delete   seed=20260801 cases=5076 ops=759840  wall=60.0s divergences=0
module=lru-map                 seed=42       cases=5534 ops=837947  wall=60.0s divergences=0
module=lru-map                 seed=20260801 cases=5649 ops=844111  wall=60.0s divergences=0
module=lru-map-with-delete     seed=42       cases=5559 ops=834690  wall=60.0s divergences=0
module=lru-map-with-delete     seed=20260801 cases=5988 ops=896591  wall=60.0s divergences=0
```

## Fuzz grammar

* **Op alphabet:** `get` (weight 8, the heaviest of any op — it is the mutating read, and an LRU's
  whole point is that a read changes recency), `peek`/`has` (2 each, the non-mutating controls),
  `set` (6), `setpop` (3), `clear` (1), `$iter`/`$next`/`$spread` (2/4/2, the lazy-iterator lifecycle
  ops), `$forEach` (2, `for_each_strategy` over a small mutation table). The `-with-delete` variants
  add `delete` (4) and `remove` (3), both weighted above `clear` specifically because interleaving
  them with eviction is where BUG-LRU-CACHE-1's sibling defect and the two port defects above were found.
* **Constructor alphabet:** capacity `1..=6` and nothing else — deliberately small relative to the
  op-count ceiling (`program_len` widened to `1..300`), so a generated program cycles the ring many
  times over at every capacity in range. The warning here is explicit: a campaign whose
  capacity is large relative to its op count proves only that a map stores things.
* **Key pool:** ten keys mixing `Str`, `Int`, `Bool`, `Null` and `Undefined` (mirroring `JsKey`'s
  primitive shapes), including the one collision unique to this family — `Int(0)` and `Str("0")` are
  the same key for the object-backed pair (`ToPropertyKey` coerces both to `"0"`) and two different
  keys for the `Map`-backed pair (SameValueZero never conflates them) — and four JS-falsy values
  (`Int(0)`, `Bool(false)`, `Null`, `Undefined`), which is what made BUG-LRU-CACHE-1 reachable on the third
  generated case.
* **Observable state, compared after every op:** `capacity`/`size`/`head`/`tail` always; the
  object-backed pair's full `items` (every live key's pointer, an order-independent JSON object —
  see DIV-LRU-CACHE-5 for why the `Map`-backed pair's own `items` is left out).

**How often eviction actually fired** — measured directly, not inferred from the weights, by
`grammar_self_check` (`crates/difffuzz/src/modules/lru_cache.rs`, no oracle, no `node`): over 400
generated programs (up to 300 ops each),

```
lru-cache grammar (no delete): 60,220 ops, 5,760 evictions (9.6% of ops)
lru-cache-with-delete grammar: 63,235 ops, 3,176 evictions (5.0%), 1,329 successful deletes (2.1%)
```

Both self-check tests assert a floor on these figures (20:1 and 40:1/100:1 respectively) so a future
change to the weights that regresses back toward "write-only" fails loudly rather than silently.

## Falsification record (gate 6)

**The assertion the sabotage had to break was named first:** the last `assert_eq!` in
`structures::lru_cache::tests::reproduces_the_upstream_walkthrough` —
`entries(&cache) == vec![("four", 4), ("two", 5), ("three", 3)]`, which depends on `cache.get("four")`
having promoted `"four"` to the front a few lines earlier — and the equivalent upstream assertion,
`test/lru-cache.js`'s own `Array.from(cache.entries())` check right after `cache.get('four')`.

**The sabotage:** for an LRU the sharp target is recency, not storage, so `LruCache::get` had its
`self.splay_on_top(pointer);` call commented out — the read still returns the right value, it just
stops moving anything.

**Confirmed red, in all three places a promotion-on-read failure could be caught:**

* The named Rust assertion: `left: [("two", 5), ("four", 4), ("three", 3)]` vs.
  `right: [("four", 4), ("two", 5), ("three", 3)]` — `"four"` never moved.
* The original suite: `72 passing, 16 failing` (down from 88 passing) — every block that reads an
  entry and then asserts on order went red.
* **The differential fuzzer noticed too**, and fast: `target/release/difffuzz --module lru-cache
  --seed 42 --cases 200` found a divergence in **74 operations, 0.4 seconds**, minimised to nine
  operations:

  ```
  divergence in observable state after op #24: get(1)
    head:
      port:     1
      upstream: 0
    tail:
      port:     0
      upstream: 1
  ```

  `head`/`tail` disagreeing immediately after a `get` is exactly what a broken promotion looks like
  from the outside — the sharpest possible confirmation that the grammar's heavy `get` weighting is
  pulling its weight.

**Reverted; confirmed green again** at all three: the named assertion passes, `88 passing` on the
original suite, and a 200-case replay of the same seed comes back `0 divergences`.

## Bench table

`bench/results.json` → `modules["lru-cache"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`get`/`has` (50/25/25) over a 1e6-key domain, capacity 20% of the
domain (`bench/runner/src/lru_cache.rs::capacity_for`), xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **32.4** | 58.1 | 1.8× faster |
| p99 ns/op | **112.9** | 287.7 | 2.5× faster |
| RSS delta MB | **27.8** | 111.8 | |
| structure-only RSS delta MB | **1.2** | 9.8 | |
| startup ms | **0.6** | 17.1 | 28× (reported separately; not throughput) |

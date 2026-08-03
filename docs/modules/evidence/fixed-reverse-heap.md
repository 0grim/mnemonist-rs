# fixed-reverse-heap — evidence

Gate artifacts for `docs/modules/fixed-reverse-heap.md`: test-to-gap table, fuzz grammar, full
falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/fixed_reverse_heap.rs` — 8 tests:

| Test | Closes gap |
|---|---|
| `keeps_only_the_smallest_items`, `to_array_leaves_the_heap_intact`, `a_reverse_comparator_keeps_the_largest_items` | the upstream blocks, as a baseline |
| `consume_below_capacity_returns_only_the_live_prefix` | 15 |
| `peek_after_clear_answers_a_discarded_item` | 5, 7 — asserts the stale value *is* returned, so a future "tidy-up" of `clear` fails here |
| `consume_after_clear_ignores_the_stale_contents` | 5 — the other half, and why the bug is latent |
| `a_capacity_of_zero_silently_accepts_nothing` | 1 |
| `a_comparator_may_re_enter_and_push` | 8 — and the assertion is that the array grows past `capacity` while `size` does not |

## Fuzz grammar

* **Op alphabet:** `push(v)` (weight 7) · `peek()` (2) · `clear()` (2) · `toArray()` (2) ·
  `consume()` (1).
* **Constructor alphabet:** `ArrayClass` (see the exclusion below), one of five comparator
  factories, and a **generated capacity in `0..5`, zero included** — because a capacity of `0` is
  accepted upstream (BUG-FIXED-REVERSE-HEAP-1) and a grammar that only generated sensible capacities would never have
  visited that branch.
* **The comparator factories are `heap`'s**, minus `clearer`: this structure's `clear` sets `size`
  and does not rebind `items`, so it is not the rebinding case `heap` uses that factory for.
  `clear` is an ordinary op in the alphabet instead, which reaches BUG-FIXED-REVERSE-HEAP-2 directly.
* **Both constructor arities are generated** — 30% of programs omit the comparator, which is
  upstream's `arguments.length === 2` form.
* **Observable state, compared after every op:** `size`, `capacity` and `items`. `items` is
  `capacity` slots long from construction and keeps its contents through a `clear()`, which is what
  makes BUG-FIXED-REVERSE-HEAP-2 visible in the state rather than only through a `peek`.

## Falsification record

### Fuzzer falsification

Sabotage: `FixedReverseHeap::clear` blanking the backing array as well as resetting `size` — that
is, *fixing* BUG-FIXED-REVERSE-HEAP-2. Chosen because it is the most plausible "obvious improvement" anyone would make
to this file, and because it makes the port strictly more correct than upstream and therefore
wrong.

* **All 7 assertions of `test/fixed-reverse-heap.js` still passed under it.**
* The fuzzer found it in **84 operations** and shrank it to a `clear()` on a capacity-1 heap:

  ```js
  var s = new FixedReverseHeap(Array, ascending, 1);
  s.toArray(); s.toArray();
  s.clear();          // port items [], upstream items [undefined]
  ```

* `tests/boundary/heap.js` caught it too, by name.

Reverted; the seed is committed with a provenance header in
`crates/difffuzz/proptest-regressions/fixed-reverse-heap.txt`.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:** `should be possible to consume the
heap.` — `assert.deepStrictEqual(heap.consume(), [1, 4, 8])` at
`test/fixed-reverse-heap.js:31`. Chosen because `consume` is the only algorithm this module owns
outright: `push` delegates to `Heap.siftDown`/`Heap.replace`, so a sabotage there would have proved
`heap`'s code was running rather than this module's.

**The sabotage:** `consume`'s backwards fill, `array.set(i, last_item)` → `array.set(l - 1 - i, …)`.
One index, in the loop that is the entire reason this structure stores its elements reversed.

**Confirmed red**, and red in the named place: `2 passing, 5 failing`, the named assertion failing
with `[8, 4, 1]` against `[1, 4, 8]`. Reverted; **confirmed green again**: `7 passing`.

## Bench table

`bench/results.json` → `modules["fixed-reverse-heap"]`. Methodology: `bench/methodology.md`.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`peek` (75/25), default numeric comparator, capacity `size / 2`,
xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **14.50** | 15.07 | 1.04× faster |
| p99 ns/op | 115.94 | **96.99** | upstream 1.20× faster |
| min ns/op | 13.075 | **13.055** | essentially tied (<0.2%) |
| RSS delta MB | **14.0** | 35.7 | |
| structure-only RSS delta MB | **1.3** | 9.8 | |
| startup ms | **0.6** | 15.4 | 26× (reported separately; not throughput) |

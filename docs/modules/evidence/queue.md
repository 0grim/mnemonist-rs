# queue — evidence

Gate artifacts for `docs/modules/queue.md`: test-to-gap table, fuzz grammar, full falsification
record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/queue.rs` — 15 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all eleven upstream blocks, as a baseline |
| `enqueue_returns_the_new_size` | 10 |
| `the_compaction_fires_when_the_dead_prefix_reaches_half_the_array` | 1 — pinned index by index, `offset` and `items.length` after every dequeue |
| `a_one_element_queue_compacts_immediately` | 1 — the degenerate end (`1 * 2 >= 1`) |
| `interleaved_enqueue_and_dequeue_stay_in_order` | 1 — FIFO across compactions, which is the way the offset arithmetic actually breaks |
| `dequeueing_an_empty_queue_moves_nothing` | — |
| `cursors_do_not_restart_but_the_queue_can_be_walked_again` | DIV-STACK-1/DIV-STACK-2 |
| `an_enqueue_during_iteration_is_visible_to_the_cursor` | 5 — the live end |
| `a_finished_cursor_resumes_when_the_queue_grows` | 6 — nothing latches |
| `a_compaction_detaches_an_open_cursor_onto_the_old_array` | 2, 3 |
| `a_cursor_freezes_the_offset_it_was_opened_with` | 2, 3 |
| `clear_leaves_an_open_cursor_walking_the_old_array` | 4 |
| `for_each_reads_the_live_array_where_the_cursor_reads_the_capture` | 7 |
| `an_empty_queue_iterates_zero_times` | — |
| `from_iter_accepts_any_iterator` | DIV-QUEUE-1 |

## Fuzz grammar

* **Op alphabet:** `enqueue(v)` (weight 6) · `dequeue()` (4) · `peek()` (2) · `clear()` (1) ·
  `$iter("values")` (2) · `$next()` (4) · `$spread()` (1).
* **Observable state, compared after every op:** `size`, **`offset`**, **`items`** and `toArray()`.
  The middle two are the point: `toArray()` alone cannot tell a compacted queue from an
  uncompacted one holding the same elements, so a port could get the entire schedule wrong for a
  whole program with nothing noticing.
* **Values:** `0..48`.
* **Program length:** 1..200 ops.
* **Deliberately excluded: nothing.**

## Falsification record

### Fuzzer falsification

Sabotage: `Sequence::limit` returning the frozen length — that is, giving the queue the stack's
cursor, which is exactly what a port that generalised one cursor shape across modules would
produce. Caught in **99 cases (0.1 s)**, shrunk from 200 ops to **three**, which is the smallest
program that can express it:

```js
var s = new Queue();
var it = s.values();     // opened on an empty queue
s.enqueue(0);
it.next();               // port {done: true}, upstream {value: 0}
```

Note what that repro also demonstrates: the cursor had already run off the end of an empty queue,
and obliterator's `Iterator` has no done flag, so it resumes. Reverted; the seed is committed with
a provenance header in `crates/difffuzz/proptest-regressions/queue.txt`.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:**
`should be possible to dequeue` — `assert.strictEqual(queue.dequeue(), 2)`, at `test/queue.js:52`.
Chosen because it is the only assertion in the file whose value depends on the offset having
advanced; everything else in the suite is satisfied by a queue that ignores `offset` entirely.

**The sabotage:** `Queue::dequeue` reading `items[0]` instead of `items[offset]` — forgetting the
offset, which is the most plausible single mistake in this file.

**Confirmed red**, and red in exactly the named place: `10 passing, 1 failing`, with
`actual 1, expected 2` at `test/queue.js:52`. Reverted; **confirmed green again**: `11 passing`.

Worth recording what this sabotage does *not* break, because it bounds what gate 6 proves here:
the compaction schedule itself is invisible to the original suite. A sabotage of
`++offset * 2 >= items.length` — say, `offset >= items.length` — leaves all 11 blocks green. That
is not a weakness in the choice of sabotage; it is the measurement of how thin the upstream
coverage is, and it is why `offset` and `items` are both in the fuzzer's observation set.

## Bench table

`bench/results.json` → `modules["queue"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `enqueue`/`peek`/`dequeue` (50/25/25), value magnitude 1e6, xorshift32
seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **4.7** | 8.5 | 1.8× faster |
| p99 ns/op | **7.6** | 94.2 | 12.4× faster |
| min ns/op | **4.3** | 6.4 | 1.5× faster |
| RSS delta MB | **10.1** | 62.7 | |
| structure-only RSS delta MB | **1.3** | 9.7 | |
| startup ms | **0.6** | 16.1 | 27× (reported separately; not throughput) |

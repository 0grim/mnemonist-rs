# circular-buffer — evidence

Gate artifacts for `docs/modules/circular-buffer.md`: test-to-gap table, fuzz grammar, full
falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/circular_buffer.rs` — 13 tests (11 substantive):

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all eighteen upstream blocks, `#223` included with its issue number |
| `a_push_that_overwrites_returns_the_unchanged_size` | 1 — the return sequence `1, 2, 3, 3, 3` and the `start` walk `0, 0, 0, 1, 2`, pinned against Node |
| `an_unshift_that_overwrites_returns_the_unchanged_size` | 2 |
| `a_full_push_overwrites_the_slot_start_is_on` | 5 — asserts the array *and* `start` after one overwriting push, which is what a reversed store/test pair would break |
| `push_and_unshift_overwrite_opposite_ends` | 3 |
| `get_is_bounded_by_the_capacity_here_too` | 9 |
| `from_bypasses_the_overwriting_that_this_class_exists_for` | 6 |
| `many_wraps_still_walk_in_order` | — thirteen pushes on a capacity-4 ring |
| `a_capacity_of_one_replaces_in_place` | 4 |
| `an_overwriting_push_is_visible_to_an_open_cursor` | 13 — the sharpest hybrid-capture case of the three fixed-capacity modules: the length is frozen at construction, the elements are not |
| `cursors_do_not_restart_but_the_buffer_can_be_walked_again` | 13 |

## Fuzz grammar

* **Op alphabet:** `push(v)` (weight 5) · `unshift(v)` (3) · `pop()` (2) · `shift()` (2) ·
  `peekFirst()` (1) · `peekLast()` (1) · `get(i)` (2) · `clear()` (1) · `$iter("values")` (2) ·
  `$next()` (4) · `$spread()` (1) · `$forEach(mutation, at)` (3).
* **Observable state, compared after every op:** `size`, `capacity`, `start`, `items`, `toArray()`.
* **Neither insert can throw here**, so a generated program never stops growing and spends almost
  all of its length *past* the capacity, where every insert overwrites and `start` walks. With
  capacities of 1..=8 and 200-op programs, a program wraps the ring tens of times.
* **The return value of every insert is compared**, which is what pins gaps 1 and 2.
* **Both backing classes**, `get` indices 0..=11, values to 320.
* **Deliberately excluded:** the same three as `fixed-deque` — `from` (a static), `forEach`'s
  `scope` (DIV-FIXED-STACK-3), and a negative `get` index (core takes a `usize`; covered by differential probes).

## Falsification record

### Fuzzer falsification

Sabotage: `push` returning `size + 1` when it overwrites — reading upstream's `return this.size` as
`return ++this.size`, which is what the non-overwriting branch two lines below actually does.
Caught in **497 cases (0.4 s)**, shrunk to three lines:

```js
var s = new CircularBuffer(Array, 1);
s.push(0); s.push(0);      // port 2, upstream 1
```

Reverted; the seed is committed with provenance in
`crates/difffuzz/proptest-regressions/circular-buffer.txt`. Note what this sabotage would have
survived: the entire original suite, which never asserts an overwriting insert's return value.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:**
`should be possible to wrap buffer around when pushing.` —
`assert.deepStrictEqual(buffer.toArray(), [2, 3, 4])`, at `test/circular-buffer.js:46`. Chosen
because it is the first assertion in the file that reaches an *overwriting* push, which is the only
code this module adds to `FixedDeque`.

**The sabotage:** the overwriting branch of `push` no longer advancing `start` — it still writes the
slot and still returns the unchanged size, so the buffer keeps its capacity and its size, and only
the oldest element is wrong.

**Confirmed red**, and red in precisely the named place: `16 passing, 2 failing`, the first failure
being that assertion with `actual` `[4, 2, 3]` against `expected` `[2, 3, 4]`. Reverted;
**confirmed green again**: `18 passing`.

## Bench table

`bench/results.json` → `modules["circular-buffer"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`peekLast`/`pop` (50/25/25), capacity 10,000 against 1e6 ops,
xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **4.9** | 6.2 | 1.3× faster |
| p99 ns/op | **8.5** | 10.4 | 1.2× faster |
| min ns/op | **4.3** | 5.8 | 1.3× faster |
| RSS delta MB | **6.1** | 21.0 | |
| structure-only RSS delta MB | **0.1** | 0.3 | |
| startup ms | **0.6** | 16.1 | 27× (reported separately; not throughput) |

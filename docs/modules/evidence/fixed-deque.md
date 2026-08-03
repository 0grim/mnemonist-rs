# fixed-deque — evidence

Gate artifacts for `docs/modules/fixed-deque.md`: test-to-gap table, fuzz grammar, full
falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/fixed_deque.rs` — 17 tests (16 substantive plus `Debug`):

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all sixteen upstream blocks |
| `get_is_bounded_by_the_capacity_and_returns_debris_below_it` | 1 — for both backing classes, so the `undefined` and the class-zero halves are both pinned |
| `get_past_the_size_wraps_around_to_the_shifted_element` | 1, 4 — the debris is the *wrapped* slot, not a stale tail |
| `removals_leave_the_elements_in_place` | 8 |
| `a_refused_insert_leaves_the_deque_untouched_and_names_its_method` | — the two messages differ by method name |
| `an_oversized_from_walks_the_ring_more_than_once` | 10 — `[1,2,1,2]`, and the single conditional subtraction in `pop` |
| `an_oversized_from_is_truncated_by_a_typed_class` | 10 |
| `cursors_do_not_restart_but_the_deque_can_be_walked_again` | 15 |
| `a_push_during_iteration_is_not_visible_to_the_cursor` | 16 |
| `a_shift_during_iteration_does_not_move_the_cursor` | 6, 16 — the frozen-`start` half, which is this module's sharpest cursor behaviour |
| `an_overwrite_ahead_of_the_cursor_is_visible` | 16 |
| `a_wrapped_deque_walks_front_to_back` | 4, 18 |
| `unshift_from_the_zero_start_wraps_to_the_last_slot` | — the `start === 0` wrap, reached here from an *empty* deque rather than a full one |
| `a_capacity_of_one_and_an_empty_deque_both_behave` | 17 |
| `from_array_like_accepts_any_iterator` | D-03 |
| `error_text_is_upstreams` | — the message constants, verbatim |

## Fuzz grammar

* **Op alphabet:** `push(v)` (weight 5) · `unshift(v)` (3) · `pop()` (2) · `shift()` (2) ·
  `peekFirst()` (1) · `peekLast()` (1) · **`get(i)` (2)** · `clear()` (1) · `$iter("values")` (2) ·
  `$next()` (4) · `$spread()` (1) · `$forEach(mutation, at)` (3).
* **Observable state, compared after every op:** `size`, `capacity`, **`start`**, `items`,
  `toArray()`. `start` is in the set because the upstream file asserts on it and because it is the
  one number a wrong wrap moves first.
* **`get` indices run 0..=11 against capacities of 1..=8**, so both clauses of B-62's guard are
  exercised constantly: past the size (debris) and past the capacity (the guard that fires).
* **Both backing classes**, capacities 1..=8, values to 320.
* **Deliberately excluded:** `from` (a static cannot appear in an op sequence; covered by the
  original test and the differential probes), `forEach`'s `scope` (D-61), and a **negative** `get`
  index — the fuzzer drives `mnemonist-core`, whose `get` takes a `usize`, and the negative path is
  the bridge's. It is covered by four differential probes instead, and this exclusion is the reason
  they are recorded above rather than left as scratch work.

## Falsification record

### Fuzzer falsification

Sabotage: `get`'s guard changed from `index >= self.capacity` to `index >= self.size` — the
"obvious correction" of B-62, and the change any reader who has not checked upstream would make.
Caught in **823 cases (1.2 s)**, shrunk to five lines:

```js
var s = new FixedDeque(Array, 2);
s.push(0); s.push(0);
s.forEach(function (v, i) { if (i === 0) s.pop(); });
s.get(1);        // port undefined, upstream 0
```

Reverted; the seed is committed with provenance in
`crates/difffuzz/proptest-regressions/fixed-deque.txt`.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:**
`should be possible to unshift the deque.` — `assert.strictEqual(deque.start, 3)`, at
`test/fixed-deque.js:151`. Chosen because `start` is the one piece of internal geometry the file
inspects directly, so it is the assertion that most specifically exercises the ring rather than its
results.

**The sabotage:** `previous_start()` returning `self.start.saturating_sub(1)` instead of wrapping to
`capacity - 1` when `start` is zero — dropping the one line that makes `unshift` a ring operation
rather than a bounded one.

**Confirmed red**, and red in precisely the named place: `13 passing, 3 failing`, and the second
failure is that assertion, at `test/fixed-deque.js:151`, with `actual` `0` against `expected` `3`.
The other two are `should be possible to pop the deque.` and `should handle tricky situations.`,
which both reach `unshift` on a deque whose `start` is zero. Reverted; **confirmed green again**:
`16 passing`.

## Bench table

`bench/results.json` → `modules["fixed-deque"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`peekLast`/`pop` (50/25/25), capacity 10,000 against 1e6 ops,
xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **4.6** | 7.5 | 1.6× faster |
| p99 ns/op | **5.9** | 13.4 | 2.3× faster |
| min ns/op | **4.1** | 7.0 | 1.7× faster |
| RSS delta MB | **6.2** | 19.3 | |
| structure-only RSS delta MB | **0.1** | 0.2 | |
| startup ms | **0.6** | 16.7 | 28× (reported separately; not throughput) |

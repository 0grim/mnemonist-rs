# stack — evidence

Gate artifacts for `docs/modules/stack.md`: test-to-gap table, fuzz grammar, full falsification
record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/stack.rs` — 14 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all eleven upstream blocks, as a baseline |
| `push_returns_the_new_size` | 1 |
| `popping_an_empty_stack_does_not_move_the_size` | 2 |
| `size_and_the_backing_length_track_each_other` | the two quantities upstream keeps separate |
| `cursors_do_not_restart_but_the_stack_can_be_walked_again` | 8, 9 — both levels of DIV-STACK-2 in one test |
| `a_push_during_iteration_is_not_visible_to_the_cursor` | 6 |
| `a_pop_during_iteration_opens_a_gap_at_the_top_of_the_walk` | 5 — the `undefined` window |
| `clear_rebinds_the_array_and_leaves_an_open_cursor_untouched` | 4 — the one a `Vec<T>` cannot express |
| `a_cursor_detached_by_clear_never_sees_the_new_array` | 4, extended past the rebinding |
| `for_each_reads_the_live_array_where_the_cursor_reads_the_capture` | 7 — the third behaviour |
| `peek_is_a_pure_read` | — |
| `an_empty_stack_iterates_zero_times` | — |
| `from_iter_accepts_any_iterator` | DIV-QUEUE-1: core takes any `IntoIterator` |
| `duplicates_are_kept` | — |

`crates/mnemonist-core/src/cursor/mod.rs` — 3 new tests for `Sequence::limit`
(`a_live_limit_sees_growth_that_a_frozen_one_does_not`,
`a_live_limit_resumes_after_reporting_done`,
`a_live_limit_that_shrinks_ends_the_walk_without_a_gap`), bringing that file to 16.

## Fuzz grammar

* **Op alphabet:** `push(v)` (weight 6) · `pop()` (3) · `peek()` (2) · `clear()` (2) ·
  `$iter("values")` (2) · `$next()` (4) · `$spread()` (1).
* **Observable state, compared after every op:** `size`, **`items`** and `toArray()`. `items` is a
  public property upstream, and observing it directly is what makes the array-rebinding checkable
  without waiting for a cursor to notice it. Comparing `size` *and* `items` separately is how a port
  that silently unified the two would be caught.
* **Values:** `0..48`, small enough that duplicates are frequent — a stack is not a set.
* **Program length:** 1..200 ops.
* **Deliberately excluded: nothing.** Every method `stack.js` exposes is in the alphabet or the
  observation set, except `inspect`, which is not ported.

## Falsification record

### Fuzzer falsification

Sabotage: `clear()` emptying the backing array in place instead of rebinding it — which is the only
thing a `Vec<T>` can do, and which makes `clear()` indistinguishable from popping everything. Caught
in **101 cases (0.1 s)**, shrunk from 200 ops to four:

```js
var s = new Stack();
s.push(0);
var it = s.values();
s.clear();
it.next();      // port {value: undefined}, upstream {value: 0}
```

Reverted; the seed is committed with a provenance header in
`crates/difffuzz/proptest-regressions/stack.txt`, where proptest replays it before any novel case
on every subsequent run.

### Falsification of the port (gate 6)

Gate 6 asks that sabotaging the core turns the **original mocha suite** red, proving it exercises
Rust rather than a JS fallback.

**The assertion the sabotage had to break was named first:**
`should be possible to create a values iterator` —
`assert.strictEqual(iterator.next().value, 3)`, at `test/stack.js:102`. Chosen because it is the
first assertion in the file that reaches the reversed cursor, which is the arithmetic most likely
to be mis-ported.

**The sabotage:** `Sequence::slot` for `Stack` walking **forward** (`items[ordinal]`) instead of
newest-first (`items[l - ordinal - 1]`).

**Confirmed red**, and red in the named place: `8 passing, 3 failing`, the failures being the
values iterator, the entries iterator and the `for…of` block. Note what stayed green — `toArray`,
which does its own reversal — so the sabotage isolated the cursor rather than the module.
Reverted; **confirmed green again**: `11 passing`.

**A second, separate falsification, of the dispatch.** Sabotage: an off-by-one in branch 1
(`i + 1 < l`, dropping the last element of every indexed sequence). Named assertion:
`should be possible to create a stack from an arbitrary iterable`,
`assert.deepStrictEqual(stack.toArray(), [3, 2, 1])` at `test/stack.js:88`. **Confirmed red**,
`13 passing, 9 failing` across both stack and queue. Reverted, green again.

**A third attempt that stayed green, and what it proved.** Deleting branch 1's `[object Arguments]`
clause left all 22 assertions passing — see the document's "Bugs this found" and the log for the
withdrawn claim this disproved. A falsification that cannot fail is just a second green light; this
one was informative precisely because it failed to fail.

## Bench table

`bench/results.json` → `modules["stack"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`peek`/`pop` (50/25/25), value magnitude 1e6, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **4.6** | 7.3 | 1.6× faster |
| p99 ns/op | **6.8** | 30.3 | 4.5× faster |
| min ns/op | **4.2** | 5.2 | 1.2× faster |
| RSS delta MB | **7.9** | 44.9 | |
| structure-only RSS delta MB | **1.3** | 9.9 | |
| startup ms | **0.6** | 15.7 | 26× (reported separately; not throughput) |

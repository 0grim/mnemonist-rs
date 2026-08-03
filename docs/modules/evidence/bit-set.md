# bit-set — evidence

Gate artifacts for `docs/modules/bit-set.md`: test-to-gap tables, fuzz grammar, full falsification
record, full benchmark tables.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/bits.rs` — 13 tests:

| Test | Closes gap |
|---|---|
| `a_no_op_reset_decrements_size_when_the_top_bit_of_the_word_is_set` | 1 — B-17, including `size == -2` |
| `a_no_op_reset_is_harmless_while_the_top_bit_is_clear` | 1 — the control that explains why upstream never noticed |
| `rank_returns_zero_whenever_the_size_counter_is_zero` | 2 — the propagation into `rank` |
| `select_loses_thirty_two_positions_per_skipped_word` | 5 — B-18 |
| `select_answers_minus_one_a_position_or_undefined` | 6, 7, 8 — all three return shapes |
| `out_of_range_indices_are_inert_rather_than_corrupting` | 9 |
| `a_bit_past_length_but_inside_the_word_is_counted_yet_invisible` | 9 — B-23 |
| `a_cursor_keeps_the_array_it_was_opened_over` | 11, 12 |
| `writes_ahead_of_the_cursor_are_visible_but_not_within_the_current_word` | 12 — the word-granularity half |
| `a_walk_is_not_restartable` | 13 |
| `entries_pair_each_bit_with_its_ordinal` | — |
| `the_last_word_is_full_when_the_length_is_a_multiple_of_thirty_two` | — B-22, the `\|\| 32` misfire |
| `cloning_copies_the_backing_store` | — a port-side invariant, not an upstream one |

`crates/mnemonist-core/src/structures/bit_set.rs` — 15 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all twelve upstream blocks, as a baseline |
| `a_reset_that_clears_nothing_still_decrements_size` | 1 |
| `a_corrupted_size_makes_rank_lie` | 2 — and that `select` bails on the same counter |
| `the_same_reset_is_harmless_while_the_words_top_bit_is_clear` | 1 |
| `select_loses_a_word_of_positions_for_every_empty_word_it_skips` | 5 |
| `select_off_the_end_is_undefined_and_out_of_range_is_minus_one` | 6, 7, 8 |
| `indices_past_the_backing_array_are_inert` | 9, 10 |
| `a_bit_between_length_and_the_end_of_its_word_is_counted_but_unreachable` | 9 |
| `clear_detaches_an_open_cursor_from_the_words_it_zeroes` | 11, 12 |
| `clear_resets_size_and_the_set_is_reusable` | 3, 11 |
| `repeated_sets_and_resets_are_idempotent_in_size` | 4 |
| `a_zero_length_set_holds_and_yields_nothing` | 14 |
| `iteration_yields_exactly_length_bits` | 14 — nine lengths across both word boundaries |
| `cursors_do_not_restart_but_the_set_can_be_walked_again` | 13 — both levels of D-07 |
| `writes_during_iteration_are_visible_only_beyond_the_current_word` | 12 |
| `rank_saturates_past_the_end_rather_than_reading_off_it` | 9 |
| `allocates_one_word_per_thirty_two_bits_rounded_up` | — |

## Fuzz grammar

* **Op alphabet:** `set(i)` (weight 4) · `set(i, 0)` (2) · **`reset(i)` (3)** · `flip(i)` (2) ·
  `get(i)` (2) · `test(i)` (1) · `rank(i)` (2) · `select(r)` (2) · `clear()` (1) ·
  `$iter("values")` (1) · `$iter("entries")` (1) · `$next()` (3) · `$spread()` (1).
* **Observable state, compared after every op:** `size`, `length`, **`array`** and `toJSON()`.
  `array` is the point — `size` alone would agree in plenty of programs where the words had already
  diverged.
* **Lengths:** `0..=400`, thirteen words. Zero is included because `new BitSet(0)` allocates nothing
  and is the degenerate end of every guard; 400 is sparse enough that empty words between set bits
  are routine, which is what B-18 needs.
* **Indices:** `0..length + 64`, so a steady fraction land in B-23's band and beyond it.
* **Program length:** 1..200 ops.
* **Deliberately excluded: nothing.** Out-of-range indices, negative-adjacent behaviour and cursor
  interleaving are all generated. `reset` is weighted **up** rather than down, because B-17 only
  misfires on a bit that is already clear, and a low weight would make that rare rather than routine.

`$iter` alternates between `values` and `entries`: they share an implementation in this port and are
separate closures upstream, so fuzzing only one would leave the other unchecked. `clear()` is in the
alphabet *because* it interacts with an open cursor.

## Falsification record

### Fuzzer falsification

Sabotage: `reset` given the `>>> 0` upstream forgot — B-17 *fixed*, which is the single most
plausible thing a future cleanup does to this file. Caught in **1,325 cases (2.0 s)** and shrunk from
200 ops to **two**:

```js
var s = new BitSet(1);
s.set(31);      // inside word 0, past length 1 -- accepted (B-23)
s.reset(0);     // clears nothing; upstream decrements anyway
// port size 1, upstream size 0
```

Worth noting what that two-op program shows: **B-23 and B-17 compound.** The `set(31)` is only
possible because an index past `length` but inside the last word is accepted, and it is what puts
bit 31 into the word, which is the precondition for `reset`'s signed comparison to misfire. Neither
defect alone reaches the state, and no upstream test passes an index past `length` at all. Reverted;
the seed is committed with provenance in `crates/difffuzz/proptest-regressions/bit-set.txt`.

### Falsification of the port (gate 6)

**Named first:** `length divisible by 32 iteration, issue #117.` →
`assert.strictEqual(counter, set.length)` at `test/bit-set.js:178`. Chosen because it is the only
assertion in the file that depends on the last-word width calculation, and because upstream issue
#117 exists precisely because that calculation was once wrong.

**The sabotage:** `length % 32 || 32` reduced to `length % 32` — "fixing" a guard that genuinely
looks like a bug, and which B-22 shows *is* one at length 0.

**Confirmed red**, at exactly the named line: `11 passing, 1 failing`, `32 !== 64` at
`test/bit-set.js:178`. Reverted; **confirmed green again**: 12 passing.

Neither of this module's headline defects could have served as the sabotage. "Fixing" `reset`'s
missing `>>> 0` leaves the suite green, because every `reset` in the file clears a bit that is
actually set. "Fixing" `select` leaves it green too, because its `select` test uses a length of 11
and never skips a word.

## Bench tables

`bench/results.json` → `modules["bit-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`reset`/`get`/`test` (50/25/25) over capacity 1e6, xorshift32
seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 8.87 | **7.94** | 1.12× slower |
| p99 ns/op | 16.30 | **14.87** | 1.10× slower |
| RSS delta MB | **6.1** | 17.6 | |
| structure-only RSS delta MB | **1.3** | 9.8 | |
| startup ms | **0.6** | 17.9 | 30× (reported separately; not throughput) |

Note: these figures predate the `split`/`to_int32` fast-path fix described in the document and log;
`bench/results.json` is the current source of truth per `bench/methodology.md` and reflects the
post-fix state where it has been re-run. See the log for the before/after probe numbers.

**RefCell + `to_int32` probe** (`bench-runner --bit-set-probe`), three variants of the identical op
stream:

| variant | p50 ns/op |
|---|---|
| wrapped `BitSet` (`RefCell` + `Words` + `to_int32`) | 8.456 |
| bare `Vec<u32>` + `to_int32`, no `RefCell` | 4.489 |
| bare `Vec<u32>`, plain `usize`, no `RefCell`, no `to_int32` | **3.026** |

The isolated gap (5.43 ns/op) splits roughly 73%/27% between the `RefCell`/`Words` wrapper layer
(3.97 ns/op) and the `to_int32` conversion alone (1.46 ns/op). Both bare variants beat upstream's own
published p50 (7.935 ns) outright.

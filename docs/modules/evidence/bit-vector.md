# bit-vector — evidence

Gate artifacts for `docs/modules/bit-vector.md`: test-to-gap table, fuzz grammar, full
falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/bit_vector.rs` — 19 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all twenty-one upstream blocks, as a baseline |
| `pop_leaves_size_and_the_bits_behind` | 1, 2, 3, 4 — BUG-BIT-VECTOR-1, the whole sequence with the assertion upstream skipped |
| `pushing_true_onto_an_already_set_slot_counts_it_twice` | 3 |
| `set_at_length_writes_a_bit_that_length_does_not_cover` | 5, 6 |
| `get_is_undefined_only_strictly_past_the_length` | 6 |
| `a_zero_length_vector_with_capacity_still_iterates_a_whole_word` | 7 — BUG-BIT-VECTOR-2 |
| `a_length_that_exactly_fills_its_words_walks_all_of_them` | 7 — the same misfire where it is also correct |
| `reallocate_clamps_length_even_when_the_capacity_does_not_change` | 10 |
| `reallocate_to_zero_drops_the_array_and_the_length` | 8 |
| `a_shrinking_reallocate_discards_the_words_above_the_cut` | 9 |
| `a_zero_override_falls_back_to_the_current_capacity` | 15 |
| `a_policy_can_fail_three_ways` | 12, 13 — and our own refusal |
| `a_non_integer_policy_result_is_rounded_up_to_a_word` | 14 |
| `grow_loops_the_policy_until_it_covers_the_target` | 11 — seven applications |
| `to_json_takes_one_word_past_the_length_clamped_by_the_array` | — five lengths, including the clamped case |
| `reallocate_detaches_an_open_cursor` | 20 |
| `growth_during_iteration_is_invisible_to_an_open_cursor` | 20 |
| `cursors_do_not_restart_but_the_vector_can_be_walked_again` | 20 |
| `an_initial_length_of_thirty_derives_a_capacity_of_thirty_two` | — the `initialLength \|\| initialCapacity` quirk |
| `inherits_the_reset_and_select_defects_verbatim` | 16, 17 — re-verified against `BitVector`, not inferred |
| `indices_past_the_backing_array_are_inert` | 19 |

Plus the 13 tests on the shared `bits.rs`, listed in `docs/modules/evidence/bit-set.md`.

## Fuzz grammar

* **Op alphabet:** `set(i)` (3) · `set(i, 0)` (2) · `reset(i)` (3) · `flip(i)` (2) · `get(i)` (2) ·
  `test(i)` (1) · `rank(i)` (2) · `select(r)` (2) · **`push(1)` (3) · `push(0)` (3) · `pop()` (3)** ·
  `resize(l)` (2) · `reallocate(c)` (2) · `grow(c)` (1) · `grow()` (1) · `$iter("values")` (1) ·
  `$iter("entries")` (1) · `$next()` (3) · `$spread()` (1).
* **Observable state, compared after every op:** `size`, `length`, `capacity`, **`array`** and
  `toJSON()`.
* **Initial lengths:** `0..=200`. **Indices:** `0..length + 64`. **Extents:** `0..512`.
* **Program length:** 1..200 ops.
* `push(1)` and `push(0)` are separate ops on purpose: only the former touches `size` and only the
  latter leaves a stale bit, and BUG-BIT-VECTOR-1 needs both interleaved with `pop`.
* `set` is the only op in this grammar that throws, and its message is compared in full through the
  `{"$throw": …}` encoding added for `hashed-array-tree`.
* **Deliberately excluded: custom growth policies.** Upstream's policy is a JS function and a
  generated program is JSON. The default policy is therefore the only one fuzzed — and since the
  default is strictly increasing, **both throws in `applyPolicy` are unreachable from this
  grammar**. They are covered by native tests in `mnemonist-core` instead
  (`a_policy_can_fail_three_ways`, `a_non_integer_policy_result_is_rounded_up_to_a_word`).

## Falsification record

### Fuzzer falsification

Sabotage: `pop` made to clear the bit it returns and to decrement `size` — which is what `pop` is
supposed to do, and the single most plausible repair anyone would make to this module. Caught in
**1,075 cases (1.0 s)** and shrunk from 200 ops to **two**:

```js
var s = new BitVector(0);
s.push(1);
s.pop();
// port     array [0], size 0, toJSON [0]
// upstream array [1], size 1, toJSON [1]
```

Two operations, three of the five observed fields disagreeing — and upstream's own `pop` test
performs that exact pair, then asserts only the returned value and `length`. Reverted; the seed is
committed with provenance in `crates/difffuzz/proptest-regressions/bit-vector.txt`.

### Falsification of the port (gate 6)

**Named first:** `should throw if the policy returns an irrelevant size.` →
`assert.throws(function () { vector.push(1); }, /policy/)` at `test/bit-vector.js:291`. Chosen
because the policy machinery is the best-covered part of the upstream file, so a sabotage there has
a real assertion to break.

**The sabotage:** `applyPolicy`'s `newCapacity <= this.capacity` weakened to `<`, i.e. accepting a
policy that returns exactly the current capacity. A boundary flip, not a deletion.

**Confirmed red**, at exactly the named line: `20 passing, 1 failing`, "Missing expected exception"
at `test/bit-vector.js:291`. Reverted; **confirmed green again**: 21 passing.

Neither of BUG-BIT-VECTOR-1's halves could have served as the sabotage:

* "Fixing" `push(0)` to clear its slot leaves the suite **green**, because every slot the push test
  writes over is already zero.
* "Fixing" `pop` to decrement `size` leaves it **green** too, because no assertion in the file reads
  `size` after a `pop`.

Both would have been sabotages incapable of failing — which is exactly the failure mode gate 6
exists to catch. Across this group of four modules, **five** plausible-looking sabotages were
rejected on that ground before a usable one was found.

## Bench table

`bench/results.json` → `modules["bit-vector"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`get`/`pop` (50/25/25), xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 6.42 (post-fix; 8.1–8.3 pre-fix, a tie) | **8.3** | 1.30× faster (post-fix) |
| p99 ns/op | **12.9** | 14.5 | 1.1× faster |
| min ns/op | **7.5** | 7.8 | tie |
| RSS delta MB | **6.1** | 17.8 | |
| structure-only RSS delta MB | **1.3** | 9.8 | |
| startup ms | **0.6** | 16.4 | 27× (reported separately; not throughput) |

The p50 figure moved when this module's shared `split` picked up `bit-set`'s `ToInt32` fast-path fix
— see `docs/modules/log/bit-vector.md` and `docs/modules/log/bit-set.md` for that history.

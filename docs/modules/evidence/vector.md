# vector — evidence

Gate artifacts for `docs/modules/vector.md`: test-to-gap table, fuzz grammar, full falsification
record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/vector.rs`:

| Test | Closes gap |
|---|---|
| `get_and_set_admit_index_equal_to_length` | 1 — BUG-VECTOR-1 |
| `a_full_vector_drops_the_admitted_write` | 1 — the companion case where `index == length == capacity`, so there is no capacity-region slot to admit into |
| `stale_data_from_a_pop_survives_a_growth_and_stays_reachable` | 2 — BUG-VECTOR-2 |
| `a_policy_returning_infinity_is_refused_before_any_allocation` | 5 |
| `a_policy_returning_nan_is_refused` | 5 |
| `a_policy_returning_a_negative_number_is_invalid_before_being_non_finite` | 5 — and which of the two upstream throws each non-finite value lands in |
| `a_policy_returning_not_a_number_is_invalid` | 6 |
| `shrinking_a_pointer_vector_keeps_its_current_width` | 7 |
| `a_zero_capacity_pointer_vector_starts_at_the_narrowest_width` | 8 |
| `a_pointer_vector_too_large_to_index_is_refused` | — a guard with no upstream-reachable input, kept because `Vector::pointer` needs an `Err` path to return |
| `float64_values_are_stored_exactly` | 3, 4 |
| `fixed_values_truncate_at_their_own_width` | 3 |
| `cursors_do_not_restart_but_the_vector_can_be_walked_again` | 9 |
| `a_pop_during_iteration_stays_bounded_by_the_frozen_length` | 10 |
| `an_empty_vector_pops_and_iterates_to_nothing` | 11 |
| `fills_to_capacity_without_running_off_the_end` | — a port-side invariant: growth never over-allocates |
| `the_out_of_bounds_message_names_the_actual_backing_class` | 12 |

## Fuzz grammar

* **Constructor:** one of `Uint8Array`/`Uint16Array`/`Uint32Array`/`Float64Array`, with
  `initialCapacity`/`initialLength` each in `0..48` — independently, so `initialLength >
  initialCapacity` (upstream's `capacity = Math.max(initialLength, initialCapacity)`) is routine.
* **Op alphabet:** `push(v)` (weight 5) · `pop()` (3) · `set(i, v)` (3) · `get(i)` (3) ·
  `grow(c)`/`grow()` (1 each) · `resize(l)` (2) · `reallocate(c)` (1).
* **Indices:** `0..64`, well past any generated length or capacity, so both the `index == length`
  admission and the ordinary out-of-bounds throw are common outcomes, not edge cases.
* **Values:** `0.0..70000.0` — past `255` so a `Uint8Array`/`Uint16Array` truncating store is
  exercised, and full-precision `f64`s so `Float64Array` stores are compared exactly.
* **Observable state, compared after every op:** `length`, `capacity`, and **`array`** — the whole
  backing store, capacity region included, encoded exactly as the oracle encodes a JS typed array.
  `array` is the point: without it, BUG-VECTOR-1 and BUG-VECTOR-2 are only checkable indirectly through `get`.

## Falsification record

### Falsification of the port (gate 6)

**Named first:** `vector_matches_upstream` (`crates/difffuzz/tests/differential.rs`) should go red,
because the fuzz grammar's `set`/`get` indices routinely land on `index == length`, and the
sabotage removes exactly the admission that lets that case succeed.

**The sabotage:** `Vector::set`'s bound check tightened from `if self.length < index` to
`if self.length <= index` — "fixing" the off-by-one that BUG-VECTOR-1 documents, the single most obvious
thing a future cleanup would do to this file.

**Confirmed red:** `cargo test -p difffuzz --test differential vector_matches_upstream` failed
immediately, on the campaign's very first shrunk case:

```
divergence in return value after op #1: set(22, 11908.642978421198)
  $throw:
    port:     "Vector(Float64Array).set: index out of bounds."
    upstream: <absent>
  $self:
    port:     <absent>
    upstream: true
minimal repro:
var s = new Vector(Float64Array, {"initialCapacity":12,"initialLength":17});
s.resize(22);
s.set(22, 11908.642978421198);
```

Reverted; **confirmed green again**: `vector_matches_upstream ... ok`.

### Falsification of the harness (bench, gate 10)

A ~5,000-iteration `black_box` spin was inserted into every `push` call's timed path (50% of ops),
rebuilt, and re-measured: `p50_ns_per_op` moved from 6.9 to 486.8 (55×), `p99_ns_per_op` from 22.7
to 719.8 (12×), and `regressions` went from empty to three entries — while the untouched Node side
stayed at its normal ~8.8 ns/op. Reverted and re-measured: the checksum (`249930270812`) was
identical before sabotage, during, and after revert, and the figures returned to the table below
within run-to-run noise.

## Bench table

`bench/results.json` → `modules["vector"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`get`/`pop` (50/25/25) growing from `(capacity 0, length 0)`,
xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **7.35** | 9.56 | 1.3× faster |
| p99 ns/op | **27.9** | 60.5 | 2.2× faster |
| RSS delta MB | **11.8** | 37.4 | |
| structure-only RSS delta MB | **1.3** | 9.7 | |
| startup ms | **0.6** | 16.9 | 28× (reported separately; not throughput) |

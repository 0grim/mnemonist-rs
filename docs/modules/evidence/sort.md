# sort — evidence

Gate artifacts for `docs/modules/sort.md`: test-to-gap table, full fuzz campaign table, full
falsification record, full benchmark table.

## Test-to-gap mapping

| Gap | Where | What |
|---|---|---|
| 1, 2, 3 | `sort/{insertion,quick}.rs` `degenerate_windows_do_nothing`, `check_window` tests | Empty window, empty slice, inverted window |
| 2 | `tests/boundary/sort.js` "every window of a fixed array" | **All 78 windows**, differentially against vendored upstream, for all four functions |
| 4 | `insertion.rs::nan_pins_the_elements_to_its_right`, `sort/mod.rs::nan_compares_false_in_every_direction` | `NaN` pins rather than sinks |
| 5 | `tests/boundary/sort.js` "refuse a non-numeric element" | The stated divergence, asserted as a refusal so silently accepting would be noticed |
| 6 | `tests/boundary/sort.js` "leave elements outside the window untouched" | A string and an object either side of the window, differentially |
| 7 | `insertion.rs::indices_past_the_end_of_the_array_never_move`, `quick.rs::indices_past_the_end_of_the_array_do_not_panic`, and the fuzz grammar | Index values drawn from a range **wider than the value array** |
| 8 | fuzz grammar | Index array length independent of the value array's |
| 9 | `tests/boundary/sort.js` "return the very array it was given" | `strictEqual(returned, argument)` for all four, plus mutation read back through the *caller's* handle |
| 10 | `quick.rs::every_pointer_width_sorts_the_same_way`, `tests/boundary/sort.js` "Uint16Array of indices" | 300 members, forcing the 16-bit width |
| 11 | `typed_arrays.rs::indices_truncates_its_length_but_not_its_width`, `indices_refuses_the_lengths_upstream_refuses`, `tests/boundary/sort.js` | Every boundary length, both signs, both integralities, `NaN`, `Infinity`, `> 2³²` |
| 12 | `quick.rs::already_sorted_input_does_not_overflow_the_stack` | 4,096 elements sorted, reversed and all-equal |
| 13 | `insertion.rs::is_stable_where_quick_sort_is_not`, `quick.rs::disagrees_with_insertion_sort_on_equal_keys` | The two permutations asserted against each other, and both checked non-decreasing |

## Fuzz campaign table

| seed | cases | ops | wall | divergences |
|---|---|---|---|---|
| 42 | 4,898 | 196,041 | 60.0s | 0 |
| 20260801 | 5,076 | 204,428 | 60.0s | 0 |

## Falsification record

### Fuzzer falsification (gate 6)

1. **`indices` choosing its width from the truncated length.** Must break
   `indices_truncates_its_length_but_not_its_width` and the boundary spec's
   "truncate a fractional length while sizing the width from the raw one". Both went red; the
   fuzzer found it in 62 cases (0.3s) and shrank it to a **single operation**, `m.indices(256.5)`.
2. **`a > b` rewritten as `!(a <= b)`** in `inplace_insertion_sort` — identical for every totally
   ordered type, the exact opposite whenever either side is `NaN`. Must break
   `nan_pins_the_elements_to_its_right`. It went red; the fuzzer found it in 300 cases (0.4s) and
   shrank it to `m.inplaceInsertionSort([NaN, 0], 0, 2)`.

Both seeds are committed in `crates/difffuzz/proptest-regressions/sort.txt` with a provenance block
saying they came from sabotages and not from real port defects.

### Falsification of the port (gate 6, on the original suite)

Deleting the `array.swap(j - 1, j)` line from
`mnemonist_core::sort::insertion::inplace_insertion_sort` must break `test/sort.js`'s
**"insertion → should properly sort inplace."** — `assert.deepStrictEqual(data, [-3, 1, 1, 2, 3, 5,
6, 7, 8, 9, 18])`. Confirmed: 13 passing → **9 passing, 4 failing**, the named assertion among
them, with `AssertionError [ERR_ASSERTION]: Expected values to be strictly deep-equal`. Back to 13
passing after revert.

Note which blocks *stayed* green, because it is the useful part: all six `quick` blocks and both
`insertion` **indices** blocks passed with the sabotage in place. They are a different code path,
and a falsification that had targeted a shared helper would have gone red everywhere and told us
less.

## Bench table

`bench/results.json` → `modules["sort"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, 500 samples/side.

**`sort-2e4x50`** — quicksort of a freshly-generated 20,000-element random array (values 0..1e6),
50 passes, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/element | **31.9** | 78.4 | 2.5× faster |
| p99 ns/element | **41.8** | 99.7 | 2.4× faster |
| min ns/element | **30.6** | 75.8 | 2.5× faster |
| RSS delta MB | **1.4** | 41.0 | |
| startup ms | **0.6** | 16.2 | 27× (reported separately; not throughput) |

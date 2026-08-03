# multi-array — evidence

Gate artifacts for `docs/modules/multi-array.md`: test-to-gap table, fuzz grammar, full benchmark
tables and probes.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/multi_array.rs` — 11 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_set_walkthrough`, `reproduces_the_upstream_push_walkthrough`, `get_returns_none_past_dimension_and_the_bucket_otherwise`, `has_and_multiplicity_agree_with_upstream` | the upstream blocks, as a baseline, both container kinds |
| `inserting_out_of_order_leaves_a_real_gap_at_dimension` | the untested gap-read case above |
| `containers_and_associations_walk_dimension_in_gets_order`, `values_are_global_insertion_order_or_reversed_per_bucket`, `entries_walk_each_bucket_tail_to_head_in_dimension_order`, `keys_is_the_dimension_range` | the five iterator factories directly against literal expected sequences, including the forward-vs-reverse order contrast between `get` and `values(index)` (see the core module's docs — the sharpest place a transcription error would hide) |
| `fixed_capacity_values_narrow_to_their_width` | the untested overflow-truncation case above |
| `an_empty_multi_array_has_no_containers_or_values` | the zero-state baseline |

## Fuzz grammar

* **Op alphabet:** `set` (weight 5) and `push` (weight 4) dominate, since they are the only ops
  that grow a bucket or exercise the capacity throw; `get`/`has`/`multiplicity` (weights 3/2/2)
  round it out. `containers`/`associations`/`values`/`entries`/`keys` are deliberately **not** in
  the alphabet — see the document and the spec's own module docs: all five now return a genuine
  opaque iterator on both sides, which `fuzz/oracle.js`'s `encode()` reduces to `{}`
  regardless of what is actually inside, so comparing them can only ever agree trivially.
* **Index pool:** ten indices, small enough that `set`/`push` collide on the same bucket constantly.
* **Constructor:** alternates between the default dynamic container (weight 3) and a fixed-capacity
  `Uint8Array`/`Uint16Array`/`Uint32Array` with a small capacity (weight 2, capacity `1..12`), so a
  `push`/`set` past capacity is common rather than rare.
* **Observable state:** `size`, `dimension`. `get`'s own return value (compared per-op, not just as
  state) renders a container exactly as `fuzz/oracle.js`'s `encode()` renders the real value: a
  plain array in dynamic mode, `{"$typed": ..., "values": [...]}` in fixed mode.

## Bench tables

`bench/results.json` → `modules["multi-array"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`get`/`multiplicity` (50/25/25), dynamic (unbounded, exact-`f64`)
container, over a 20,000-index domain, ~25 values per bucket on average by the run's end, xorshift32
seed 42. Original (first) measurement:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 50.2 | **26.4** | 1.9× slower |
| p99 ns/op | **177.5** | 272.8 | 1.5× faster |
| min ns/op | **6.4** | 11.7 | |
| RSS delta MB | **19.9** | 115.8 | |
| structure-only RSS delta MB | **0.1** | 5.7 | |
| startup ms | **0.6** | 17.3 | 29× (reported separately; not throughput) |

Final whole-suite pass, after the fixes recorded in the log:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | ~38.3 | ~29.81 | 1.31× slower |

## Allocation-counting probe

`bench/runner/examples/multi_array_alloc_probe.rs` builds a 20,000-index array with 25 values/bucket
and, with an allocation-counting global allocator, isolates `get` from `multiplicity` (an O(1) read
with no walk and no allocation) over 250,000 calls of each:

| variant | ns/call | allocations/call |
|---|---|---|
| `get(index)` (walk + materialise) | 50.96 | **1.000** |
| `multiplicity(index)` (O(1), no walk) | 0.82 | 0.000 |
| bare `Vec::with_capacity(25)` + fill, no walk | 34.88 | 1.000 |

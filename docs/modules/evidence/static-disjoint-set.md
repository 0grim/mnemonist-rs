# static-disjoint-set — evidence

Gate artifacts for `docs/modules/static-disjoint-set.md`: test-to-gap table, fuzz grammar, full
falsification record, full benchmark tables.

## Test-to-gap mapping

| Test | Closes gap |
|---|---|
| `should_be_possible_to_have_a_set_working` | 1:1 port of the upstream `it`, as a baseline |
| `singleton_is_its_own_root` | 1, 12 |
| `find_compresses_the_whole_path` | 1, 2 — builds a 4-deep chain by hand and asserts every node on the path now points at the root |
| `dimension_only_drops_on_a_successful_union` | 3, 4, 5 — the branch that made gate 6 a false green |
| `empty_set_is_degenerate_but_legal` | 11 |
| `picks_a_distinct_width_per_array` | 8 — size 300, where `parents` is 16-bit and `ranks` is still 8-bit |
| `mapping_width_follows_the_current_dimension` | 9 — asserts the width *narrows* as unions accumulate |
| `root_rank_wraps_at_the_ranks_array_width` | 10 — 299 increments into a `Uint8Array`, asserts `299 % 256` |
| `mapping_and_compile_agree_and_are_index_ordered` | 15, 16 — both called on the same set and cross-checked |
| `reproduces_upstream_rank_bug` | 17 — pins a concrete input where the elected root differs |
| `rejects_a_size_no_pointer_array_can_index` | 8 (the throw) |
| `find_panics_out_of_range` | 13 |
| `utils::typed_arrays::*` (4 tests) | 8 — every width boundary, non-integral input, and `NaN` |

## Fuzz grammar

* **Op alphabet:** `union(x, y)` (weight 3), `find(x)`, `connected(x, y)`.
* **Observable state, compared after every op:** `size`, `dimension`, `mapping()`, `compile()`.
  The last two are observations rather than ops on purpose — both call `find` on every item, so
  path compression is exercised on every step of every program rather than when the generator
  happens to pick it.
* **Sizes:** 1..=400, straddling 256 so the `parents` 8→16-bit switch is generated, and large
  enough for the rank wrap to be reachable.
* **Program length:** 1..600 ops.
* **Deliberately excluded from the grammar:** out-of-range indices (see the divergence table
  above). Nothing else is excluded.

## Falsification record (gate 6)

Sabotage: *fix* BUG-STATIC-DISJOINT-SET-1 in the core — the most plausible way this port could realistically break, since
it makes the port strictly more correct than upstream and therefore wrong. Caught in **129 cases
(0.3 s)** and shrunk from a 600-op program to three operations:

```js
var s = new StaticDisjointSet(23);
s.union(10, 7);   // ranks[10] stays 0; ranks[7] never set
s.union(11, 7);   // upstream: 0 == 0, equal-ranks branch, root becomes 11
s.find(10);       // upstream 11, rank-correct port 10
```

The sabotage was reverted; the seed is committed in
`crates/difffuzz/proptest-regressions/static-disjoint-set.txt` with a provenance header, and
proptest replays it before any novel case on every subsequent run.

## Bench tables

`bench/results.json` → `modules["static-disjoint-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, 32 MB L3, WSL2, Node 24.18.1, rustc 1.97.1.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `union`/`find`/`connected` (50/25/25) over size 1e6, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **15.6** | 22.6 | 1.5× faster |
| p99 ns/op | **34.5** | 68.5 | 2.0× faster |
| RSS delta MB | **11.1** | 41.9 | |
| structure-only RSS delta MB | **1.4** | 11.8 | |
| startup ms | **0.6** | 15.3 | 25× (reported separately; not throughput) |

**`mixed-4e6`** — the same op mix at four times the size:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **21.8** | 42.9 | 2.0× faster |
| p99 ns/op | **43.6** | 134.9 | 3.1× faster |
| min ns/op | **13.1** | 28.1 | |
| RSS delta MB | **25.3** | 78.4 | |
| structure-only RSS delta MB | **13.0** | 23.4 | |

No regressions on either workload.

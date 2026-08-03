# multi-set — evidence

Gate artifacts for `docs/modules/multi-set.md`: test-to-gap table, fuzz grammar, full benchmark
table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/multi_set.rs` — 16 tests:

| Test | Closes gap |
|---|---|
| `deleting_an_existing_item_behaves_normally` | the upstream `delete` block, as a baseline |
| `b_161_deleting_an_absent_item_corrupts_size_and_dimension_but_reports_true` | BUG-MULTI-SET-2 |
| `b_162_edit_into_an_existing_key_does_not_adjust_dimension` | BUG-MULTI-SET-3 |
| `set_replaces_a_missing_item_but_adds_to_an_existing_one` | BUG-MULTI-SET-1 |
| `a_set_is_its_own_subset_and_superset_by_identity` | the `A === B` shortcut |

## Fuzz grammar

* **Op alphabet:** `add` (weight 5), `remove` (3), `set`/`edit` (2 each), `delete`/`has`/
  `multiplicity`/`frequency` (2/2/2/1), `clear` (1), `top` (1, bounded `n` in `1..=5` so it never hits
  its own arity guard — see the spec's module docs for why that guard is out of scope for a
  core-level campaign).
* **Item pool:** three items (`"a"`, `"b"`, `"c"`).
* **Count pool:** `1`, `2`, `4`, `0`, `-1`, `-3` — positive (so multiplicities build up), zero (a
  documented no-op), and negative (the sign-flip delegation between `add` and `remove`). Fractional
  and `NaN` counts are deliberately not in this grammar; see DIV-MULTI-SET-3 and the spec's own module docs.
* **Observable state:** `size`, `dimension`, `items` (`[item, count]` pairs, in insertion order).

## Bench table

`bench/results.json` → `modules["multi-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `add`/`multiplicity`/`remove` (50/25/25) over a 20,000-item domain,
xorshift32 seed 42. Figures below are post-fix (see log for the pre-fix figures and the fix
history):

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **16.13–16.37** (four-run spread) | 22.3–24.80 | ~1.36× faster |
| p99 ns/op | **37.4** | 44.7 | 1.2× faster |
| min ns/op | 17.9 | **15.9** | 1.13× slower |
| RSS delta MB | **8.1** | 30.0 | |
| structure-only RSS delta MB | **0.1** | 5.7 | |
| startup ms | **0.6** | 16.5 | 27× (reported separately; not throughput) |

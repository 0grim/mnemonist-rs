# hashed-array-tree — evidence

Gate artifacts for `docs/modules/hashed-array-tree.md`: test-to-gap table, fuzz grammar, full
falsification record, full benchmark table.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/hashed_array_tree.rs` — 15 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all ten upstream blocks, as a baseline |
| `pop_reads_the_last_block_rather_than_the_popped_index_s_block` | 2 — B-15, pinned value by value against Node |
| `pop_after_a_shrinking_resize_reads_a_block_that_is_no_longer_live` | 2, 8 — the same defect reached a second way |
| `get_at_length_reads_the_block_instead_of_reporting_absence` | 5 — B-16 |
| `set_at_length_writes_a_slot_that_length_does_not_cover` | 6 — B-16's write half |
| `indexing_at_capacity_raises_the_typeerror_upstream_raises` | 7 — including V8's exact message |
| `the_out_of_bounds_message_names_the_array_class` | across all three widths; upstream matches `/bounds/` |
| `stores_truncate_at_the_element_width` | 12, 13 |
| `derives_the_index_split_constants_from_the_block_size` | 3 |
| `a_bare_grow_adds_exactly_one_block` | 10 |
| `a_shrinking_resize_keeps_the_blocks_and_their_contents` | 8, 11 |
| `push_after_a_shrinking_resize_overwrites_the_stale_slot` | 9 |
| `initial_capacity_is_the_larger_of_the_two_rounded_up_to_a_block` | 14 — seven combinations |
| `rejects_every_non_power_of_two_block_size` | 15 — eleven rejected, seven accepted |
| `a_block_size_upstream_only_accepts_by_truncation_is_refused` | 15 — the ToInt32 boundary at 2^32 |
| `a_block_size_of_one_gives_every_element_its_own_block` | 4 |
| `a_fresh_tree_pops_nothing` | — |
| `indexes_across_block_boundaries` | 1 — ten elements over three blocks, every index read back |

## Fuzz grammar

* **Op alphabet:** `push(v)` (weight 5) · `pop()` (3) · `set(i, v)` (3) · `get(i)` (3) ·
  `grow(c)` (1) · `grow()` (1) · `resize(l)` (2).
* **Observable state, compared after every op:** `length`, `capacity`, `blockSize`, `offsetMask`,
  `blockMask` and **`blocks`** — every block, slot for slot.
* **Constructors:** all three widths × block sizes `{1, 2, 4, 8}` × `initialLength` and
  `initialCapacity` each `0..24`.
* **Indices:** `0..64`, against lengths that rarely exceed 30. **Values:** `0..320`, above 255 so a
  `Uint8Array` tree truncates.
* **Program length:** 1..200 ops.
* **Deliberately excluded:** non-integer arguments (see the divergence table) and `blockSize`
  values outside `{1, 2, 4, 8}`. Nothing else — in particular out-of-bounds indices are generated
  freely, and both throws are compared by their full message.

## Falsification record

### Fuzzer falsification

**A — `pop` reading the right block.** The obvious cleanup: `blocks[length >> blockMask]` instead of
`blocks[blocks.length - 1]`. Caught in **1,228 cases (1.8 s)**, shrunk from 200 ops to five:

```js
var s = new HashedArrayTree(Uint8Array, {blockSize: 1});
s.resize(2);   // two one-element blocks, length 2
s.pop();       // length 1
s.pop();       // length 0
s.push(1);     // writes block 0
s.pop();       // port 1 (block 0), upstream 0 (block 1, the last one)
```

**B — `set`'s guard tightened from `<` to `<=`.** What anyone tidying the bounds check would write.
Caught in **2,165 cases (5.2 s)**, shrunk to ten ops ending in `s.set(19, 0)`, where the port
returned `{"$throw": "HashedArrayTree(Uint8Array).set: index out of bounds."}` and upstream returned
`{"$self": true}`.

Both sabotages were reverted and both seeds are committed with provenance in
`crates/difffuzz/proptest-regressions/hashed-array-tree.txt`.

### Falsification of the port (gate 6)

Separate from the fuzzer falsifications: gate 6 asks that sabotaging the core turns the *original
mocha suite* red, proving it exercises Rust rather than a JS fallback.

**The first attempt failed the gate's own standard, and is recorded because that is the point of the
gate.** Named assertion: `should be possible to push values.` →
`assert.strictEqual(array.capacity, 256)` at `test/hashed-array-tree.js:61`. Sabotage: `push`'s
growth guard weakened from `capacity == length` to `capacity < length`. It **did** go red — but at
`test/hashed-array-tree.js:73`, in the *pop* test, because the off-by-one still grows, just one push
late, and `capacity` still reaches 256 by the 250th push. A sabotage that goes red somewhere other
than where it was predicted to is weaker evidence than one that goes red where predicted: it shows
the suite runs Rust, but not that the named assertion depends on the named code. Reverted and redone.

**Second attempt. Named assertion:** `should be possible to pop values.` →
`assert.strictEqual(array.pop(), 2)` at `test/hashed-array-tree.js:71`. Chosen because `pop` is the
one method whose defect this module exists to reproduce.

**The sabotage:** mis-porting `(--this.length) & this.offsetMask` as a *post*-decrement — computing
the offset from the pre-decrement length. This is the single most plausible way to get that line
wrong, and it is a one-token change.

**Confirmed red**, at exactly the named line: `9 passing, 1 failing`, the failure being
`0 !== 2` at `test/hashed-array-tree.js:71`. Reverted; **confirmed green again**: 10 passing.

## Bench table

`bench/results.json` → `modules["hashed-array-tree"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`get`/`pop` (50/25/25), `Uint32Array` blocks at the default
1024-element block size, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **5.5** | 8.6 | 1.6× faster |
| p99 ns/op | **10.0** | 26.4 | 2.6× faster |
| min ns/op | **4.8** | 8.0 | 1.7× faster |
| RSS delta MB | **7.0** | 23.6 | |
| structure-only RSS delta MB | **1.3** | 9.6 | |
| startup ms | **0.6** | 16.4 | 27× (reported separately; not throughput) |

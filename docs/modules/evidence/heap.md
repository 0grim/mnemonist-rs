# heap — evidence

Gate artifacts for `docs/modules/heap.md`: test-to-gap table, fuzz grammar, full falsification
record, full benchmark tables.

## Test-to-gap mapping

`crates/mnemonist-core/src/structures/heap.rs` (24 tests) and
`crates/mnemonist-core/src/utils/comparators.rs` (9 tests):

| Test | Closes gap |
|---|---|
| `push_pop_is_ascending`, `to_array_leaves_the_heap_intact`, `consume_empties_the_heap`, `a_max_heap_reverses_the_comparator`, `heapify_then_consume_sorts` | the upstream blocks, as a baseline |
| `a_comparator_that_grows_the_array_mid_sift_does_not_panic` | 1 — and the assertion is that it *completes*, which an algorithm holding `&mut Vec` could not |
| `a_comparator_that_shrinks_the_array_makes_the_walk_read_undefined` | 1 — the frozen `endIndex` half |
| `clear_detaches_an_in_flight_sift` | 1 — the rebinding half (DIV-STACK-3) |
| `a_throwing_comparator_desynchronises_size_from_the_array` | 2 |
| `sort_with_is_stable`, `sort_with_puts_undefined_last_without_comparing_it` | the two `Array.prototype.sort` properties `nsmallest` depends on |
| `nsmallest_over_an_array_like`, `nlargest_over_an_iterable` | 13–18, the paths |
| `pushpop_on_an_empty_heap_returns_its_argument`, `from_items_heapifies_in_place` | 20, 21 |
| `replace_on_an_empty_heap_throws_upstreams_message` | 11 |
| `undefined_compares_equal_to_everything` | the slot semantics gaps 1 and 13 both depend on — and it asserts the *trap*, that Rust's own `Option` ordering says the opposite |
| `nan_compares_equal_to_everything` | 4 |
| `reverse_swaps_arguments_rather_than_negating`, `the_two_reverses_agree_pointwise` | 6, 7 |
| `tuple_comparator_is_lexicographic`, `tuple_comparator_reads_past_a_short_tuple_as_undefined` | 9 |

## Fuzz grammar

```
module=heap seed=42       cases=25677 ops=2619243 wall=120.0s divergences=0
module=heap seed=20260801 cases=12248 ops=1256277 wall=60.0s  divergences=0
module=heap seed=31337    cases=11060 ops=1147769 wall=60.0s  divergences=0
module=heap seed=42       cases=12589 ops=1283659 wall=60.0s  divergences=0   (post-fix re-run)
```

* **Op alphabet:** `push(v)` (weight 6) · `pop()` (3) · `peek()` (2) · `replace(v)` (2) ·
  `pushpop(v)` (2) · `toArray()` (2) · `clear()` (1) · `consume()` (1).
* **Constructor alphabet:** six comparator factories, mirrored name for name between
  `fuzz/oracle.js` and `crates/difffuzz/src/modules/heap.rs`. Two are pure (`ascending`,
  `descending`); four are not:

  | factory | what it does from inside the sift |
  |---|---|
  | `pushy` | `items.push(99)` — grows the array under an index the sift already chose |
  | `popper` | `items.pop()` — shrinks it, so the walk reads past its frozen `endIndex` |
  | `clearer` | `heap.clear()` — **rebinds** it, detaching the sift onto the old array |
  | `boom` | throws, leaving `items.length` one ahead of `size` |

* **Observable state, compared after every op:** `size` and `items`. They are separate quantities
  upstream and BUG-HEAP-1 makes them genuinely disagree, so comparing both is what pins it.
* **Values:** `0..24`, small enough that duplicates are frequent — a heap's tie-breaking is
  observable through `toArray`, and `sift_up`'s `>= 0` is the only thing that decides it.

## Falsification record

### Fuzzer falsification

Sabotage: `Heap::clear` truncating the backing array in place instead of rebinding it — the DIV-STACK-3
collapse, and the most plausible way a future cleanup breaks this port, because `set_length(0)` and
`allocate(0)` leave an **identical** observable state for every program whose comparator has no side
effects.

* **All 14 assertions of `test/heap.js` still passed under it**, and all 7 of
  `test/fixed-reverse-heap.js`.
* The fuzzer found it in **0.1 s** and shrank a 200-op program to three operations:

  ```js
  var s = new Heap(clearer);   // clears the heap on its first comparison
  s.push(1);
  s.pushpop(18);               // upstream 1, truncating port undefined
  ```

* `tests/boundary/heap.js` caught it too, by name.

Reverted; the seed is committed with a provenance header in
`crates/difffuzz/proptest-regressions/heap.txt`, and proptest replays it before any novel case.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:** `should be possible to pop the heap.`
— `assert.strictEqual(heap.pop(), 1)` at `test/heap.js:47`. Chosen because it is the shortest path
from `push` through `sift_down` to an observable value, so a sabotage of the sift cannot miss it by
accident.

**The sabotage:** `sift_down`'s `compare(item, parent) < 0.0` inverted to `> 0.0` — one character,
in the function every other algorithm in the file calls.

**Confirmed red**, and red in the named place: `3 passing, 11 failing`, the named assertion failing
with `34 !== 1`. Reverted; **confirmed green again**: `14 passing`.

## Bench tables

`bench/results.json` → `modules["heap"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`pop`/`peek` (50/25/25), default numeric comparator, value range
1e6, xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 32.0 | **24.3** | 1.32× slower |
| p99 ns/op | **48.8** | 60.6 | 1.24× faster |
| min ns/op | 20.8 | **19.5** | 1.07× slower |
| RSS delta MB | **9.9** | 46.8 | |
| structure-only RSS delta MB | **1.3** | 9.8 | |
| startup ms | **0.6** | 16.5 | 27× (reported separately; not throughput) |

**Bare-`Vec<f64>` counterfactual probe** (`bench-runner --heap-probe`), isolating the `RefCell` +
`Comparator` trait indirection from the algorithm itself:

| variant | p50 ns/op | min ns/op |
|---|---|---|
| wrapped (`RefCell<VecStore<f64>>` + `Comparator` trait) | 31.781 | 20.529 |
| bare `Vec<f64>`, no indirection | **21.721** | **16.271** |

Checksums agree (both are valid min-heaps over the same op stream; ties among numerically-equal
pushed values do not affect which *value* a pop returns, so tie-break policy does not need to match
for this check to be meaningful).

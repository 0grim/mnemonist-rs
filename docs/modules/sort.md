# sort

Upstream: `sort/quick.js` (116 LOC) + `sort/insertion.js` (50 LOC) +
`utils/typed-arrays.js` (187 LOC, only `indices` and `getPointerArray` reachable) ·
`test/sort.js` — **170 lines, 13 `it` blocks, 23 assertion statements**.

Port: `crates/mnemonist-core/src/sort/{mod,insertion,quick}.rs`,
`crates/mnemonist-core/src/utils/typed_arrays.rs`.
Bridge: `crates/mnemonist-napi/src/sort.rs`.
Shims: `tests/bridge/sort.js`, `tests/bridge/sort/{insertion,quick}.js`,
`tests/bridge/utils/typed-arrays.js`.

**This is the first unit in the port with no instance.** Every module before it is a constructor
with methods; these are four free functions that mutate a caller-supplied array. That changes the
shape of almost everything downstream — there is no `#[napi]` class, no `RefCell<Core…>`, nothing
for `crate::cursor` to attach a `Symbol.iterator` to, no export shape a one-line shim can forward,
and no observable state for the differential fuzzer to compare. Each of those is dealt with below.

It is also the first unit whose upstream surface spans **three files with two different export
shapes**.

---

## What upstream tests

Thirteen `it` blocks, six of them structurally identical between the two algorithms, over one
fixture:

```js
var DATA = [2, 7, 1, 5, 8, 9, 1, -3, 3, 18, 6];

insertion.inplaceInsertionSort(DATA.slice(), 0, DATA.length);      // deepStrictEqual
insertion.inplaceInsertionSort(DATA.slice(), 3, 7);                // three slices
insertion.inplaceInsertionSortIndices(DATA.slice(), typed.indices(DATA.length), 0, DATA.length);
// …and the same six for `quick`, minus the [1] / [2,1] edge cases
```

Characterising the shape of that coverage:

* **One array, eleven elements, four windows.** `0..11`, `0..3`, `3..7`, `5..11`, plus `[1]` and
  `[2, 1]` for insertion only. Every element is a small integer.
* **The index permutations are asserted exactly**, and they *differ* between the two algorithms —
  `[7, 2, 6, …]` for insertion, `[7, 6, 2, …]` for quick — because `DATA` holds `1` twice and
  insertion sort is stable while quicksort is not. This is the single most valuable thing the
  original suite does, and it is the reason both algorithms had to be transcribed statement by
  statement rather than delegated to `slice::sort_unstable_by`.
* **Two "sanity tests"** sort 1,000 `Math.random()` values and assert the result is *strictly*
  increasing. With random doubles a duplicate is essentially impossible, so the strictness never
  bites.
* **`utils/typed-arrays.js` is called exactly once, with 11**, inside `typed.indices(DATA.length)`.

## What upstream does NOT test

This is the section that carries the weight. Everything below is reachable through the public API
and never exercised by the original suite.

**The window**

1. **An empty window is never passed.** `lo === hi` is legal and is a no-op.
2. **A window that is not the whole array *and* not one of the four fixed ones** is never used.
   `test/sort.js` uses four windows out of the 78 a length-11 array admits.
3. **`lo > hi` is never passed.** Upstream treats it as empty; this port refuses it (below).

**The values**

4. **`NaN` is never sorted.** It loses every relational comparison in both languages, so it neither
   sinks nor lets anything sink past it — it pins its neighbours in place. A port written against
   an `Ord`-style total order would get this wrong and pass all 23 assertions.
5. **No two elements are ever `Infinity`, `-0` or a non-number.** Upstream compares through
   `valueOf`/`toString`, so strings and objects sort by JavaScript's relational comparison.
6. **Nothing outside the window is ever checked for being left alone.** Upstream reads and writes
   only `array[lo..hi)`, so `inplaceQuickSort(['x', 3, 1, 2, {}], 1, 4)` is legal and leaves both
   non-numbers untouched. A port that read the whole array would break it.

**The indices flavours**

7. **Every index is always in range.** `typed.indices(n)` produces exactly `0..n`, so
   `array[indices[j]]` never reads past the end of `array`. Upstream gets `undefined` there, and
   **every comparison against `undefined` is false**, which changes the permutation. This is the
   entire justification for `mnemonist_core::sort`'s `Option<&T>` comparisons and it is invisible
   to the original suite.
8. **The indices array is always the same length as the value array.** A shorter or longer one is
   legal.
9. **The array is never returned by identity.** Both flavours return the object they were given;
   `test/sort.js` only ever inspects the return value, so a port that sorted a **copy** and handed
   the copy back passes all thirteen blocks while breaking every in-tree caller — and mnemonist
   itself calls these from `passjoin-index.js` and `suffix-array.js` for their side effect.

**`utils/typed-arrays.js#indices`**

10. **Only `Uint8Array` is ever constructed.** `indices(11)` selects the 8-bit width; the 16- and
    32-bit branches, and the `> 2³²` throw, are unreachable from this test file.
11. **`length` is always a small non-negative integer.** The fractional, negative and `NaN` cases
    are all reachable and all behave differently — see the divergence-free finding below.

**The two algorithms' own machinery**

12. **The partition stack is never driven deep.** Eleven elements need at most four stack entries
    out of 64. Sorted, reverse-sorted and all-equal inputs — quicksort's three classic degenerate
    partitions — never occur.
13. **`inplaceQuickSort`'s duplicate handling is only checked through one array with one repeated
    value.**

## What we test in addition

Every gap above is closed by a mix of Rust unit tests, `tests/boundary/sort.js` (differential
against vendored upstream, covering all 78 windows of the fixed array, non-numeric refusal,
untouched regions outside the window, identity of the returned array, and every boundary length for
`indices`), and the fuzz grammar (independent index-array lengths, indices wider than the value
array). Full test-to-gap mapping: evidence file.

Plus the differential fuzzer: 9,974 programs and 400,469 operations across two 60-second campaigns,
zero divergences.

## Bugs this found

Two, both in upstream, both verified against Node 24.18.1, and both **found by reading rather than
by fuzzing** — for the reason given under "Deliberate divergences": the port cannot reach them,
because reaching them requires an element that runs JavaScript during a comparison.

### BUG-SORT-1 — `sort/insertion.js` declares its loop counter as a global

Both exported functions open with

```js
function inplaceInsertionSort(array, lo, hi) {
  i = lo + 1;          // no `var`, no `let`
  var j, k;
```

`i` is therefore `globalThis.i`, shared by every call in the realm. After one
`inplaceInsertionSort([3, 1, 2], 0, 3)`, `global.i` is `3`.

That is not merely untidy. `>` invokes `valueOf`, so an element can re-enter the sorter
mid-comparison and the inner call leaves the outer call's counter wherever it finished:

```js
function reentrant(v, payload) {
  return {valueOf: function () { if (payload) { payload(); payload = null; } return v; }};
}
var inner = [3, 1, 2];
var outer = [reentrant(5),
             reentrant(1, function () { insertion.inplaceInsertionSort(inner, 0, 3); }),
             reentrant(3), reentrant(2)];

insertion.inplaceInsertionSort(outer, 0, 4);
outer.map(Number);   // [1, 5, 3, 2]   — expected [1, 2, 3, 5]
```

The file would also throw `ReferenceError` outright under `'use strict'`, and mnemonist ships
`"type": "commonjs"` sloppy-mode files, so today it does not.

### BUG-SORT-2 — `sort/quick.js`'s partition stack is module state, shared by all four sorts

```js
var LOS = new Float64Array(64),
    HIS = new Float64Array(64);
```

allocated once at module scope and used by `inplaceQuickSort` *and*
`inplaceQuickSortIndices`. `i` is a proper local here, so the failure is subtler than BUG-SORT-1's: the
outer call's index keeps pointing into a stack the inner call has overwritten. Measured on a
40-element array whose first compared element re-enters:

```js
quick.inplaceQuickSort(arr, 0, 40);
// [0,1,4,7,10,13,…,35,37,38,32,29,26,…,3]   — 38 of 40 elements out of order
```

Both are the same defect wearing different clothes — shared mutable module state in a function that
can be re-entered through a comparison — and both are invisible to `test/sort.js`, which sorts
numbers only.

## Deliberate divergences

Three, below.

### DIV-SORT-1 — the port takes numbers, upstream takes anything

`crates/mnemonist-napi/src/sort.rs` reads elements as `f64`. Upstream is duck-typed and compares
whatever it is given through JavaScript's relational operators, which coerce via
`valueOf`/`toString`. Supporting that means calling back into JavaScript from inside the sort
loop — the **re-entrant callback capability** `heap` establishes — which this unit
deliberately does not reach for, since
nothing in `test/sort.js` or in mnemonist's own callers passes a non-number.

The refusal is loud: `mnemonist-rs: sort element 3 is not a number… see docs/modules/sort.md.`

**This divergence is why BUG-SORT-1 and BUG-SORT-2 are unreachable in the port, and it is worth being precise
about the direction.** The port is not *fixing* those bugs; it is refusing the only inputs that can
observe them. With numeric elements, no user code can run during a comparison, so upstream's shared
global counter and shared partition stack are never re-entered and a local behaves identically.
Reproducing the bugs bug-for-bug would require first admitting JavaScript callbacks into the sort
loop and then adding shared state
to reproduce a defect nothing can see — the port would be *less* faithful, not more, and this
port's own rule that "a divergence where our port is more correct is a bug in the port" does not
apply to a regime the port does not admit.

### DIV-SORT-2 — windows outside `0..=length` are refused

Upstream reads `undefined` past the end of an array and writes into holes, producing a sparse array
with genuine `undefined` elements. A JS array hole has no Rust representation, and modelling one
would mean `Vec<Option<f64>>` throughout for a regime the original suite never enters. The window
is checked instead, in `mnemonist_core::sort::check_window`, and the bridge reports it with a
message naming the limit. Same position `PointerVec::get` already takes for the same reason.

### DIV-SORT-3 — the export shape is re-assembled by the shim

The addon exports at top level, so there is no `sort/quick` object to hand back and `indices` is far
too generic a name to claim at the top of an addon that will eventually carry forty modules' worth
of helpers. It is `typedArraysIndices` in the addon, and `tests/bridge/sort.js` maps the flat names
back into upstream's two-file shape.

`tests/bridge/sort.js` is the aggregate and the two leaves are cut from it, rather than the reverse.
`test/sort.js` never requires `../sort.js`, so an aggregate that merely re-required its own leaves
would be a decorative file that exists only to satisfy `tests/verify.sh` gate 3. This way it is
load-bearing.

### Not a divergence, but easy to mistake for one: `indices` and its two coercions

`exports.indices` uses its argument twice and coerces it **differently** each time.
`getPointerArray` compares `length - 1` as a double; the `TypedArray` constructor applies `ToIndex`,
which truncates. So:

```js
typed.indices(256.5)    // Uint16Array(256) — a width wider than 256 elements need
typed.indices(255.5)    // Uint8Array(255)
typed.indices(-0.5)     // Uint8Array(0)     — ToIndex accepts -0
typed.indices(-1)       // RangeError: Invalid typed array length: -1
typed.indices(NaN)      // mnemonist: Pointer Array of size > 4294967295 is not supported.
```

All confirmed against Node 24.18.1. `mnemonist_core::utils::typed_arrays::indices` therefore takes
an `f64` rather than a `usize`; the first draft took a `usize`, truncated at the boundary, and
produced `Uint8Array(256)`. `tests/boundary/sort.js` caught it, and the fuzzer's first
falsification pins it.

## Fuzz + bench

### Fuzz

Two campaigns, both clean, **400,469 operations, zero divergences**. Full campaign table: evidence
file.

Ops: `inplaceInsertionSort`, `inplaceQuickSort`, `inplaceInsertionSortIndices`,
`inplaceQuickSortIndices`, `indices`. Arrays of 0–24 elements drawn from a pool of 11 values
including `NaN` and fractions; windows generated against the subject's own length; index arrays of
independent length whose entries are drawn **wider than the value array**, so a good share point
past its end. Full grammar and exclusions in `fuzz/log.txt`.

This is the first free-function module, so it is also the first campaign that compares **no
observable state at all** — there is none. What it compares is the return value *and every
argument after the call*, which is where an in-place sort's whole effect lives. `fuzz/oracle.js`
does that echo generically for any module declaring `ModuleSpec::functions()`.

Throughput is ~3,270 op/s against ~23,600 for the structure modules. That is the payload — an op
here ships a whole array in each direction and can allocate a 65,537-element typed array on both
sides, where `union(x, y)` ships two integers — **not** a regression, and the op counts are not
comparable across modules.

**Falsification (gate 6, on the fuzzer).** Two sabotages, each naming the assertion it had to break,
each confirmed red and then green after revert: `indices` choosing its width from the truncated
length rather than the raw one is caught in 62 cases (0.3s), shrunk to a single operation; and
rewriting `a > b` as `!(a <= b)` in `inplace_insertion_sort` — identical for every totally ordered
type, the exact opposite whenever either side is `NaN` — is caught in 300 cases (0.4s), shrunk to a
two-element array containing `NaN`. Both seeds are committed in
`crates/difffuzz/proptest-regressions/sort.txt` with a provenance block saying they came from
sabotages and not from real port defects. Full record: evidence file.

**Falsification of the port (gate 6, on the original suite), separate and cruder, is what proves
the original test file exercises Rust rather than a JS fallback:** deleting the
`array.swap(j - 1, j)` line from `mnemonist_core::sort::insertion::inplace_insertion_sort` breaks
`test/sort.js`'s "insertion → should properly sort inplace." (13 passing → 9 passing, 4 failing,
the named assertion among them); reverted, back to 13 passing. All six `quick` blocks and both
`insertion` **indices** blocks stayed green throughout, because they are a different code path —
a falsification that had targeted a shared helper would have gone red everywhere and told us less.
Full record: evidence file.

### Bench

`bench/results.json` → `modules["sort"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, 500 samples/side.

`sort` has no instance and no per-element op stream, so there is nothing for a K = 1000 batch to
mean here — an "op" is "sort one freshly-generated array", not one comparison. This reuses the
`drain` shape instead: one measured sample per **sort**, the same convention `sparse-set`'s
iteration walk uses for an operation that is not a stream of cheap calls. `inplaceQuickSort` was
chosen over `inplaceInsertionSort` as the representative default — the general-purpose sort most
callers of this module actually use.

**`sort-2e4x50`** — quicksort of a freshly-generated 20,000-element random array (values 0..1e6),
50 passes, xorshift32 seed 42. Every pass sorts *fresh* data, generated once for all 50 passes
before any timing and never re-sorted: upstream's fixed-pivot quicksort's worst case is
already-sorted input, and re-sorting the previous pass's (now-sorted) output would have silently
turned an O(n log n) benchmark into an O(n²) one on every pass after the first — the same shape of
mistake `bit-set`'s `rank` was. The checksum is **position-weighted** (`Σ (index+1) × value`) rather
than a sum, because a sum cannot distinguish a correctly sorted array from an unsorted one of the
same multiset; weighting by final index makes it sensitive to quicksort's own (non-stable)
tie-breaking, so checksum agreement is evidence both sides ran the identical statement-by-statement
algorithm BUG-SORT-2's docs describe, not merely that both produced *a* sorted array.

The port is 2.5× faster at p50 (31.9 vs 78.4 ns/element), 2.4× faster at p99, 2.5× faster at min. No
regressions. Full table: evidence file. `structure_rss_delta_mb` (port 0.1 MB, upstream 5.8 MB) is a
different kind of number here from the other ten modules': there is no persistent structure left
after the call returns, so it measures the transient footprint of allocating and sorting one
20,000-element array — read it as "memory to hold and sort `size` elements", not "size of the sort
structure".

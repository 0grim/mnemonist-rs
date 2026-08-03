# hashed-array-tree

Upstream: `hashed-array-tree.js` (209 LOC) + `utils/typed-arrays.js` (only the width selection is
reachable) · `test/hashed-array-tree.js` — **114 lines, 10 `it` blocks, 24 assertion statements**.

Port: `crates/mnemonist-core/src/structures/hashed_array_tree.rs`.
Bridge: `crates/mnemonist-napi/src/hashed_array_tree.rs`.
Shim: `tests/bridge/hashed-array-tree.js`.

Chosen first among this group because it needs **no new primitive** — `PointerVec` and nothing else — so it
tests the pipeline rather than the machinery. It turned out to be the module with the most
upstream defects per line so far: two, both structural, both invisible to a test file that never
leaves the first block.

---

## What upstream tests

Ten `it` blocks. Characterising the shape rather than restating them:

* **The default block size, or a tiny one used only for capacity arithmetic.** `blockSize: 128` and
  `blockSize: 2` appear, but the `2` case only ever calls `grow`, and the `128` case only reads
  index 34 — inside the first block. **No assertion in the file ever reads or pops an element that
  lives outside block 0.**
* **Two array classes, `Uint8Array` and `Uint32Array`**, and no value above 250, so nothing
  truncates.
* **The two throws are matched loosely**: `/hashed-array/` and `/power of two/` and `/bounds/`.
* `pop` is exercised on a tree that holds at most two elements in one 1024-element block.
* The out-of-bounds read is checked at index `2` on a tree of length `0`.

## What upstream does NOT test

**The block indexing itself — which is the entire point of the structure**

1. **No element outside block 0 is ever read back.** `array.get(34)` after 250 pushes into
   128-element blocks is the only read, and 34 is in the first block. So the `index >> blockMask` /
   `index & offsetMask` split is never checked against an index that needs it.
2. **`pop` never crosses a block boundary**, which is why defect BUG-HASHED-ARRAY-TREE-1 below survives.
3. **`blockMask` and `offsetMask` are never asserted**, only `capacity`.
4. **A `blockSize` of 1** — every element its own block — is never constructed.

**The off-by-one in both bounds guards**

5. **`get(length)` is never called.** The guard is `this.length < index`, not `<=`, so
   `index === length` is admitted. Upstream's own "should return undefined on out-of-bound values"
   test would have caught this had it asked for index `0` instead of index `2` on its length-0 tree.
6. **`set(length, v)` is never called**, so the write that lands without moving `length` is unseen.
7. **`get`/`set` at `index === capacity` is never called**, which is where upstream raises a
   `TypeError` from indexing a block that does not exist.

**Growth and shrink interactions**

8. **A shrinking `resize` is done once and never followed by anything.** `resize(20)` from 23 is the
   only shrink, and the test then only grows again. That the blocks and their *contents* survive —
   and are re-exposed by a later `resize` up, or reachable by `pop` — is untested.
9. **`push` after a shrinking `resize`** is never done.
10. **`grow()` with no argument on a tree that has never allocated** is never done.
11. **`resize` to the current length** (the early-return branch) is never done.

**Values**

12. **Truncation is never triggered.** The largest value pushed is 249, into a `Uint8Array`.
13. **`Uint16Array` is never used**, so only two of the three widths are seen.

**Constructor**

14. **`initialLength` and `initialCapacity` are never given together**, so `Math.max` of the two is
    unexercised.
15. **Only `27` is rejected** as a block size. The guard is a ToInt32 test and its boundaries are
    untouched.
16. **`new HashedArrayTree(undefined)` passes upstream's `arguments.length` check** and leaves
    `ArrayClass` undefined — never tested.

**Never called at all**

17. `inspect()` and the `nodejs.util.inspect.custom` symbol — ~22 LOC, a fifth of the module.

## What we test in addition

`crates/mnemonist-core/src/structures/hashed_array_tree.rs` — 15 tests, closing every gap above
except 16 and 17: a 1:1 reproduction of all ten upstream blocks as a baseline, BUG-HASHED-ARRAY-TREE-1 pinned value by
value against Node (both directly and reached via a shrinking resize), BUG-HASHED-ARRAY-TREE-2's read and write halves,
the exact `TypeError` at `index === capacity` across all three widths, truncating stores, the index
split constants derived from the block size, a bare `grow` adding exactly one block, a shrinking
resize keeping its blocks and contents, a push after a shrinking resize overwriting the stale slot,
`initialLength`/`initialCapacity` combined across seven cases, every non-power-of-two block size
rejected (and the ToInt32 boundary at 2^32), a block size of one giving every element its own block,
and indexing across block boundaries. Full test-to-gap mapping: evidence file.

The **differential fuzzer** then covers gaps 1–14 continuously. Block sizes are drawn from
`{1, 2, 4, 8}` rather than the 1024 default precisely because upstream's coverage gap is "never
leaves block 0"; at `blockSize: 2` a 200-op program is almost entirely cross-block. Indices run to
64 against lengths that rarely exceed 30, so roughly a third of generated `set`s are out of bounds
and a steady trickle land exactly on `length`.

**Still untested, stated rather than glossed:** gap 17 (`inspect`, not ported — a Node display
convenience with no upstream assertion), gap 16 in its `arguments.length` form (see the divergence
table), and non-integer lengths, which `usize` cannot hold.

## Bugs this found

**BUG-HASHED-ARRAY-TREE-1 — `pop` reads the last *block*, not the block holding the popped index.**
Verified against Node 24.18.1. The sharpest defect in the file:

```js
var lastBlock = this.blocks[this.blocks.length - 1];   // the LAST block
var i = (--this.length) & this.offsetMask;             // offset of the POPPED index
return lastBlock[i];
```

The offset is computed from the popped index; the block is taken unconditionally from the end of
`blocks`. They agree only while the tree occupies a single block — which is the whole of upstream's
coverage, since its `pop` test uses the 1024-element default and pushes twice. Measured on Node with
`blockSize: 2` after pushing `1, 2, 3`:

```js
blocks === [[1, 2], [3, 0]]
pop()  // 3   -- index 2, offset 0, last block: right by luck
pop()  // 0   -- index 1, offset 1, last block: reads the padding
pop()  // 3   -- index 0, offset 0, last block: yields 3 a second time
```

The `2` is unreachable and the `3` comes back twice. `length` is decremented correctly throughout,
so only the return value is wrong — which is why nothing downstream notices. A shrinking `resize`
reaches the same defect without any growth at all, because `resize` never deallocates:
`push 7,8,9,10; resize(1); pop()` gives `9`, not `7`.

**BUG-HASHED-ARRAY-TREE-2 — the `set`/`get` bounds guard is `length < index`, admitting `index === length`.**
Verified against Node 24.18.1. Three consequences:

* `get(length)` returns the raw block slot rather than `undefined`. A **brand-new tree answers
  `get(0)` with `0`**, not `undefined` — the value upstream's own out-of-bounds test asserts, one
  index away.
* `set(length, v)` **writes**, and `length` does not move. The value is invisible to `pop` and to
  any subsequent `push` (which overwrites it), but visible to `get(length)`.
* When that admitted index is also `capacity`, `blocks[capacity >> blockMask]` is `undefined` and
  upstream raises `TypeError: Cannot set properties of undefined (setting '0')`. So the same guard
  produces a silent write, a silent read and a hard throw depending on where `length` happens to sit.

Both are reproduced, not fixed. Fixing either would change values a caller can observe.

**Not a bug, but worth recording: `powerOfTwo` runs on a ToInt32.** `(x & (x - 1)) === 0` converts
both operands to signed 32-bit, so `blockSize: 2**32` **passes** the guard and yields
`blockMask === 32`. In JavaScript `index >> 32` is `index >> 0`, so the structure silently stops
being a hashed array tree. Verified on Node: the constructor succeeds. This port reproduces the
acceptance in `power_of_two` — so the truncation is visible and tested — and then refuses the size
on the next line rather than misrepresenting the shift.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **`ArrayClass` is a `PointerWidth`, not a constructor.** | Rust has no runtime constructor value. The bridge identifies the class by its `name` and maps it; `Uint8Array`/`Uint16Array`/`Uint32Array` are covered, and anything else is refused rather than reinterpreted. Upstream would happily take `Float64Array`. The class name survives because the `set` error message embeds it. |
| — | **`blockSize >= 2^31` is refused.** | Upstream accepts `2**32` by ToInt32 truncation and then shifts by `blockMask mod 32`. Reproducing that would mean reproducing a structure that does not index. The acceptance is reproduced in `power_of_two` and pinned by a test; the *use* is refused. |
| — | **`new HashedArrayTree(undefined)` throws.** | Upstream's guard is `arguments.length < 1`, and napi's typed signature cannot tell an omitted argument from one passed as `undefined`. Upstream leaves `ArrayClass` undefined and only fails later, if it ever allocates. The omitted case — the only one the original suite uses — is exact. |
| — | **Non-integer lengths are truncated.** | `resize(3.5)` really does leave `length === 3.5` on Node. A `usize` cannot hold it; truncating toward zero is the closest honest reading. No upstream call site or test passes one. |
| — | **`blocks` is not exposed to JS.** | A public array of typed arrays upstream, writable *through*. napi can only hand out a copy, which would silently break write-through — worse than its absence. Same call as the `SparseSet` bridge makes for `dense`/`sparse`. It is exposed in Rust, and the fuzzer compares it block for block after every op. |
| — | **`get`/`pop` yield `Either<u32, Undefined>`, not `Option<u32>`.** | DIV-FIXED-STACK-1, re-learned here: napi renders `None` as `null`, and both `assert.strictEqual(…, undefined)` assertions in the original file fail against `null`. This is the second module to hit it; it is now a checklist item, not a discovery. |
| — | **`Error` is an enum whose `Display` is upstream's message.** | The `set` message embeds the array class and the `TypeError` embeds the block offset, so a `&'static str` could not carry either. `Display` renders exactly what upstream throws, which is also what makes the fuzzer's `$throw` comparison meaningful. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion and no Rust equivalent. |
| — | **The V8 `TypeError` text is reproduced verbatim.** | `Cannot set properties of undefined (setting '0')` is V8's phrasing, not the language's. Reproducing it is what lets the fuzzer compare thrown messages in full; it also ties these campaigns to Node 24.18.1, which is stated rather than hidden. |

## Fuzz + bench

### Fuzz

Two campaigns, two seeds, **2.15 M operations, zero divergences**:

```
module=hashed-array-tree seed=42       cases=14429 ops=1429629 wall=120.0s divergences=0
module=hashed-array-tree seed=20260801 cases=7319  ops=723058  wall=60.0s  divergences=0
```

Reproduce with `target/release/difffuzz --module hashed-array-tree --seed 42 --cases 14429`.

The op alphabet covers `push`/`pop`/`set`/`get`/`grow`/`resize`. Observable state is `length`,
`capacity`, `blockSize`, `offsetMask`, `blockMask` and **`blocks`** — every block, slot for slot —
which is what makes the truncating stores, the `set(length)` write and "a shrinking resize
deallocates nothing" checkable directly rather than only through their eventual effect on `get`.
Constructors draw all three widths × block sizes `{1, 2, 4, 8}` × `initialLength`/`initialCapacity`
each `0..24`. Indices run `0..64` against lengths that rarely exceed 30; values run `0..320`, above
255 so a `Uint8Array` tree truncates. Deliberately excluded: non-integer arguments (see the
divergence table) and `blockSize` values outside `{1, 2, 4, 8}` — nothing else, and in particular
out-of-bounds indices are generated freely, with both throws compared by their full message. Full
grammar: evidence file.

**The `$throw` encoding was added for this module.** `spec::CheckFailure` carried a note, written
before any such module existed, that an exception thrown by an operation arrives as apparatus
failure and would abort the campaign rather than being reported — and that the fix is to encode it
on both sides. `fuzz/oracle.js` now does. Sabotage B below is the proof that it works, because it is
a divergence *in the throwing itself*.

**The fuzzer was falsified twice, once per defect, and both make the port strictly *more correct*
than upstream — the only direction differential fuzzing can work in on a bug-for-bug port.**
Sabotage A is the obvious `pop` cleanup, reading `blocks[length >> blockMask]` instead of the last
block; caught in 1,228 cases (1.8 s), shrunk from 200 ops to five. Sabotage B tightens `set`'s guard
from `<` to `<=`, what anyone tidying the bounds check would write; caught in 2,165 cases (5.2 s),
shrunk to ten ops ending in a divergence in the thrown message itself. Both reverted; both seeds
committed with provenance in `crates/difffuzz/proptest-regressions/hashed-array-tree.txt`. Full
repro code: evidence file.

**Falsification of the port (gate 6):** the first attempt failed the gate's own standard, and is
recorded because that is the point of the gate — a sabotage of `push`'s growth guard went red, but
at a different assertion than the one named beforehand, which is weaker evidence than going red
where predicted, since it shows the suite runs Rust but not that the named assertion depends on the
named code. Redone: the assertion named second was `should be possible to pop values.` —
`assert.strictEqual(array.pop(), 2)` at `test/hashed-array-tree.js:71` — chosen because `pop` is the
one method whose defect this module exists to reproduce. The sabotage, mis-porting the offset
computation as a post-decrement rather than pre-decrement, is confirmed red at exactly the named
line (9 passing, 1 failing, `0 !== 2`); reverted, confirmed green again (10 passing). Full record:
evidence file.

### Bench

`bench/results.json` → `modules["hashed-array-tree"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`get`/`pop` (50/25/25), `vector`'s exact shape (`get` at a
uniformly random *existing* index, modulo the current length), `Uint32Array` blocks at the default
1024-element block size: the port is 1.6× faster at p50 (5.5 vs 8.6 ns/op), 2.6× faster at p99
(10.0 vs 26.4), 1.7× faster at min. No regressions. Full table: evidence file.

`pop`'s upstream defect (BUG-SPARSE-MAP-1, reading the last allocated block rather than the block the popped
index actually falls in) is reproduced bug-for-bug and contributes to the checksum exactly as
everything else does — the checksum matching on both sides is itself evidence the port takes the same wrong
branch upstream does, at the same block boundaries, not merely that both sides returned *a* number.

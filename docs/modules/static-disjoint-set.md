# static-disjoint-set

Upstream: `static-disjoint-set.js` (195 LOC) + `utils/typed-arrays.js` (187 LOC, only
`getPointerArray` reachable) · `test/static-disjoint-set.js` — **34 lines, 1 `it` block,
6 assertions**.

Port: `crates/mnemonist-core/src/structures/static_disjoint_set.rs`,
`crates/mnemonist-core/src/utils/typed_arrays.rs`.
Bridge: `crates/mnemonist-napi/src/static_disjoint_set.rs`.

This is the smallest unit in the repo and the one with the largest coverage gap. That combination
is why it was chosen to shake out the fuzz and bench harnesses first.

---

## What upstream tests

The entire suite is one `it` block. It builds a set of 10, performs six unions, and asserts six
things:

```js
var sets = new StaticDisjointSet(10);
sets.union(0, 1); sets.union(1, 5); sets.union(0, 7);
sets.union(8, 9);
sets.union(2, 3); sets.union(2, 4);

assert.strictEqual(sets.size, 10);
assert.strictEqual(sets.dimension, 4);
assert.strictEqual(sets.connected(1, 7), true);
assert.strictEqual(sets.connected(6, 0), false);
assert.deepStrictEqual(Array.from(sets.mapping()), [0, 0, 1, 1, 1, 0, 2, 0, 3, 3]);
assert.deepStrictEqual(Array.from(sets.compile(), a => Array.from(a)), [[0,1,5,7],[2,3,4],[6],[8,9]]);
```

Characterising the shape of that coverage:

* It is **partition-only**. Every assertion is a function of *which items share a set*. Nothing
  observes *which item is the root*, how deep any tree is, or what the internal arrays hold.
* It is **single-shot**. One size, one op order, one call each to `mapping` and `compile`, both at
  the end.
* `Array.from(...)` on both aggregate results **erases the return type**, so the test cannot
  distinguish a `Uint8Array` from a `Uint32Array` from a plain `Array`.

## What upstream does NOT test

This is the section that carries the weight. Everything below is reachable through the public API
and never exercised by the original suite.

**Whole methods and branches**

1. **`find()` is never called.** The module's central method — the one containing both the root
   walk and the path-compression loop — has zero direct assertions. It is exercised only as an
   internal detail of `union`, `connected`, `mapping` and `compile`.
2. **Path compression is never observed.** No assertion can distinguish a compressed forest from
   an uncompressed one, so the entire second `while (true)` loop in `find` could be deleted and the
   suite would stay green.
3. **The `xRoot === yRoot` early return in `union` is never taken.** All six unions merge distinct
   sets. This is not a hypothetical: it is the exact branch the first falsification attempt
   sabotaged, which is why gate 6 stayed green and proved nothing.
4. **Self-union `union(x, x)` is never performed.**
5. **`dimension` is asserted once, at the end.** That it does *not* decrement on a no-op union is
   never checked — which follows from (3).
6. **`union`'s return value is never used.** Upstream returns `this` for chaining; nothing asserts
   it, so the chaining contract is unverified.
7. **`inspect()` and the `nodejs.util.inspect.custom` symbol are never called.** ~15 LOC of the
   module, untouched.

**The entire typed-array width machinery**

8. **Only one size is ever constructed: 10.** `getPointerArray(10)` returns `Uint8Array`, and
   `getPointerArray(Math.log2(10))` also returns `Uint8Array`. So:
   * The 16-bit and 32-bit branches of `getPointerArray` are **never reached** through this module.
   * `parents` and `ranks` never differ in width, so the fact that upstream deliberately sizes them
     from two different expressions is never distinguished from a copy-paste.
   * The `size > 2^32` throw is never reached.
9. **`mapping()`'s width is chosen at call time from `this.dimension`**, and is observable in JS via
   `constructor.name` / `instanceof`. `Array.from` in the test throws that away. A `mapping()` that
   returned a plain `Array` would pass.
10. **Typed-array write truncation is never triggered.** Reaching it needs a rank above 255, i.e.
    hundreds of unions on one root. Upstream's largest set is 10.

**Degenerate and boundary inputs**

11. `new StaticDisjointSet(0)` — `Math.log2(0)` is `-Infinity`, which still selects the narrowest
    width. Untested.
12. `new StaticDisjointSet(1)` — the single-element set. Untested.
13. Out-of-range indices. `find(1e9)` reads past the typed array, gets `undefined`, and propagates
    `NaN` through the parent walk. Untested.
14. Non-integer, negative, or missing constructor arguments. Untested.

**Sequencing and the mutating reads**

15. **`mapping()` and `compile()` both mutate.** Each calls `find` on every item, so each rewrites
    `parents` via path compression. Calling either one *twice*, or interleaving them with unions,
    is never done — so the side effect is never observed, and neither is its idempotence.
16. Neither is called on a fresh set (all singletons) or on a fully merged one.

**The consequence worth stating plainly**

17. **The BUG-STATIC-DISJOINT-SET-1 rank bug is structurally invisible to this suite.** Because every assertion is
    partition-only (see above), and union-find produces the correct partition regardless of which
    tree is attached to which, no possible assertion in this file could detect that upstream reads
    `ranks[x]` where it means `ranks[xRoot]`. The suite is not merely thin here; it is the wrong
    *kind* of test to find this class of defect.

## What we test in addition

Rust native tests in `crates/mnemonist-core/src/structures/static_disjoint_set.rs`, closing every
gap above except 7 and 14: a 1:1 port of the upstream `it` as a baseline, a singleton set, path
compression pinned by hand on a 4-deep chain, that `dimension` only drops on a successful union
(the branch that made gate 6 a false green the first time it was tried), an empty set, a distinct
width per array at size 300, `mapping()`'s width narrowing as unions accumulate, rank wrapping at
the `Uint8Array` boundary, `mapping`/`compile` agreeing and being called twice, a concrete input
that pins the upstream rank bug, the width-selection throw, and an out-of-range `find` panicking
rather than silently propagating `NaN`. Full test-to-gap mapping: evidence file.

The **differential fuzzer** then covers gaps 1–6, 8, 9, 15, 16 and 17 continuously rather than at
hand-picked points, because `mapping` and `compile` are in the observable-state set and therefore
run after *every* operation of *every* generated program. Gap 6 in particular is checked on every
`union`: the oracle encodes `this` as `{"$self": true}` and the port must return the same.

**Still untested, stated rather than glossed:** gap 7 (`inspect`, not bridged — it is a Node
display convenience with no upstream assertion) and gap 14 (the napi bridge types the constructor
argument as `u32`, so non-integer and negative inputs are rejected by napi's own coercion before
any port code runs; upstream's behaviour on those inputs is therefore not reproduced and not
compared).

## Bugs this found

**BUG-STATIC-DISJOINT-SET-1 — `union` compares the ranks of the items, not the roots.** Upstream reads `this.ranks[x]` /
`this.ranks[y]` where union-by-rank requires `this.ranks[xRoot]` / `this.ranks[yRoot]`, while
incrementing `this.ranks[xRoot]`. Non-root ranks are therefore never maintained, stay 0 forever, and
the equal-ranks branch fires on nearly every union — disabling the heuristic and degrading `find`
towards O(n). Results stay correct; the elected root does not.

**DIV-STATIC-DISJOINT-SET-1 — the second-order consequence, which is the more interesting half.** Because BUG-STATIC-DISJOINT-SET-1 makes the
equal-ranks branch near-universal, one root's rank is bumped once per union, far past the
`log2(size)` the array was sized for. And `ranks` is *always* a `Uint8Array` in practice: widening
it would need `log2(size) > 256`, and `parents` already caps `size` at 2³². So it **wraps** — a
300-element set ends with `ranks[0] == 43`. Verified against real Node, which agrees exactly.

A naive `Vec<u32>` port diverges here silently, and **no upstream test catches it**, because
upstream never builds a set large enough. Two defects compounding — an upstream logic error making
an otherwise-unreachable overflow reachable — is the strongest single argument for differential
testing in this port so far, and neither half is visible from reading one file.

**What the fuzzer found: nothing new.** Stated plainly because the honest result matters more than
the flattering one. Two campaigns, 2.10 M operations across 6,984 distinct programs, zero
divergences.

That is the expected outcome and not a failure of the fuzzer, for a reason worth recording: **a
faithful port reproduces upstream's bugs, so differential fuzzing structurally cannot find them.**
BUG-STATIC-DISJOINT-SET-1 was found by reading. What the fuzzer is actually for on this module is the other direction —
catching the port drifting away from upstream, including drifting towards *correctness* — and it
is proven to do that (see below).

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| DIV-STATIC-DISJOINT-SET-2 | **The rank bug is reproduced, not fixed.** | `find(x)` returns a root and which item becomes root is observable. "Fixing" it would be a silent behavioural divergence. Pinned by `reproduces_upstream_rank_bug` so a future cleanup fails the suite. |
| DIV-STATIC-DISJOINT-SET-1 | **`PointerVec` masks every write to the selected width.** | Selecting the right width is only half of typed-array semantics; the truncating *write* is the other half. A plain `Vec<u32>` would diverge at rank 256. |
| — | **`find`/`connected`/`mapping`/`compile` take `&mut self`.** | They all path-compress. Rust's type system forces the mutation to be declared where JS hides it — arguably an improvement in legibility, and it costs the caller nothing. |
| — | **`union` returns `bool`; the bridge returns `this`.** | Core reports whether a merge happened, which upstream exposes only through `dimension`. The bridge drops it so the JS surface matches exactly. |
| — | **`mapping()` returns `Mapping { width, values }`.** | Rust has no runtime array constructor. The chosen width travels with the values and the bridge rebuilds the matching `Uint8Array`/`Uint16Array`/`Uint32Array`, so the JS-observable type is preserved. |
| — | **`new()` returns `Result`; out-of-range indices raise `RangeError`.** | Upstream throws for the first and returns `undefined`-driven garbage for the second. There is no honest Rust reproduction of `NaN` arithmetic on array indices, so the bridge raises instead of inventing a value. Documented in the napi module docs, adaptation 3, and excluded from the fuzz grammar for that reason. |

## Fuzz + bench

### Fuzz

Two campaigns, two seeds, **2.10 M operations across 6,984 distinct programs, zero divergences** —
comfortably past gate 9's 60-second floor:

```
module=static-disjoint-set seed=42       cases=4683 ops=1403272 wall=120.1s divergences=0
module=static-disjoint-set seed=20260731 cases=2301 ops=700097  wall=60.0s  divergences=0
```

Reproduce exactly with `target/release/difffuzz --module static-disjoint-set --seed 42
--cases 4683`. Full grammar detail and the campaign-count correction history: evidence file and log.

Throughput is ~11,700 op/s including a full `mapping()` + `compile()` comparison after every single
op, which is the persistent-oracle decision paying off: at one `node` spawn per op the same
campaign would have taken roughly 16 hours.

The op alphabet covers `union`/`find`/`connected`, with `mapping()`/`compile()` as *observations*
rather than ops — both call `find` on every item, so path compression is exercised on every step of
every program rather than when the generator happens to pick it. Sizes run 1..=400, straddling 256
so the `parents` 8→16-bit switch is generated and large enough for the rank wrap to be reachable.
Deliberately excluded from the grammar: out-of-range indices (see the divergence table above).
Nothing else is excluded. Full grammar: evidence file.

**The fuzzer was falsified before it was trusted.** A fuzzer that has never been seen to fail is
the same problem gate 6 exists to prevent. The sabotage chosen was to *fix* BUG-STATIC-DISJOINT-SET-1 in the core — the
most plausible way this port could realistically break, since it makes the port strictly more
correct than upstream and therefore wrong. It was caught in **129 cases (0.3 s)** and shrunk from a
600-op program to three operations. The sabotage was reverted; the seed is committed in
`crates/difffuzz/proptest-regressions/static-disjoint-set.txt` with a provenance header, and
proptest replays it before any novel case on every subsequent run. Full repro: evidence file.

### Bench

`bench/results.json` → `modules["static-disjoint-set"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, 32 MB L3, WSL2, Node 24.18.1, rustc 1.97.1.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `union`/`find`/`connected` (50/25/25) over size 1e6: the port is 1.5×
faster at p50 (15.6 vs 22.6 ns/op), 2.0× faster at p99 (34.5 vs 68.5). **`mixed-4e6`** — the same
mix at four times the size: 2.0× faster at p50 (21.8 vs 42.9), 3.1× faster at p99 (43.6 vs 134.9).
No regressions on either workload. Full tables: evidence file.

`PointerVec` gives each logical width its own real backing store (`Vec<u8>`/`Vec<u16>`/`Vec<u32>`,
where the narrowing cast *is* the truncation), matching upstream's own `Uint8Array ranks` /
`Uint32Array parents` split rather than backing every width with a `Vec<u32>` — see the log for the
regression this fixed and the hypothesis that turned out to explain it.

The leading hypothesis for the remaining p99 gap is **address-space stride rather than resident
size**: at a wider type the same logical indices span a larger address range, so random reads touch
more pages, and TLB pressure is exactly the kind of cost that lands in the tail rather than the
median. **This is unconfirmed** — confirming it needs `perf stat -e dTLB-load-misses` on both
revisions, which has not been run. It is recorded as a hypothesis, not a finding.

**Methodological caveat, and why interleaving earned its place.** Upstream's own p99 measured
102.1 ns/op in one run and 134.9 in another on the same host — a 32% swing from ambient load alone.
Absolute ns/op are therefore not comparable across runs; only the within-run A/B comparison is
sound, which is precisely what the interleaving requirement protects. The smaller ratios in these
tables should be read as "roughly 2×", not as three significant figures.

`bench/drive.js` derives the `regressions` array mechanically from the published metrics, so a
future regression cannot be quietly dropped from a run.

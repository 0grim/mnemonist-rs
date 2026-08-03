# multi-array

Upstream: `multi-array.js` (448 LOC) — depends on `utils/typed-arrays.js` and `vector.js` (already
ported as their own units) · `test/multi-array.js` — **238 lines, 12 `it` blocks**.

Port: `crates/mnemonist-core/src/structures/multi_array.rs` — `MultiArray`, an array-of-arrays
represented as one flat item array plus a singly-linked-list-per-bucket threaded through it (no
per-bucket allocation). Bridge: `crates/mnemonist-napi/src/multi_array.rs`. Shim:
`tests/bridge/multi-array.js`. Fuzz spec: `crates/difffuzz/src/modules/multi_array.rs`.

`test/multi-array.js`'s require-closure pulls in `vector.js`'s own `PointerVector`/typed-array
machinery only incidentally (`multi-array.js` itself uses it internally for `tails`/`lengths`); the
test file never constructs a `Vector` directly, so nothing extra was needed beyond `vector` already
being ported as its own unit.

---

## What upstream tests

* **`set`/`push`**, both the default (growable, exact-value) container and a fixed-capacity
  `Uint8Array` container, including the capacity-exceeded throw (`assert.throws(..., /capacity/)`)
  once `push` runs past a fixed capacity.
* **`get`**, past `dimension` (`undefined`) and within it (the bucket, in insertion order), both
  container kinds.
* **`has`**/**`multiplicity`**/**`count`** (an alias of `multiplicity`).
* **Inserting out of order** — `set(34, ...)` then `set(2, ...)` on a fresh instance, checking that
  `dimension` jumps to 35 and that the two set indices hold the right buckets, for both container
  kinds.
* **Five iterator factories** — `containers()`, `associations()`, `values()` (with and without an
  index argument), `entries()`, `keys()` — each drained with `obliterator/take`.
* **The `Uint8Array` constructor + capacity** combination end to end (add, overflow, get).

## What upstream does NOT test

**A fixed-capacity `Array` container, or an unbounded (no-`capacity`) typed-array container.**
`hasFixedCapacity` is driven purely by whether a truthy `capacity` was passed, independent of
`Container` — so `new MultiArray(Array, 10)` and `new MultiArray(Uint8Array)` are both
syntactically constructible, and neither is exercised. The second would in fact break real upstream
on first use (`this.items = new Uint8Array(); ...; this.items.push(item)` — typed arrays have no
`.push`). See "Deliberate divergences" (D-450).

**`get(index)` for an index below `dimension` that was never actually `set`** — the "insert out of
order" test only ever reads the two indices it wrote, never one of the gap indices in between. By
upstream's own arithmetic such a read returns an empty array (`[]`), not `undefined`; this port
reproduces that (`inserting_out_of_order_leaves_a_real_gap_at_dimension`) without upstream's own
suite ever checking it.

**A fixed-capacity container's value overflow.** The one `Uint8Array` test in the file never pushes
a value past 255, so the truncating `ToUint32`-then-narrow store this port reproduces
(`fixed_capacity_values_narrow_to_their_width`) is untested upstream too.

**A container mutated from inside a walk over it.** No test drains one of the five iterators while
also calling `set`/`push`. Upstream's own iterators close over `this.dimension`/a bucket's `length`
at creation, matching this port's snapshot-at-creation cursors (see the core module's docs and
`crates/mnemonist-napi/src/multi_array.rs`'s bridge docs) on every input either can reach — untested
in the direction that would distinguish "snapshot" from "genuinely live."

## What we test in addition

`crates/mnemonist-core/src/structures/multi_array.rs` — 11 tests, closing every gap above: a
baseline reproduction of the `set`/`push` walkthroughs, `get`/`has`/`multiplicity` for both
container kinds, the untested gap-read case, all five iterator factories checked directly against
literal expected sequences (including the forward-vs-reverse order contrast between `get` and
`values(index)` — the sharpest place a transcription error would hide), the untested
overflow-truncation case, and the empty-instance baseline. Full test-to-gap mapping: evidence file.

**Differential fuzzer** — a ten-index pool shared by `set`/`push`/`get`, alternating dynamic and
fixed-capacity (small capacities) constructors; see "Fuzz + bench" for measured numbers, including a
`grammar_self_check` counting multi-value buckets and capacity-exceeded throws directly.

**Still untested, stated rather than glossed:** a container mutated mid-walk (see above), and the
two `(Container, capacity)` combinations D-450 records as out of scope.

## Bugs this found

No upstream defect found in this unit.

One defect in this port's own bridge, found by the differential fuzzer's very first short smoke run
(before any full campaign was logged) and fixed before this unit was committed as complete:
`containers`/`associations`/`values`/`entries`/`keys` returned a plain `Vec`/`Array` instead of a
genuine iterator object, an API-*shape* divergence from upstream's real `obliterator` `Iterator`
that the original test suite could not catch (`take()` accepts either shape via duck-typing) but
that a direct comparison of `s.containers()`'s raw return value caught immediately: `[]` from the
port against `{}` from upstream. Fixed by building a real `#[napi(iterator)]` generator for each of
the five, matching `multi_map.rs`'s and `vector.rs`'s own established pattern. See
`crates/mnemonist-napi/src/multi_array.rs`'s module docs.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-450 | **Only two of the four syntactically-possible `(Container, capacity)` combinations are modelled**: the default/any-`Container` unbounded form, and a `Uint8Array`/`Uint16Array`/`Uint32Array` + truthy-`capacity` fixed form. A fixed-capacity `Array` and an unbounded typed array are refused with a message naming the supported set. | `test/multi-array.js` exercises exactly the two modelled combinations. The other two are not meaningfully supported by real upstream either — see "What upstream does NOT test." |

`capacity || null` (any JS-falsy `capacity`, including `0` and `NaN`, means dynamic mode, not just
an omitted argument) is reproduced exactly by `crates/mnemonist-napi/src/multi_array.rs`'s
`truthy_capacity`, and is a fidelity point, not a divergence — noted here because it is the kind of
detail a straightforward reading of the constructor would miss.

## Fuzz + bench

### Fuzz

Two campaigns, two seeds, **1.67M operations, zero divergences** — after the bridge fix above; the
campaigns logged are the clean, post-fix runs:

```
module=multi-array  seed=42        cases=8583 ops=834584 wall=60.0s divergences=0
module=multi-array  seed=20260802  cases=8567 ops=836499 wall=60.0s divergences=0
```

Reproduce with e.g. `target/release/difffuzz --module multi-array --seed 42 --cases 8583`.

The op alphabet weights `set`/`push` heaviest, since they are the only ops that grow a bucket or
exercise the capacity throw; `get`/`has`/`multiplicity` round it out. `containers`/`associations`/
`values`/`entries`/`keys` are deliberately **not** in the alphabet — all five now return a genuine
opaque iterator on both sides, which the oracle's `encode()` reduces to `{}` regardless of what is
actually inside, so comparing them can only ever agree trivially. The index pool is ten indices,
small enough that `set`/`push` collide on the same bucket constantly. Constructors alternate
between the default dynamic container and a fixed-capacity typed container with a small capacity, so
a `push`/`set` past capacity is common rather than rare. Observable state is `size`, `dimension`,
plus `get`'s own return value compared per-op. Full grammar: evidence file.

**Direct evidence the grammar reaches the states this campaign is for** — `grammar_self_check`, 400
generated programs, up to 300 ops each: 49,449 steps with a multi-value bucket, 10,149
capacity-exceeded throws. Both floors are asserted in the test itself, so a future weighting change
that regresses this back toward "every bucket holds exactly one value" or "capacity is never
actually reached" fails loudly.

**Falsification of the port (gate 6):** the assertion named first was `test/multi-array.js`'s
"should be possible to get subarrays." — `assert.deepStrictEqual(array.get(0), [1, 2, 3]);`. The
sabotage, in `MultiArray::set`'s dynamic-mode branch, linked each new item to itself instead of to
the previous tail — breaking the backward-walk linked list every `get`/`values`/`entries` depends
on — is confirmed red at the named assertion (`get(0)` came back `[3, 3, 3]`, not `[1, 2, 3]`; six
of eleven native tests failed, and the real upstream mocha suite failed the same assertion with the
identical wrong values). Reverted; confirmed green again (11/11 native tests, 12/12 upstream `it`
blocks). Nothing was found to be blind — the sabotage broke exactly the mechanism it targeted (every
bucket read collapsed to repeating its most-recently-pushed value) and nothing else was needed to
catch it, in either instrument.

### Bench

`bench/results.json` → `modules["multi-array"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`get`/`multiplicity` (50/25/25), dynamic (unbounded, exact-`f64`)
container, over a 20,000-index domain deliberately far smaller than the op count, ~25 values per
bucket on average by the run's end (matching `multi-map`'s own ratio): the port is now 1.31× slower
at p50 (38.3 vs 29.81 ns/op) and 1.5× faster at p99 (177.5 vs 272.8). No regressions on p99/RSS/
startup. Full table and the investigation that took p50 from an initial 1.9× loss to the current
1.31×: evidence file and log.

**A split result: p50 loses, p99 wins, stated as both rather than either alone.** Reporting only
p99 here would have hidden a real p50 regression; reporting only p50 would have hidden that the
port's tail is *better* than upstream's, which is the more usual pattern for a GC'd runtime doing
frequent small allocations (`get` allocates a fresh array/`Vec` on every call, tail-to-head, and
does so on both sides).

`get` allocates exactly once per call, confirmed by counting rather than by reading the source — the
compiler does not elide it. The gap between `get` and `multiplicity` (an O(1) read with no walk and
no allocation) splits roughly 70/30 between the allocation itself and the pointer-chain walk. This
accounts for a bit over half of the whole-workload p50 gap by a back-of-envelope calculation, so it
is a real, substantial contribution but not shown to be the entire explanation — no probe was run
against `set` to check whether it also carries part of the gap. Recorded as confirmed-but-partial
rather than confirmed-and-complete, per this project's rule against overclaiming causation.

**The allocation stays.** `get`'s allocation is not incidental: it exists to match upstream's own
`#.get(index)` contract, which returns a fresh JS `Array` — a non-allocating alternative would have
to be an *additional* method (e.g. a walk that writes into a caller-supplied buffer or yields an
iterator), not a replacement for `get`, since the benchmark and the upstream test suite both need
`get`'s existing return type. Adding one is a `crates/mnemonist-core` change and would need
multi-array's fuzz campaign and bench figures re-run before it could stand, which puts it outside
this investigation's scope.

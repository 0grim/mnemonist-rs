# vector

Upstream: `vector.js` (373 LOC) + `utils/typed-arrays.js` + `obliterator/iterator` ·
`test/vector.js` — **234 lines, 18 `it` blocks, 53 assertion statements**.

Port: `crates/mnemonist-core/src/structures/vector.rs`.
Bridge: `crates/mnemonist-napi/src/vector.rs`.
Shim: `tests/bridge/vector.js`.

**Scope cut, stated up front:** upstream's `Vector` is generic over any array-like
`ArrayClass` — a real typed array, a plain `Array`, or a caller's own factory. `test/vector.js`
only ever constructs one with `Uint8Array`, `Uint32Array`, `Vector.Float64Vector` and
`Vector.PointerVector`, so this port models exactly those four backings
([`Storage::Fixed`]/[`Storage::F64`]/[`Storage::Pointer`] in the core crate). Signed and clamped
typed arrays, `Float32Array`, a plain `Array` backing and a caller `factory` are not modelled —
the same call `sparse-map`'s bridge makes for its `Values` constructor. Every finding below is
about the four backings the test file actually reaches.

---

## What upstream tests

Eighteen `it` blocks, and the shape of the coverage is narrower than the count suggests:

* **Construction, once each:** a missing-argument arity check, a dynamic vector's initial
  `length`/`capacity`, and the `{initialLength, initialCapacity}` options form.
* **`get`/`set`, exactly twice each:** one in-bounds pair (`set(2, 24)` on a length-3 vector) and
  one clearly out-of-range pair on each side — `get(2)` on a length-0 vector, `set(56, …)` on a
  length-4 one. Neither call is ever adjacent to a real boundary.
* **`push`/`pop`, by growth and by drain:** 250 sequential pushes on a `Uint8Array` capacity-5
  vector (checking `length`, the default-policy `capacity` it lands on, and one mid-range `get`),
  and a pop-to-empty-and-refill on a `Uint32Array`.
* **The growth machinery is well exercised on its own terms:** `reallocate` grown and shrunk,
  `grow(capacity)` and `grow()`, `resize` grown and shrunk, a custom policy used successfully, and
  a policy whose result is `<= capacity` asserted to throw (`/policy/`).
* **Subclasses:** `Vector.Float64Vector` once, `Vector.PointerVector` once (constructed, then
  pushed 500 times to see `array` become a `Uint16Array`).
* **Construction from an iterable:** `Vector.from` with and without an explicit capacity, and
  through a named subclass (`Vector.Uint8Vector.from`).
* **Iteration:** a `values()` iterator, an `entries()` iterator, and `for...of`, each drained by
  hand or in full — never interleaved with a mutation.

## What upstream does NOT test

**The one boundary every growth/read/write guard is built around**

1. **`index === length` is never probed**, on either `get` or `set`. Every in-bounds call in the
   file is comfortably inside the current length, and every out-of-bounds call is far past it.
   This is the precondition for B-101: the guard upstream actually wrote is `this.length < index`,
   not `<=`, and no assertion in the file would notice if that operator were flipped.
2. **A `pop` is never followed by a `grow`/`reallocate`/`resize` and then a read of the popped
   slot.** The one `pop`-then-`push` sequence in the file (`should be possible to pop values.`)
   only reads the freshly pushed positions afterwards, never the one just vacated. This is the
   precondition for B-102.
3. **Values are never pushed past their backing's truncation point.** The `Uint8Array` growth test
   pushes `0..250`, which never exceeds `255`; `Uint16Array` is never pushed past `65535` at all.
   Truncation on store is asserted nowhere.
4. **`Float64Array` never stores a non-integer.** The one `Float64Vector` test sets `24`, an
   integer that would round-trip through any backing.

**The growth policy, beyond the one throw it tests**

5. **A policy returning `NaN`, `Infinity`, or a negative number** is never exercised — only the
   "returns a value `<=` current capacity" throw (`PolicyTooSmall`) is. `applyPolicy`'s guard is
   `typeof newCapacity !== 'number' || newCapacity < 0`, which every non-finite number passes
   (every comparison against `NaN` is false, and `Infinity` is not `< 0`), so both propagate into
   an allocation call no test ever reaches.
6. **A policy returning something that is not a number at all** (a string, an object) is untested;
   the file's one custom policy always returns a number.

**Growth mechanics beyond the one path exercised**

7. **`reallocate`'s two branches — grow and shrink — are each tested once**, but never on a
   `PointerVector`, whose width is *re-derived* from the new capacity on every growth. Shrinking a
   `PointerVector` below a width boundary (say, from a `Uint16Array`-width capacity back under
   256) is never probed, so whether the width narrows back down is unstated by the suite.
8. **A zero-capacity `PointerVector`'s initial width** is asserted once, before any push; a
   *zero-length* one after some growth-and-shrink sequence is not.

**Iteration**

9. **A cursor is never re-drained.** `values()`/`entries()` are each exhausted exactly once; a
   second call to `.next()` after `done: true`, or opening a second iterator over the same vector,
   is not tested.
10. **Mutation during iteration** (pushing or popping mid-walk) is never performed.
11. **An empty vector's iteration** is never checked — every iterator test starts from
    `Vector.from([1, 2, 3], …)`.

**Never called at all**

12. `Vector.PointerVector.from`, and — because `test/vector.js` never sets an out-of-bounds index
    on anything but a `Uint8Array` — the out-of-bounds message's `ArrayClass.name`
    interpolation is only ever checked against one class name.

## What we test in addition

`crates/mnemonist-core/src/structures/vector.rs` — 18 unit tests beyond
`reproduces_the_upstream_suite`, the 1:1 port of all 18 upstream blocks:

| Test | Closes gap |
|---|---|
| `get_and_set_admit_index_equal_to_length` | 1 — B-101 |
| `a_full_vector_drops_the_admitted_write` | 1 — the companion case where `index == length == capacity`, so there is no capacity-region slot to admit into |
| `stale_data_from_a_pop_survives_a_growth_and_stays_reachable` | 2 — B-102 |
| `a_policy_returning_infinity_is_refused_before_any_allocation` | 5 |
| `a_policy_returning_nan_is_refused` | 5 |
| `a_policy_returning_a_negative_number_is_invalid_before_being_non_finite` | 5 — and which of the two upstream throws each non-finite value lands in |
| `a_policy_returning_not_a_number_is_invalid` | 6 |
| `shrinking_a_pointer_vector_keeps_its_current_width` | 7 |
| `a_zero_capacity_pointer_vector_starts_at_the_narrowest_width` | 8 |
| `a_pointer_vector_too_large_to_index_is_refused` | — a guard with no upstream-reachable input, kept because `Vector::pointer` needs an `Err` path to return |
| `float64_values_are_stored_exactly` | 3, 4 |
| `fixed_values_truncate_at_their_own_width` | 3 |
| `cursors_do_not_restart_but_the_vector_can_be_walked_again` | 9 |
| `a_pop_during_iteration_stays_bounded_by_the_frozen_length` | 10 |
| `an_empty_vector_pops_and_iterates_to_nothing` | 11 |
| `fills_to_capacity_without_running_off_the_end` | — a port-side invariant: growth never over-allocates |
| `the_out_of_bounds_message_names_the_actual_backing_class` | 12 |

**Differential fuzzing (see Fuzz below)** covers the same ground the native tests do, from the
opposite direction: instead of a handful of hand-picked boundary cases, every generated program
routinely lands on `index == length`, pushes values past truncation width, and pops immediately
before a grow — at ~1.45M operations, zero divergences, which is the strongest evidence B-101 and
B-102 are reproduced exactly rather than approximately.

**Still untested, stated rather than glossed:** `Vector.PointerVector.from` (gap 12's first half;
no native test constructs a `PointerVector` from an iterable, though the bridge supports it).

## Bugs this found

**B-101 — `get`/`set` admit `index === length`, one past the last pushed element.**
`status: VERIFIED against Node 24.18.1`. Both guards are `<`, not `<=`:

```js
Vector.prototype.set = function(index, value) {
  if (this.length < index) throw new Error('...index out of bounds.');
  this.array[index] = value;
  return this;
};
```

```text
var v = new Vector(Uint8Array, 5);   // length 0, capacity 5
v.set(0, 42);                        // 0 < 0 is false: WRITES. length stays 0.
v.get(0) === 42
```

`set(length, v)` does not advance `length`, so the write is invisible to anything that only reads
`length`/`capacity` — it shows up only in `array` itself, or in a subsequent `get(length)`.

**B-102 — a popped slot's stale data survives a growth, and B-101 keeps it reachable.**
`status: VERIFIED against Node 24.18.1`. `pop()` reads and decrements; it never clears:

```js
Vector.prototype.pop = function() {
  if (this.length === 0) return;
  return this.array[--this.length];
};
```

Growth then copies the *whole* old backing array, capacity included:

```js
if (typed.isTypedArray(this.array))
  this.array.set(oldArray, 0);   // not `oldArray.subarray(0, this.length)`
```

```text
var v = new Vector(Uint8Array, 2);
v.push(9); v.push(8);   // array [9, 8], length 2
v.pop();                // length 1, array UNCHANGED: [9, 8]
v.reallocate(4);        // array [9, 8, 0, 0] -- the 8 survived the copy
v.get(1) === 8          // length(1) < index(1) is false: reads the stale 8
```

**Neither defect alone reaches this state.** Without B-101's admission, index 1 would be refused
after the pop (`length` is 1, so `get(1)` needs the guard to let `1 === length` through). Without
the whole-capacity copy, the grow would carry forward a zero instead of the stale `8`. `pop` is
called four times across the whole suite and never followed by a growth call; this compounding is
entirely unexercised upstream.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **Only four `ArrayClass` values are modelled: `Uint8Array`/`Uint16Array`/`Uint32Array`/`Float64Array`.** | `test/vector.js` never constructs a plain `Array`, a signed or clamped typed array, or a caller `factory`; the four modelled here are exactly what it, `Vector.PointerVector`, and `Vector.Float64Vector` reach. Modelling all fifteen JS "typed array or factory" combinations for a module the suite exercises through four is effort spent on a surface nobody is checking. |
| — | **`Vector.PointerVector` has no `ArrayClass` value to resolve.** | Upstream's `pointerArrayFactory` is a private function reachable only through `Vector.PointerVector = subClass(pointerArrayFactory)` — there is no global a caller could pass to the base constructor to reach the same behaviour. The bridge gives it its own hidden factory rather than pretending a resolvable `ArrayClass` exists. |
| — | **`get`/`set` admit `index === length`, matching B-101 exactly.** | Tightening the guard to `<=` would be *more correct* than upstream and is exactly the failure mode this port must avoid — see the falsification below, which confirms the fuzz campaign would catch that "fix". |
| — | **Growth bulk-copies the whole old capacity, matching B-102.** | Copying only up to `length` is the "obvious" implementation and would silently zero the stale slot upstream leaves reachable — a real behaviour turned into a false one. |
| — | **A growth policy is a `Box<dyn Fn(f64) -> Option<f64>>` called from Rust.** | Same shape as `bit-vector`'s `Policy`, for the same reason: a JS policy can throw (parked in a `RefCell` at the bridge, preferred over the core's own classification) and can return "not a number" (`None`), which `applyPolicy`'s `typeof !== 'number'` check explicitly guards against. |
| — | **A policy returning non-finite (`Infinity`/`NaN`) is refused with `Error::PolicyNotRepresentable`, not reproduced as an allocation attempt.** | Upstream's guard does not catch either value, and both flow into `new ArrayClass(Infinity)` / `(NaN)`, throwing `RangeError: Invalid typed array length: …` from inside the engine. There is no honest Rust expression of "allocate `Infinity` elements"; refusing earlier, catchably, is the same call `bit-vector`'s identical divergence makes. |
| — | **`Vector.from`'s pushed values are coerced with `ToNumber` at the bridge, not checked to already be numbers.** | `push`/`set` themselves take a typed `f64` napi parameter, narrower than upstream's implicit typed-array coercion — the same simplification `hashed-array-tree`'s bridge makes. |
| — | **The backing store's `array` field is exposed to napi callers as a copy.** | `test/vector.js` never reads `vector.array` directly (unlike `bit-set`'s suite), so this is not load-bearing for gate 4, but the differential fuzzer compares the real backing store slot for slot after every operation, so the representation is verified even though the original suite never asked to see it. |

## Fuzz + bench

### Fuzz

```
module=vector seed=42       cases=7337 ops=735742 wall=60.0s divergences=0
module=vector seed=20260801 cases=7209 ops=718336 wall=60.0s divergences=0
```

Two campaigns, two seeds, **1,454,078 operations, zero divergences**. Reproduce with
`target/release/difffuzz --module vector --seed 42 --cases 7337`.

* **Constructor:** one of `Uint8Array`/`Uint16Array`/`Uint32Array`/`Float64Array`, with
  `initialCapacity`/`initialLength` each in `0..48` — independently, so `initialLength >
  initialCapacity` (upstream's `capacity = Math.max(initialLength, initialCapacity)`) is routine.
* **Op alphabet:** `push(v)` (weight 5) · `pop()` (3) · `set(i, v)` (3) · `get(i)` (3) ·
  `grow(c)`/`grow()` (1 each) · `resize(l)` (2) · `reallocate(c)` (1).
* **Indices:** `0..64`, well past any generated length or capacity, so both the `index == length`
  admission and the ordinary out-of-bounds throw are common outcomes, not edge cases.
* **Values:** `0.0..70000.0` — past `255` so a `Uint8Array`/`Uint16Array` truncating store is
  exercised, and full-precision `f64`s so `Float64Array` stores are compared exactly.
* **Observable state, compared after every op:** `length`, `capacity`, and **`array`** — the whole
  backing store, capacity region included, encoded exactly as the oracle encodes a JS typed array.
  `array` is the point: without it, B-101 and B-102 are only checkable indirectly through `get`.

**A harness bug this campaign's own design surfaced, fixed before trusting any result from it**
(D-103): the oracle's response line is a full-precision JSON number for every non-truncating
`Float64Array` value this grammar generates. `serde_json`'s default float parser is not always
correctly rounded for such values — a scratch test parsing the literal `"38403.356486892444"`
recovered a value one ULP away from what Rust's own `f64::from_str` gives for the same text. The
wire log showed the port and the oracle's raw response text agreeing exactly; only the *parsed*
`Value` used for the comparison disagreed. Enabling `serde_json`'s `float_roundtrip` feature
(workspace `Cargo.toml`) fixed it — the same class of finding as D-78, a harness defect that
manufactures divergences rather than catching real ones. `vector` is the first module whose
grammar generates `f64` values wide enough to land in the affected range.

### Falsification of the port (gate 6)

**Named first:** `vector_matches_upstream` (`crates/difffuzz/tests/differential.rs`) should go red,
because the fuzz grammar's `set`/`get` indices routinely land on `index == length`, and the
sabotage removes exactly the admission that lets that case succeed.

**The sabotage:** `Vector::set`'s bound check tightened from `if self.length < index` to
`if self.length <= index` — "fixing" the off-by-one that B-101 documents, the single most obvious
thing a future cleanup would do to this file.

**Confirmed red:** `cargo test -p difffuzz --test differential vector_matches_upstream` failed
immediately, on the campaign's very first shrunk case:

```
divergence in return value after op #1: set(22, 11908.642978421198)
  $throw:
    port:     "Vector(Float64Array).set: index out of bounds."
    upstream: <absent>
  $self:
    port:     <absent>
    upstream: true
minimal repro:
var s = new Vector(Float64Array, {"initialCapacity":12,"initialLength":17});
s.resize(22);
s.set(22, 11908.642978421198);
```

Reverted; **confirmed green again**: `vector_matches_upstream ... ok`.

### Bench

`bench/results.json` → `modules["vector"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `push`/`get`/`pop` (50/25/25) growing from `(capacity 0, length 0)`,
xorshift32 seed 42. `get` always lands on a uniformly random *existing* index (`workload.a[i] %
current length`), so the upstream `index == length` boundary — an unguarded, presumably-a-bug read
one past the end, which belongs to the differential fuzzer and not this benchmark — is never
exercised here.

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **7.35** | 9.56 | 1.3× faster |
| p99 ns/op | **27.9** | 60.5 | 2.2× faster |
| RSS delta MB | **11.8** | 37.4 | |
| structure-only RSS delta MB | **1.3** | 9.7 | |
| startup ms | **0.6** | 16.9 | 28× (reported separately; not throughput) |

**A clean win, and the smallest per-op margin of the five modules added in this pass** — expected,
since `vector` was picked specifically as the throughput floor: a growable array with the least
per-op work of anything benchmarked here, so there is the least room for either side's overhead to
show. The p99 gap (2.2×) is wider than the p50 gap (1.3×), consistent with the growth-policy reallocs
this workload includes landing inside V8's GC accounting on some batches and not on the port's,
which never triggers a collector.

**Falsification of the harness itself, run against this module.** A ~5,000-iteration `black_box`
spin was inserted into every `push` call's timed path (50% of ops), rebuilt, and re-measured:
`p50_ns_per_op` moved from 6.9 to 486.8 (55×), `p99_ns_per_op` from 22.7 to 719.8 (12×), and
`regressions` went from empty to three entries — while the untouched Node side stayed at its normal
~8.8 ns/op. Reverted and re-measured: the checksum (`249930270812`) was identical before sabotage,
during, and after revert, and the figures returned to the table above within run-to-run noise. The
harness can detect a regression it did not have before, which is the property gate 6 asks a
falsification to demonstrate.

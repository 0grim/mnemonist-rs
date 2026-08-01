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

**Rust native tests** — `crates/mnemonist-core/src/structures/multi_array.rs` (11):

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_set_walkthrough`, `reproduces_the_upstream_push_walkthrough`, `get_returns_none_past_dimension_and_the_bucket_otherwise`, `has_and_multiplicity_agree_with_upstream` | the upstream blocks, as a baseline, both container kinds |
| `inserting_out_of_order_leaves_a_real_gap_at_dimension` | the untested gap-read case above |
| `containers_and_associations_walk_dimension_in_gets_order`, `values_are_global_insertion_order_or_reversed_per_bucket`, `entries_walk_each_bucket_tail_to_head_in_dimension_order`, `keys_is_the_dimension_range` | the five iterator factories directly against literal expected sequences, including the forward-vs-reverse order contrast between `get` and `values(index)` (see the core module's docs — the sharpest place a transcription error would hide) |
| `fixed_capacity_values_narrow_to_their_width` | the untested overflow-truncation case above |
| `an_empty_multi_array_has_no_containers_or_values` | the zero-state baseline |

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
`planning/NOTES.md`'s "multi-array, symspell, passjoin-index" entry for the full account, and
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

```
module=multi-array  seed=42        cases=8583 ops=834584 wall=60.0s divergences=0
module=multi-array  seed=20260802  cases=8567 ops=836499 wall=60.0s divergences=0
```

Two campaigns, two seeds, **1.67M operations, zero divergences** — after the bridge fix above;
the campaigns logged are the clean, post-fix runs. Reproduce with e.g.
`target/release/difffuzz --module multi-array --seed 42 --cases 8583`.

* **Op alphabet:** `set` (weight 5) and `push` (weight 4) dominate, since they are the only ops
  that grow a bucket or exercise the capacity throw; `get`/`has`/`multiplicity` (weights 3/2/2)
  round it out. `containers`/`associations`/`values`/`entries`/`keys` are deliberately **not** in
  the alphabet — see "Deliberate divergences" and the spec's own module docs: all five now return a
  genuine opaque iterator on both sides, which `fuzz/oracle.js`'s `encode()` reduces to `{}`
  regardless of what is actually inside, so comparing them can only ever agree trivially.
* **Index pool:** ten indices, small enough that `set`/`push` collide on the same bucket constantly.
* **Constructor:** alternates between the default dynamic container (weight 3) and a fixed-capacity
  `Uint8Array`/`Uint16Array`/`Uint32Array` with a small capacity (weight 2, capacity `1..12`), so a
  `push`/`set` past capacity is common rather than rare.
* **Observable state:** `size`, `dimension`. `get`'s own return value (compared per-op, not just as
  state) renders a container exactly as `fuzz/oracle.js`'s `encode()` renders the real value: a
  plain array in dynamic mode, `{"$typed": ..., "values": [...]}` in fixed mode.

**Direct evidence the grammar reaches the states this campaign is for** — `grammar_self_check`
(`crates/difffuzz/src/modules/multi_array.rs`, no oracle, no `node`), 400 generated programs, up to
300 ops each:

```
multi-array grammar: 49449 steps with a multi-value bucket, 10149 capacity-exceeded throws
```

Both floors are asserted in the test itself (`> 100` and `> 20` respectively), so a future weighting
change that regresses this back toward "every bucket holds exactly one value" or "capacity is never
actually reached" fails loudly.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:** `test/multi-array.js`'s "should be
possible to get subarrays." — `assert.deepStrictEqual(array.get(0), [1, 2, 3]);` after
`set(0,1); set(0,2); set(0,3); set(1,4); set(1,5); set(2,6);`.

**The sabotage:** in `MultiArray::set`'s dynamic-mode branch, the pointer-chain link
`self.pointers.push(previous_tail)` was changed to `self.pointers.push(pointer)` — a
self-referencing pointer instead of a link to the item pushed just before it, breaking the
backward-walk linked list every `get`/`values`/`entries` depends on.

**Confirmed red:** the named assertion failed (`get(0)` came back `[3, 3, 3]`, not `[1, 2, 3]`) —
six of eleven native tests failed, and the real upstream mocha suite failed the same assertion at
`test/multi-array.js:216`, with the identical wrong values.

**Reverted; confirmed green again:** 11/11 native tests, 12/12 upstream `it` blocks.

**Nothing was found to be blind.** The sabotage broke exactly the mechanism it targeted (every
bucket read collapsed to repeating its most-recently-pushed value) and nothing else was needed to
catch it, in either instrument.

### Bench

**Not run.** Gate 10 is deferred to a serial pass on an idle machine (DESIGN.md §7.3). `multi-array`
is therefore complete except gate 10, and deliberately not in `tests/scope.txt`.

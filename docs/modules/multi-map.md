# multi-map

Upstream: `multi-map.js` (408 LOC) — depends on `vector.js` (373 LOC, already ported as its own unit)
· `test/multi-map.js` — **381 lines, 17 `it` blocks, 91 assertion statements**.

Port: `crates/mnemonist-core/src/structures/multi_map.rs` — `MultiMap<K, V>`, a `Map` from key to
*bucket* (an `Array`-like or `Set`-like container, decided once at construction). Bridge:
`crates/mnemonist-napi/src/multi_map.rs`. Shim: `tests/bridge/multi-map.js`. Fuzz spec:
`crates/difffuzz/src/modules/multi_map.rs`.

`test/multi-map.js` also `require`s `vector.js` for its one "should work with vectors" case, so
that dependency sits in this unit's require-closure — already ported and
already in `tests/bridge/vector.js`, so nothing extra was needed here beyond knowing it was there.

---

## What upstream tests

Every method, across both container kinds:

* **`set`/`has`/`multiplicity`** for the default (`Array`) and `Set` containers, including that a
  `Set`-kind `multiplicity` does not grow past 1 on a repeated identical value.
* **`remove`**, both container kinds, including the three-step drain (`['one', 1, 2] → [2, 1] → [2] →
  gone`) that walks a bucket down to empty and confirms the *key* disappears from the map, not just
  the bucket.
* **`delete`**, **`clear`**.
* **`get`**, both container kinds — `instanceof Set` is asserted for the `Set` case, and exact array
  equality for the `Array` case.
* **`forEach`** (flattened `(value, key)` pairs) and **`forEachAssociation`** (`(container, key)`
  pairs, one call per key).
* **Five iterator factories** — `keys()`, `values()`, `entries()`, `containers()`, `associations()` —
  each asserted twice: once via manual `.next()` calls and once via `[...map.X()]`.
* **`for...of`** (`Symbol.iterator`, aliased to `entries`).
* **`MultiMap.from(iterable)`**.
* **A non-`Set`, non-`Array` container** (`Vector.Uint8Vector`) — the one test that is not just
  "the same assertions against the other container kind".

## What upstream does NOT test

**Any container class other than exactly `Array` (the default) or exactly `Set`.** The `Vector`
case only ever asserts `Array.from(map.get('one'))` against the pushed numbers — never
`instanceof Vector`, never a `Vector`-specific method (`push`/`get`/`capacity`) called on the
returned container, never `remove` against a `Vector`-backed map (which upstream itself would throw
on: a real `Vector` has no `indexOf`/`splice`, and `remove`'s `Array` branch calls both). So the
*only* thing this test establishes about a non-`Set` container is that `.push` gets called on it and
the pushed values come back in order — precisely what this port's `List`-kind bucket already is. See
"Deliberate divergences" for what that licenses.

**A container mutated from inside a walk over it.** No `forEach`/`forEachAssociation` block, and no
iterator-lifecycle interleaving (mutate, then call `.next()` again), ever runs. `MultiMap`'s own
`values()`/`entries()`/`forEach` obtain, per upstream key, either a *live* `Set` iterator or an
`Array`-index walk with the length captured once — see the core module's docs for exactly what that
means and what this port's flattened cursor simplifies.

**`remove`'s size/dimension bookkeeping on a `Set`-kind bucket that is already empty.** Upstream's
`Set` branch checks `container.size === 0` *after* `container.delete(value)`, regardless of whether
anything was actually removed — but a live bucket is never empty by construction (an empty bucket's
key is always deleted from `items`), so this branch is dead code on every reachable input, upstream's
own included. Confirmed by reading, not by a test.

**`MultiMap.from`'s second argument** (`Container`) is asserted with `undefined` and with `Set`
elsewhere in the file's own default-container tests, but never in combination with `.from` itself —
`.from` is only ever called with the default container in `test/multi-map.js`.

## What we test in addition

**Rust native tests** — `crates/mnemonist-core/src/structures/multi_map.rs` (9):

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_walkthrough`, `remove_matches_upstream_size_and_deletion_bookkeeping`, `delete_removes_the_whole_bucket`, `clear_resets_size_and_dimension` | the upstream blocks, as a baseline |
| `set_kind_deduplicates_by_the_supplied_equality`, `list_kind_never_deduplicates` | the `Set`/`Array` write-path contrast directly, over a `V` with no `Hash`/`Eq` at all |
| `remove_on_a_set_kind_bucket_drops_the_key_once_it_empties` | the drain-to-zero path, `Set`-kind |
| `a_key_deleted_ahead_of_a_live_cursor_is_skipped` | the flattened cursor's outer liveness |
| `set_with_hands_a_rejected_duplicate_back_instead_of_dropping_it` | the resource-leak contract `fuzzy_multi_map`'s bridge depends on (see "Bugs this found") |
| `fallible_equality_short_circuits_on_the_first_error` | the fallible `set_with`/`remove_with` machinery, over a comparator that returns `Err` |

**Differential fuzzer** — a three-key pool shared by `set`/`remove`, weighted so buckets routinely
hold several values and drain back to zero; see "Fuzz + bench" for the measured numbers, including a
`grammar_self_check` that counts both states directly rather than inferring them from op weights.

**Still untested, stated rather than glossed:** a container mutated mid-walk (the flattened cursor's
one stated simplification — see the core module's docs), and any container beyond `Array`/`Set`
rendered through the bridge (which upstream's own suite does not distinguish from `Array` either, per
the gap above).

## Bugs this found

No upstream defect found in this unit. One resource-leak defect in this port's own bridge, found and
fixed before it reached `fuzzy-multi-map` (which depends on `MultiMap::set_with`) rather than by that
unit's own tests, is recorded there rather than duplicated here — see `docs/modules/fuzzy-multi-map.md`,
"Bugs this found", and `crates/mnemonist-core/src/structures/multi_map.rs`'s `set_with` docs.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-160 | **Any `Container` constructor other than exactly the global `Set` is treated identically to `Array`, and a bucket's rendered value is always a plain `Array` or a real `Set` — never a `Vector` or other custom class.** | Upstream's own write path only ever branches on `this.Container === Set`; everything else takes the identical `container.push(value); this.size++;` line. `test/multi-map.js`'s one non-`Array`/`Set` case (`Vector.Uint8Vector`) only ever asserts `Array.from(map.get(key))`, never `instanceof Vector` or a `Vector`-specific method on the returned container — see "What upstream does NOT test". A caller that does check `instanceof Vector`, or that relies on a custom container's behaviour beyond `.push`, would see a plain array instead; nothing in the original suite can tell the difference. |
| D-161 | **`Set`-kind membership is a linear scan against a supplied equality, not a hash lookup.** | `V` carries no `Hash`/`Eq` bound at all — see `MultiMap`'s module docs for why: `fuzzy-multi-map`'s own values can be arbitrary JS objects, whose `Set` membership is SameValueZero-by-identity and needs `napi_strict_equals`, which no compile-time bound can express. A performance cost relative to a real hash-based `Set`, not a behavioural one — buckets in every observed test and fuzz case are small. |
| D-162 | **The flattened `values()`/`entries()`/`forEach` cursor snapshots a bucket's contents once, when the outer walk reaches that key, rather than re-reading it live.** | Upstream's own walk holds a live `Set` iterator or a length frozen at entry, per key; this port cannot fully reproduce a mutation to the *same* bucket mid-walk over it, though it does correctly reproduce the outer map's own liveness (a key deleted ahead of the cursor is skipped). Untested by `test/multi-map.js`, which never mutates from inside a walk. See `MultiMap`'s module docs for the exact case this does not cover. |

## Fuzz + bench

### Fuzz

```
module=multi-map  seed=42       cases=8404 ops=838808 wall=90.0s divergences=0
module=multi-map  seed=20260801 cases=6221 ops=618991 wall=60.0s divergences=0
```

Two campaigns, two seeds, **1.46M operations, zero divergences**. Reproduce with e.g.
`target/release/difffuzz --module multi-map --seed 42 --cases 8404`.

* **Op alphabet:** `set` (weight 5, the only op that grows a bucket), `remove` (3), `delete`/`has`/
  `get`/`multiplicity` (2 each), `clear` (1). Cursor-lifecycle ops (`$iter`/`$next`/`$spread`) are
  deliberately not in this alphabet — see the spec's own module docs for why, and what covers cursor
  behaviour instead.
* **Key pool:** three keys (`"a"`, `"b"`, `"c"`), small enough that `set`/`remove` collide
  constantly rather than spreading across a wide, mostly-empty map.
* **Value pool:** four values, two strings and two numbers, wide enough that a `Set`-kind bucket
  sees genuine duplicates and genuine distinct members both.
* **Constructor:** alternates between the default container and `{"$global": "Set"}`, so both
  bucket kinds get their own campaign share.
* **Observable state:** `size`, `dimension`, and `items` rendered exactly as `fuzz/oracle.js`'s
  `encode()` renders the real per-key `Array`/`Set`.

**Direct evidence the grammar reaches the states this campaign is for** — `grammar_self_check`
(`crates/difffuzz/src/modules/multi_map.rs`, no oracle, no `node`), 400 generated programs, up to
300 ops each:

```
multi-map grammar: 25,761 steps with a multi-value bucket, 4,157 keys drained to zero and removed
```

Both floors are asserted in the test itself (`> 100` for each), so a future weighting change that
regresses this back toward "a map that only ever stores one value per key" fails loudly.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:** `test/multi-map.js:49` —
`assert.strictEqual(map.multiplicity('one'), 1);` — the `Set`-kind dedup check in "should be
possible to get the multiplicity of a key in the map", which only holds if two identical
`map.set('one', 'hello')` calls into a `Set`-kind bucket are actually deduplicated.

**The sabotage:** `MultiMap::set_with`'s `Set`-kind branch had its "was this value already present"
check discarded (`let _ = present;`) so every `set` unconditionally pushed and incremented `size`,
regardless of the equality check just performed.

**Confirmed red:** the named assertion failed (`multiplicity('one')` came back `2`, not `1`), and
four further assertions downstream of it in the same run also went red (the iterator/entries/values
blocks, which read the now-corrupted bucket) — `12 passing, 5 failing`, down from `17 passing`.

**Reverted; confirmed green again:** `17 passing`, the original count.

**Nothing was found to be blind.** The sabotage broke exactly the mechanism it targeted and nothing
else was needed to catch it.

### Bench

`bench/results.json` → `modules["multi-map"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`get`/`remove` (50/25/25), `ContainerKind::List` (upstream's
default `Array` container), over a 20,000-key domain deliberately far smaller than the op count —
the load-bearing multi-container parameter is how many VALUES sit under one key, and a workload
where every key holds exactly one value would benchmark a map with extra indirection, not a
multi-container. **~25 values per key on average by the run's end** (500,000 `set` calls over
20,000 keys), xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **25.9** | 36.4 | 1.4× faster |
| p99 ns/op | **46.1** | 89.8 | 1.9× faster |
| RSS delta MB | **11.6** | 79.3 | |
| structure-only RSS delta MB | **0.1** | 5.8 | |
| startup ms | **0.6** | 16.8 | 28× (reported separately; not throughput) |

**No regressions.** `remove`'s linear scan (of whichever bucket its key hits) is genuinely exercised
at the ~25-value-per-key depth this workload reaches, and upstream pays the identical `Array
.indexOf` linear scan — unlike `bit-set.rs`'s `rank` trap, this is an op whose cost scales with a
workload parameter *on both sides*, not a port-only pathology, and it was checked before committing
to the 25%-weighted mix (see `bench/runner/src/multi_map.rs`'s own module docs for the arithmetic).

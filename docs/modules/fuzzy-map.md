# fuzzy-map

Upstream: `fuzzy-map.js` (185 LOC) · `test/fuzzy-map.js` — **161 lines, 10 `it` blocks, ~20 assertion
statements**.

Port: `crates/mnemonist-core/src/structures/fuzzy_map.rs`. Bridge:
`crates/mnemonist-napi/src/fuzzy_map.rs`. Shim: `tests/bridge/fuzzy-map.js`. Fuzz spec:
`crates/difffuzz/src/modules/fuzzy_map.rs`.

A `FuzzyMap` is a `Map` whose keys are computed by a hash function before every read or write, so
several distinct queries can resolve to the same stored item — "a map with lowercased keys" is
upstream's own example. It is `default-map`'s T3 shape minus the one thing `default-map` has that
this does not (a factory that manufactures a *missing* value): a miss here is just `undefined`,
exactly like a plain `Map`.

---

## What upstream tests

Ten `it` blocks:

* **Constructor validation**: three throwing cases (`{hello: 'world'}`, `[{...}]`, `[null,
  {...}]`), all matching `/hash/` — a non-function write hash, in scalar and array-descriptor form.
* **`add`**, hashing the item itself (`item.title.toLowerCase()`), and **`set`**, hashing a
  caller-supplied key — one `it` block each, checked only through `size`.
* **`clear`**, checked only through `size` afterwards.
* **`get`/`has`**, each with three case variants of the same stored key (`'HELLO'`, `'Hello'`,
  `'hello'`) resolving to one item, plus one miss.
* **Iteration**: `forEach`, `values()`, `for...of` — each with two items, checked in insertion
  order. All three read the *value* only; none ever receives a key.
* **`FuzzyMap.from`**, twice: once with the `[writeHash, readHash]` array-descriptor form over a
  plain array, once with a single `readHash` and `useSet: true` over a `Map`.

## What upstream does NOT test

**The distinction `forEach`'s own signature reveals.** Upstream's inner delegation is
`this.items.forEach(function(value) { callback.call(scope, value, value); })` — one parameter, and
the *value* passed as both callback arguments. No `it` block observes what the second argument
actually is; every test's callback only reads its first parameter. A port that supplied `(value,
key)` — the hashed key, which the map genuinely knows — would look identical to every assertion in
the file, and would still be wrong.

**Never called at all:**

1. **A stored `undefined`.** No test calls `add`/`set` with `undefined` as the item, so `get`'s and
   `has`'s disagreement on that case (mirroring `default-map`'s) is untested.
2. **Overwrite.** No test `set`s or `add`s the same resolved key twice; whether the second write
   moves the entry or updates in place is unobserved.
3. **`arguments.length`-sensitive `forEach` scope.** Every call in the file passes no `scope`
   argument; the `arguments.length > 1 ? scope : this` branch that binds `this` to the map itself
   is exercised, but the explicit-scope branch never is.
4. **`values()` / iteration on an empty map**, or after `clear()`.
5. **A single-function descriptor's write/read split**, i.e. that `set('KEY', ...)` and
   `get('key')` really do go through the *same* function when only one is given. Both `get`/`has`
   tests happen to use one function for both directions, so this is implicit rather than checked.
6. **`inspect()`** and its `nodejs.util.inspect.custom` symbol.

## What we test in addition

`crates/mnemonist-core/src/structures/fuzzy_map.rs` — 8 tests (core takes the *already-hashed* key,
since hashing is a JS callback and lives in the bridge — see the module docs for the split):

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of the assertions that do not need real hash-function plumbing |
| `has_and_get_disagree_on_a_stored_undefined` | 1 |
| `overwriting_a_stored_undefined_reports_no_displaced_value` | 1 — the `Option<V>::flatten` interaction |
| `set_overwrites_in_place_and_reports_the_displaced_value` | 2 — same resolved key, insertion order unchanged |
| `values_mut_reaches_every_stored_slot_including_the_undefined_ones` | 1 — the bridge's `clear` needs this to release every napi reference |
| `a_cursor_sees_entries_set_after_it_was_opened` | 4 — live-cursor semantics, inherited from `OrderedMap` |
| `there_is_no_delete_only_clear` | — `fuzzy-map.js` defines no `delete` at all; stated as a real absence, not an oversight |
| `an_empty_map_reports_nothing` | 4 |

Gaps 3, 5 and 6 are stated rather than closed — 3 and 5 are bridge-level (the hash-function split is
JavaScript, not core), covered instead by the differential campaign below and by
`mnemonist_napi::fuzzy_map`'s own construction tests; 6 is the same disclosed absence as every other
unit in this wave.

## Bugs this found

**None in upstream.** `fuzzy-map.js` is a thin `Map` wrapper with no size drift, no reachable
off-by-one, and no branch the port's own harness bug (below) turned out to be hiding.

**A harness bug, not an upstream one — recorded here for the paper trail.** The difffuzz spec's
`Hash::named` matched the literal names `"identity"`/`"lower"`, but `fuzz/oracle.js`'s `FACTORIES`
table and this spec's own constructor strategy both use the prefixed `fuzzyIdentity`/`fuzzyLower`
(chosen precisely so this module's factory names cannot collide with `default-map`'s, which the
oracle also serves). Every generated program panicked at construction, before a single comparison
ran — which is why this spec had never actually executed: it was not yet wired into
`tests/differential.rs`, and the one earlier manual campaign attempt persisted a regression seed
that, on inspection, was the harness panic rather than a finding. That spurious seed was deleted
rather than kept. Fixed by matching the prefixed names; see `planning/NOTES.md` for the note and
`crates/difffuzz/src/modules/fuzzy_map.rs`'s history.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-312 | **Core stores `Option<V>`, not `V`.** | `this.items.get(key)` is `undefined` for both "no such key" and "the key holds `undefined`", exactly as `default-map`. `None` spells the latter; `has`/`get` diverge on it the same way upstream's do. |
| D-313 | **Hashing lives entirely in the bridge.** | The hash function(s) are JS callbacks; core takes the already-hashed key, the same split `default-map`'s factory makes. `crates/mnemonist-napi/src/fuzzy_map.rs`'s `HashFn` is `FunctionRef<Unknown<'static>, Unknown<'static>>` rather than a typed signature, because `add`'s hash argument is genuinely unconstrained (upstream's own test hashes a bare object). |
| D-314 | **A falsy descriptor slot becomes `None`, not a stored `identity` closure.** | `if (!this.writeHashFunction) this.writeHashFunction = identity;` is a truthiness test (`0`, `''`, `false`, `null` all fall through), not a null check. `resolve_hash` mirrors the truthiness test; `None` means "classify the value directly," which is observably identical to calling a real `identity` and feeding its return into `JsKey::from_unknown`, without paying for a `FunctionRef` and a JS round trip for what is a no-op. |
| D-315 | **`forEach`'s second callback argument is the value, not a hashed key.** | Reproduces the exact one-parameter delegation shown above; both core's `values_mut`/cursor step and the bridge's `for_each` project the *value* out twice. Not tested upstream (gap 1 above), but changing it would be wrong regardless. |
| D-316 | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion. |
| D-317 | **The `[write, read]` array-descriptor form is excluded from the fuzz grammar.** | It needs two independent named factories per case; the single-function form is what the campaign spends its budget on, and the pair form is covered instead by `FuzzyMap.from`'s own upstream test and by `mnemonist_napi::fuzzy_map`'s construction tests. Disclosed rather than silently narrowed. |
| D-318 | **Fuzzed items are always strings, never objects.** | A hash function that can throw (`item.title.toLowerCase()` on a bare string) would turn every non-title-bearing generated item into an apparatus failure rather than a comparison; `identity`/`lower` both accept a bare string, keeping every generated program well-defined on both sides. |

## Fuzz + bench

### Fuzz

```
module=fuzzy-map seed=42  cases=12019  ops=1210496  wall=90.0s  divergences=0
```

**1.21M operations, zero divergences.** Reproduce with `target/release/difffuzz --module fuzzy-map
--seed 42 --cases 12019`.

* **Op alphabet:** `add(item)` (weight 4) · `set(key, item)` (4) · `get(key)` (3) · `has(key)` (2) ·
  `clear()` (1).
* **Items are drawn from a small, mixed-case string pool** (`Hello`/`hello`/`World`/`WORLD`/`Foo`/
  `bar`), so `identity` and a case-insensitive hash disagree on collisions constantly — `"Hello"`
  and `"hello"` are one key under one hash function and two under the other.
* **Constructed with either `fuzzyIdentity` or `fuzzyLower`**, the two named factories
  `fuzz/oracle.js` gained for this module (prefixed so they cannot collide with `default-map`'s
  table entries). `add` and `set` hash *different* arguments — the item itself versus the caller's
  key — which is exactly the distinction the bridge's `HashFn` split has to get right; this grammar
  puts both call shapes in the alphabet so both are checked.
* **Observable state:** `size` and `items` — `{$map: [[key, value_or_undefined], ...]}`.
* **Deliberately excluded:** the array-descriptor form and object items (see Deliberate
  divergences); both are covered elsewhere and disclosed rather than silently narrowed.

**The harness bug above was found by running this campaign, before it was fixed.** Every case
panicked mid-construction — a hard failure, not a soft "zero ops" report, because the panic happens
inside `apply()`, past `report.ops > 0`'s guard. Confirmed to compile, run, and produce zero
divergences only after `Hash::named` was corrected to the prefixed names.

### Falsification of the port (gate 6)

**Named first:** `has_and_get_disagree_on_a_stored_undefined`'s assertion `assert!(map.has(&"a"))`.

**The sabotage:** `FuzzyMap::has` changed from `self.items.contains_key(key)` (a pure key test, as
upstream's `this.items.has(key)` is) to `self.items.get(key).is_some_and(Option::is_some)` — testing
the *stored value* instead, which is wrong for a key stored with `undefined`.

**Confirmed red**, at exactly the named assertion: `assertion failed: map.has(&"a")`. Reverted;
**confirmed green again**: all 8 `fuzzy_map` unit tests pass, `cargo test --workspace` clean.

### Bench

**Not run.** Gate 10 needs an idle machine and is batched into a separate quiet pass (§7.3); this
unit is deliberately not in `tests/scope.txt` until then. Gates 1–9 are green.

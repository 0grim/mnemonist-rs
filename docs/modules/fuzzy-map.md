# fuzzy-map

Upstream: `fuzzy-map.js` (185 LOC) · `test/fuzzy-map.js` — **161 lines, 10 `it` blocks, ~20 assertion
statements**.

Port: `crates/mnemonist-core/src/structures/fuzzy_map.rs`. Bridge:
`crates/mnemonist-napi/src/fuzzy_map.rs`. Shim: `tests/bridge/fuzzy-map.js`. Fuzz spec:
`crates/difffuzz/src/modules/fuzzy_map.rs`.

A `FuzzyMap` is a `Map` whose keys are computed by a hash function before every read or write, so
several distinct queries can resolve to the same stored item — "a map with lowercased keys" is
upstream's own example. It is `default-map`'s `Map`-backed shape minus the one thing `default-map` has that
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
since hashing is a JS callback and lives in the bridge — see the module docs for the split): a
baseline reproduction of the assertions that do not need real hash-function plumbing, `has`/`get`
disagreeing on a stored `undefined`, overwriting a stored `undefined` reporting no displaced value,
`set` overwriting in place and reporting the displaced value at the same resolved key without
moving insertion order, every stored slot reachable for bridge release, a cursor seeing entries set
after it was opened, that there is no `delete` at all (stated as a real absence, not an oversight),
and an empty map reporting nothing.

Gaps 3, 5 and 6 are stated rather than closed — 3 and 5 are bridge-level (the hash-function split is
JavaScript, not core), covered instead by the differential campaign below and by
`mnemonist_napi::fuzzy_map`'s own construction tests; 6 is the same disclosed absence as every other
unit in this port — `.inspect()` is a repo-wide scope cut, listed in `docs/DIVERGENCES.md`.

## Bugs this found

**None in upstream.** `fuzzy-map.js` is a thin `Map` wrapper with no size drift, no reachable
off-by-one, and no branch the port's own harness bug (below) turned out to be hiding.

**A harness bug, not an upstream one — recorded here for the paper trail.** The difffuzz spec's
name-matching for its hash factories did not match the actual prefixed factory names
(`fuzzyIdentity`/`fuzzyLower`, chosen precisely so this module's factory names cannot collide with
`default-map`'s, which the oracle also serves), so every generated program panicked at
construction, before a single comparison ran. This spec had in fact never actually executed before
being fixed. Full account: log.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| DIV-FUZZY-MAP-1 | **Core stores `Option<V>`, not `V`.** | `this.items.get(key)` is `undefined` for both "no such key" and "the key holds `undefined`", exactly as `default-map`. `None` spells the latter; `has`/`get` diverge on it the same way upstream's do. |
| DIV-FUZZY-MAP-2 | **Hashing lives entirely in the bridge.** | The hash function(s) are JS callbacks; core takes the already-hashed key, the same split `default-map`'s factory makes. `crates/mnemonist-napi/src/fuzzy_map.rs`'s `HashFn` is `FunctionRef<Unknown<'static>, Unknown<'static>>` rather than a typed signature, because `add`'s hash argument is genuinely unconstrained (upstream's own test hashes a bare object). |
| DIV-FUZZY-MAP-3 | **A falsy descriptor slot becomes `None`, not a stored `identity` closure.** | `if (!this.writeHashFunction) this.writeHashFunction = identity;` is a truthiness test (`0`, `''`, `false`, `null` all fall through), not a null check. `resolve_hash` mirrors the truthiness test; `None` means "classify the value directly," which is observably identical to calling a real `identity` and feeding its return into `JsKey::from_unknown`, without paying for a `FunctionRef` and a JS round trip for what is a no-op. |
| DIV-FUZZY-MAP-4 | **`forEach`'s second callback argument is the value, not a hashed key.** | Reproduces the exact one-parameter delegation shown above; both core's `values_mut`/cursor step and the bridge's `for_each` project the *value* out twice. Not tested upstream (gap 1 above), but changing it would be wrong regardless. |
| DIV-FUZZY-MAP-5 | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion. |
| DIV-FUZZY-MAP-6 | **The `[write, read]` array-descriptor form is excluded from the fuzz grammar.** | It needs two independent named factories per case; the single-function form is what the campaign spends its budget on, and the pair form is covered instead by `FuzzyMap.from`'s own upstream test and by `mnemonist_napi::fuzzy_map`'s construction tests. Disclosed rather than silently narrowed. |
| DIV-FUZZY-MAP-7 | **Fuzzed items are always strings, never objects.** | A hash function that can throw (`item.title.toLowerCase()` on a bare string) would turn every non-title-bearing generated item into an apparatus failure rather than a comparison; `identity`/`lower` both accept a bare string, keeping every generated program well-defined on both sides. |

## Fuzz + bench

### Fuzz

**1.21M operations, zero divergences:**

```
module=fuzzy-map seed=42  cases=12019  ops=1210496  wall=90.0s  divergences=0
```

Reproduce with `target/release/difffuzz --module fuzzy-map --seed 42 --cases 12019`.

The op alphabet covers `add`/`set`/`get`/`has`/`clear`. Items are drawn from a small, mixed-case
string pool, so `identity` and a case-insensitive hash disagree on collisions constantly —
`"Hello"` and `"hello"` are one key under one hash function and two under the other. Constructed
with either `fuzzyIdentity` or `fuzzyLower`, the two named factories the oracle gained for this
module (prefixed so they cannot collide with `default-map`'s table entries). `add` and `set` hash
*different* arguments — the item itself versus the caller's key — which is exactly the distinction
the bridge's `HashFn` split has to get right; this grammar puts both call shapes in the alphabet so
both are checked. Observable state is `size` and `items`. Deliberately excluded: the
array-descriptor form and object items (see Deliberate divergences); both are covered elsewhere and
disclosed rather than silently narrowed.

The harness bug above was found by running this campaign, before it was fixed — every case panicked
mid-construction, a hard failure rather than a soft "zero ops" report. Confirmed to compile, run,
and produce zero divergences only after the fix; full history: log.

### Falsification of the port (gate 6)

**Named first:** `has_and_get_disagree_on_a_stored_undefined`'s assertion `assert!(map.has(&"a"))`.

**The sabotage:** `FuzzyMap::has` changed from `self.items.contains_key(key)` (a pure key test, as
upstream's `this.items.has(key)` is) to `self.items.get(key).is_some_and(Option::is_some)` — testing
the *stored value* instead, which is wrong for a key stored with `undefined`.

**Confirmed red**, at exactly the named assertion: `assertion failed: map.has(&"a")`. Reverted;
**confirmed green again**: all 8 `fuzzy_map` unit tests pass, `cargo test --workspace` clean.

### Bench

`bench/results.json` → `modules["fuzzy-map"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`get`/`has` (50/25/25) over the full 1e6-key domain, keys hashed
by `hash(x) = x >> 4` on **both** sides (an arithmetic right shift needs no floating-point rounding
to keep in sync between a Rust closure and a JS function), collapsing 16 raw keys onto one stored
slot: the port is 1.8× faster at p50 (14.7 vs 26.0 ns/op), 1.3× faster at p99 (40.1 vs 50.9).

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | **14.7** | 26.0 | 1.8× faster |
| p99 ns/op | **40.1** | 50.9 | 1.3× faster |
| RSS delta MB | **12.8** | 33.6 | |
| structure-only RSS delta MB | **1.5** | 9.7 | |
| startup ms | **0.6** | 16.5 | 27× (reported separately; not throughput) |

**No regressions.** Faster on every latency metric despite the extra hash step both sides pay
identically — `FuzzyMap` here is `default-map`'s shape without the factory's mutating-read path
(`get`/`has` are plain lookups, no `get_or_insert_with`), which is a smaller surface than
`default-map`'s own workload and, unlike that unit, does not lose.

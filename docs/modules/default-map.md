# default-map

Upstream: `default-map.js` (162 LOC) · `test/default-map.js` — **111 lines, 7 `it` blocks,
21 assertion statements**.

Port: `crates/mnemonist-core/src/map/mod.rs` (the `Map` itself),
`crates/mnemonist-core/src/structures/default_map.rs`.
Bridge: `crates/mnemonist-napi/src/js_key.rs`, `crates/mnemonist-napi/src/js_value.rs`,
`crates/mnemonist-napi/src/map_cursor.rs`, `crates/mnemonist-napi/src/default_map.rs`.

This is the **pilot for bridge tier T3**. T3 is not a family of related structures; it is one
capability — *reproduce JavaScript's `Map`* — that eleven modules share, because `default-map`,
`bi-map`, `fuzzy-map`, `fuzzy-multi-map`, `multi-map`, `multi-set`, `lru-map` and
`lru-map-with-delete` all keep their state in a `new Map()`. `default-map` was chosen because it
is the *thinnest* wrapper over one: 162 lines, of which four are not delegation. The hard part is
therefore isolated in exactly the place the other ten modules inherit it from.

---

## What upstream tests

Seven `it` blocks, all on a map built with `FACTORY = function() { return []; }`:

```js
map.get('one').push(1);          // the central idiom of the file
map.set('two', [2]);
assert.deepStrictEqual(map.get('one'), [1]);
assert.strictEqual(map.size, 2);
assert.deepStrictEqual(map.get('unknown'), []);   // reading creates
assert.strictEqual(map.size, 3);
map.clear();
// …delete/has, forEach, entries/keys/values via obliterator's take(), autoIncrement, peek…
```

Characterising the shape of that coverage:

* **Every key is a string literal.** `'one'`, `'two'`, `'unknown'`, `'test'`, `'test2'`. Nothing
  else — no numbers, no objects, no `NaN`, no `-0`, no `null`, no `undefined`.
* **Every stored value is defined.** An array from the factory, a number from `autoIncrement`, or
  a literal. **`undefined` is never stored**, which turns out to be the whole ballgame — see B-40.
* **A key is never overwritten.** `set` is only ever called on a key that does not yet exist.
* **A key is never deleted and re-added.** The one `delete` block deletes `'one'` and stops.
* **No map ever holds more than three entries**, and the largest is built in three calls.
* **Iteration is drained immediately.** `take(map.entries())` creates a cursor and exhausts it in
  one expression, so nothing about the cursor's *state* is observable.
* **`map.get(k).push(v)` appears six times.** This is the one place upstream leans hard on
  something difficult: the value must come back **by reference**, so that a mutation through the
  returned handle is visible on the next read.

## What upstream does NOT test

Everything below is reachable through the public API and never exercised by the original suite.

**The `size` counter, which is upstream's one real defect**

1. **`undefined` is never stored as a value**, so the entire B-40 chain is unreachable: `get`
   testing the *value* rather than the *key*, the factory re-running on every read of such a key,
   and `size` drifting away from `items.size` without bound.
2. **`size` is never compared against `items.size`.** They are asserted to be equal only in cases
   where they cannot differ.
3. **`size` after an overwrite is never checked**, because no key is ever overwritten.

**`Map` semantics — the capability this unit exists to establish**

4. **Insertion order is only ever checked on an append-only map.** Order after a delete, after a
   re-insert, or after an overwrite is never asserted, so nothing pins the two rules that
   distinguish a `Map` from a `HashMap`: *delete-then-reinsert moves the key to the end, overwrite
   does not*.
5. **Iterator liveness is entirely untested.** All three rules — an entry appended behind a live
   cursor **is** visited, an entry deleted ahead of it is **skipped**, and a cursor that has once
   reported `{done: true}` stays detached even if the map grows — have zero coverage.
6. **`clear()` under a live iterator is never done.** Nor is the sequel: `clear()` then `set()`
   then `next()`, which yields the *new* entry.
7. **A cursor is never re-drained**, so the non-restartability of D-06 is unobserved.
8. **`[...map]` is never used**, so the collection-level `Symbol.iterator` — the *factory* half of
   D-07, the half napi does not provide for free, and the one upstream aliases to `entries` rather
   than to `values` — has **zero** upstream coverage despite being the last line of the module.
9. **No map is ever large enough to compact.** The port's tombstone reclamation, and the cursor
   relocation that has to survive it, are unreachable at three entries.

**Keys — the whole of SameValueZero**

10. **No non-string key is ever used.** `NaN` as a key (which `Map` treats as equal to itself,
    unlike `===`), `-0` and `+0` collapsing to one key, `0` versus `'0'` being two keys, and
    `null`/`undefined`/booleans as keys are all untested.
11. **An object key is never used** — which is also true of every other T3 test file; see
    "Deliberate divergences".

**Values**

12. **`null` as a value is never stored**, so nothing distinguishes it from `undefined`. They are
    very different: one is a value, the other is absence.
13. **The falsy sweep is never done.** `false`, `0`, `''` as values are untested here (though
    `lru-cache` does test them, which is why the port handles them).
14. **A value is never overwritten**, so nothing observes that the displaced value is released.

**The factory**

15. **The factory's arguments are never inspected.** Upstream calls `this.factory(key, this.size)`;
    the test's factories declare no parameters. So neither the key nor — more interestingly — the
    *drifted* size is checked.
16. **A throwing factory is never used**, so nothing pins that a failed `get` leaves the map
    completely untouched, `size` included.
17. **`autoIncrement`'s independence is never checked** — that two calls to
    `DefaultMap.autoIncrement()` produce two separate counters.

**`forEach`**

18. **The third callback argument is never inspected.** Upstream delegates to
    `this.items.forEach(...)`, so the native `Map` passes **itself** — the *inner* map, not the
    `DefaultMap`. The test's callback declares two parameters. This one is a real divergence in the
    port; see below.
19. **`scope` is never passed**, so the `arguments.length > 1 ? scope : this` branch is untested.
20. **`forEach` is never mutated during**, so its liveness is untested for the same reason (5) is.

**Never called at all**

21. `inspect()` and the `nodejs.util.inspect.custom` symbol. ~15 LOC of the module.

## What we test in addition

**`crates/mnemonist-core/src/map/mod.rs` — 21 tests** on the `Map` itself, deliberately against a
plain `OrderedMap<&str, u32>` rather than through `DefaultMap`, so the semantics are pinned once
for all eleven T3 modules. They cover insertion-order and overwrite behaviour (gap 4), all four
iterator-liveness rules (gap 5), `clear()` under a live cursor in both operation orders (gap 6),
and compaction under, ahead of, and between live cursors (gap 9). Full test-to-gap mapping:
`docs/modules/evidence/default-map.md`.

**`crates/mnemonist-core/src/structures/default_map.rs` — 15 tests** on `DefaultMap` itself,
starting from a 1:1 port of the seven upstream blocks as a baseline. These close the `size`-drift
chain end to end (gaps 1–3): the drift itself, value by value; that `set` and `delete` resynchronise
it and `get` does not; that a refilled `undefined` keeps its insertion position; that `has` and
`peek` disagree on a stored `undefined` where `delete` distinguishes it from a missing key. They
also close the factory's two arguments (gap 15), a throwing factory leaving the map untouched in
both the missing-key and stored-`undefined` cases (gap 16), and a displaced defined value reported
by `set` (gaps 3, 14). Full list: evidence file.

**`crates/mnemonist-napi/src/js_key.rs` — 8 tests** on SameValueZero, closing gap 10. Each asserts
**both** halves — equal *and* equally hashed — because a key type where `Eq` and `Hash` disagree
grows two entries for one key and every other test still passes. They pin `NaN` as one key,
`-0`/`+0` as one key stored as `+0`, ordinary numbers distinct with integral forms coinciding,
infinities as keys distinct from each other, and the primitive shapes (`0`, `'0'`, `false`, `null`,
`undefined`, `''`) as six distinct keys.

**27 side-by-side probes** against the real upstream module, run through the built addon and the
vendored `bench/upstream/default-map.js` in one process, comparing JSON-serialised results. All 27
agree. They cover, end to end through the bridge, what the Rust tests cover in core — value
identity across a round trip, the B-40 drift and its resynchronisation, delete-then-reinsert order,
overwrite position, `NaN`/`-0` as keys, mixed primitive keys, `null` versus `undefined` as values,
all three liveness rules, `clear`-then-`set` under a cursor, non-restartability next to collection
restartability, `forEach` liveness and both `scope` bindings, `autoIncrement` independence, the
factory's two arguments, a throwing factory, the falsy sweep, spreading the map, a 40-key churn, and
a walk across a compaction. Full list: evidence file.

The differential fuzzer covers gaps 1–9 and 12 continuously rather than at hand-picked points; see
"Fuzz + bench".

**Still untested, stated rather than glossed:** gap 21 (`inspect`, not bridged), gap 18 in its exact
form (a deliberate divergence, below), gap 19 in its `arguments.length` form (same), and gap 11
(object keys — a deliberate divergence, below).

## Bugs this found

**B-40 — `DefaultMap.get` tests the *value*, not the *key*, and then increments a counter instead
of reading one. `size` drifts without bound on a map holding one entry.** Verified against
Node 24.18.1.

```js
DefaultMap.prototype.get = function(key) {
  var value = this.items.get(key);
  if (typeof value === 'undefined') {          // (1) tests the VALUE
    value = this.factory(key, this.size);
    this.items.set(key, value);
    this.size++;                               // (2) not `this.items.size`
  }
  return value;
};
```

Line (1) asks whether the stored *value* is `undefined`, which is not the same question as whether
the key is present — they differ for exactly the keys whose value is `undefined`. Line (2) then
increments a counter, where `set` and `delete` both resynchronise from `this.items.size`. Measured:

```text
m.set('a', undefined);   size 1   items.size 1
m.get('a');              size 2   items.size 1   factory called again
m.get('a');              size 3   items.size 1   factory called again
m.delete('a');           size 0   items.size 0   resynchronised
```

Three consequences, all reproduced:

* **The factory re-runs on every read** of a key whose value is `undefined`, which for a stateful
  factory such as `DefaultMap.autoIncrement()` means the counter advances on a *read*.
* **`size` is unbounded** in the number of reads, not in the number of entries, and can be
  arbitrarily larger than `items.size`.
* **The drift is silent and self-healing.** Any `set` or `delete` snaps `size` back, so a program
  that interleaves reads and writes shows a `size` that is sometimes right and sometimes not, with
  nothing to indicate which.

Reproduced rather than corrected. `DefaultMap::size()` is a stored counter and `items().len()` is
the truth. **The correction is what a careful porter would write by accident**: making `size`
return the entry count is tidier, is what the name suggests, and passes the entire upstream
suite — confirmed by falsification; see "Fuzz + bench".

The fuzz oracle's own JSON encoding once conflated integral and fractional numbers — `serde_json`
distinguishes `1` from `1.0` where JSON and JavaScript do not — which would have produced false
divergence reports on every integral key. Fixed by `number_json`, which encodes a double the way
`JSON.stringify` does; the regression seed for this fault is committed. Full history in the log.

An early version of the core falsification test for compaction-under-a-live-cursor deleted entries
*ahead* of the cursor rather than *behind* it. That forces a compaction but leaves the cursor's
physical index accidentally correct, so the test passed even against a `locate` deliberately broken
to return its unvalidated hint — a falsification test that cannot fail is just a second green light,
the same lesson gate 6 itself carries, one level down. Rewritten to delete behind the cursor, where
compaction shifts every remaining entry left; this also surfaced a real out-of-range panic in hint
validation (`hint - 1` read before `hint <= slots.len()` was checked), now fixed. Full history in
the log.

**What the fuzzer found in the port: nothing.** Two campaigns, 4.37 M operations, zero divergences.
As with `sparse-set`, that is the expected outcome — a faithful port reproduces upstream's bugs, so
differential fuzzing structurally cannot find them. B-40 was found by reading the file line by line
and confirming each step against Node. What the fuzzer is for is the other direction, and here it is
sharper than the original suite by a wide margin — see "Fuzz + bench".

**The bridge held a bare core value behind `&self`**, which LLVM was entitled to compile as a
`noalias readonly` pointer and hoist reads across a re-entrant JS callback (B-31). It now holds
`RefCell<Core>`, which is not `Freeze`, and every `&mut self` method borrows via `borrow_mut()`
taken per step and released before the callback runs, so a re-entrant callback never meets an
outstanding borrow. See `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`. Full history in the
log.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **Object, symbol, function and bigint keys are rejected**, with an error naming the limit. | `Map` compares objects by identity, and there is no identity hash for a JS object reachable from Rust. Two designs are implementable and both cost something real: tagging each object with a hidden id under a private `Symbol` is O(1) but mutates the caller's object (visible to `Object.getOwnPropertySymbols`) and fails on a frozen one; an association list of `napi_ref` probed with `napi_strict_equals` touches nothing but is O(n) per operation and holds a strong reference to every key it has ever seen, making release a leak problem in its own right. **No upstream test in the entire T3 family uses an object key** — audited across `default-map`, `set`, `bi-map`, `fuzzy-map`, `fuzzy-multi-map`, `multi-map`, `multi-set`, `lru-cache` (all four variants) and `sparse-map`; every key that reaches a `Map` is a string or a number, and `fuzzy-map` hashes its object arguments to strings *before* the `Map` sees one. Building machinery no test can reach is worse than a stated limit; answering silently and wrongly is worse than both. |
| — | **Primitive values are stored by value; only objects are stored by reference.** | Forced twice over. `napi_create_reference` **rejects a number** at `NAPI_VERSION` 9, which this addon declares — measured, not assumed: it is what made two of the seven upstream assertions fail on this bridge's first run. And it is right independently: a `napi_ref` is a V8 global handle, and one per stored value would mean a million global handles for a million-entry `lru-cache` against upstream's inline SMIs. Nothing is observable: a JS primitive has no identity, so `0 === 0` and `'a' === 'a'` regardless of provenance. `-0` and `NaN` survive verbatim, because only *keys* are normalised. |
| — | **`forEach`'s third callback argument is the `DefaultMap`, not the inner `Map`.** | Upstream delegates to `this.items.forEach(...)`, and a native `Map` passes *itself* to the callback — so upstream's third argument is the internal map, an object this port does not have. The `DefaultMap` is passed instead. Untested upstream (gap 18); the first two arguments, which the original test does use, are exact. |
| — | **`forEach(cb, undefined)` binds `this` to the map.** | Upstream keys off `arguments.length > 1`, which napi's typed signature cannot see: "omitted" and "passed as `undefined`" are the same value. Identical to `SparseSet::for_each` and recorded the same way. The omitted-argument case — the only one the original suite uses — is exact, and passing a real scope object is exact. |
| — | **`inspect()` is not ported.** | It returns the inner `Map`, which does not exist in this port, and nothing asserts on it. |
| — | **The key is stored twice.** | `OrderedMap` keeps a `HashMap<K, usize>` index alongside the entry vector. `indexmap` avoids the second copy with `hashbrown`'s raw-entry API; the core crate is zero-dependency by declaration and `std`'s `HashMap` exposes no equivalent on stable. Mitigated rather than hidden: the bridge's string keys are `Rc<str>`, so the second copy is a refcount, not the text. |
| — | **`undefined` is spelled `None`.** | Core has no JavaScript values, so `DefaultMap<K, V>` stores `Option<V>` and `None` *is* `undefined`. This is what makes B-40 expressible and testable from pure Rust, and it gets `peek` right for free — upstream's `peek` cannot distinguish a missing key from a key holding `undefined` either. |
| — | **The factory is not stored in core.** | Upstream keeps it on the instance; here it is a per-call argument to `get_or_insert_with`, and the bridge holds the `FunctionRef`. The constructor's `typeof factory !== 'function'` check is a JavaScript type test and belongs at the boundary — its message is kept verbatim. A stored `F` would also put a JS callback inside a crate that must not know JavaScript exists. |
| D-06 | **No collection implements `IntoIterator`.** | Unchanged from `sparse-set`: it would hand out a fresh iterator per `for` loop and silently restart. |
| D-07 | **`Symbol.iterator` is installed from Rust, not from the shim.** | Unchanged — but note the table row is `("DefaultMap", "entries")`, not `values`. Upstream's last line aliases `entries`, so spreading a `DefaultMap` yields `[key, value]` pairs. A table that assumed `values` for every module would have been wrong on the second module it met. |
| — | **`MapCursor` is not `crate::cursor::Sequence`.** | The `obliterator` cursor freezes a length at construction and reads elements lazily; a `Map` cursor owns its entry list, skips tombstones, and sees appends. Both are faithful — to different things. One abstraction over both would get one of them wrong, so there are two. |

A re-entrant factory or `forEach` callback is fully supported (it was a stated divergence during
initial development; see the log for how B-31 forced the fix). Verified differentially in
`tests/boundary/reentrancy.js`.

## Fuzz + bench

### Fuzz

Two campaigns, two seeds, **4.37 M operations, zero divergences**:

```
module=default-map seed=42       cases=29525 ops=2939502 wall=120.0s divergences=0
module=default-map seed=20260801 cases=14492 ops=1432987 wall=60.0s  divergences=0
```

Reproduce with `target/release/difffuzz --module default-map --seed 42 --cases 29525`.

The op alphabet covers `get`/`set`/`delete`/`peek`/`has`/`clear`, the three iterator kinds plus
`$next`, `$spread`, and `$forEach` (a callback that mutates the map it is walking — see below).
Observable state is `size` **and** `items`, compared separately after every op, because they
disagree by design once B-40 fires. Keys are drawn from a deliberately small pool that includes
`0`/`'0'`/`-0`/`NaN` so that collisions, overwrites and SameValueZero edge cases are constant rather
than lucky; values are weighted toward `undefined`, the only route to B-40. Full grammar, weights
and rationale: `docs/modules/evidence/default-map.md`.

**The fuzzer was falsified twice, and this is the headline result for this unit: both sabotages
leave the original upstream test file completely green.** Sabotage A resynchronises `size` from
`items` instead of incrementing it — the tidier reading that deletes B-40, and the single most
plausible mis-port of this module; upstream's 7 assertions all still pass. Sabotage B makes
`OrderedMap::set` re-insert an existing key instead of overwriting it in place, so an overwrite
moves the key to the end; upstream's 7 assertions again all still pass (though three of the native
Rust tests do catch it). The fuzzer catches both within roughly 150 cases and shrinks each to a
two- or three-line repro. Both were reverted; both are caught by a single regression seed committed
in `crates/difffuzz/proptest-regressions/default-map.txt`, replayed before any novel case on every
subsequent run. Full repro code and case counts: evidence file.

`$forEach(method, rule, limit)` walks the instance with a callback that calls back into it, driven
by the same cursor `$next` uses so a second hand-written walk cannot drift from it. This module's
walk is the one that differs from every other in the port: a `Map` iteration is live in both
directions, so an entry the callback adds *is* visited and one it deletes ahead of the cursor is
*not*; `set` overwrites rather than adds, which matters here and nowhere else, because a growing
walk here would never terminate. What it does not reach: `difffuzz` compares `mnemonist-core`
against upstream JS, and B-31's hoisted read lived in the napi bridge, outside that loop — no op
alphabet run against core can catch that class of bug. `tests/boundary/reentrancy.js` covers it
instead, driving the real addon with real JS callbacks. One deliberate narrowing, mirrored on both
sides: a selected callback argument that is `undefined` skips the mutation, because the alternative
reaches upstream's `NaN`-indexed swap, which `usize` cannot express and the core does not model.
Disclosed in `fuzz/log.txt`.

### Bench

`bench/results.json` → `modules["default-map"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.

**`mixed-1e6`** — 1e6 mixed `set`/`get-or-insert`/`delete` (50/25/25) over the full 1e6-key domain
(`IK = K = V = u32`; the factory always returns `Some`, so B-40's `size` drift never fires in this
workload): the port is **1.42× slower at p50, 1.15× slower at p99**, while using far less memory
(67.0 MB RSS delta versus 220.6 MB). Re-checked at 4× domain (`mixed-4e6`) before trusting a single
data point: **1.17× slower at p50, 1.27× at p99** — the loss holds at both sizes and never flips
direction across reruns, though run-to-run p50 noise on this host is up to ~32%. Full tables:
evidence file.

The cause is not confirmed. One candidate — that `get_or_insert_with`'s hit path does two hash
lookups (`slot_of` then `entry_at`) instead of one — is **ruled out by direct measurement**:
`entry_at(slot)` is a plain `Vec` index, not a second `HashMap::get`, and a dedicated probe
(`bench-runner --default-map-probe`) shows `peek` and the `get_or_insert_with` hit path costing the
same within 0.7%. The more likely explanation is that at a 1,000,000-key domain, `OrderedMap`'s
internal `HashMap<K, usize>` index no longer fits comfortably in cache, so a uniformly-random key
makes close to every lookup a real DRAM access — consistent with the data (a bigger domain shrinking
the *relative* size of the regression, as a shared memory-latency floor would produce) but not
isolated with its own falsifying measurement, such as a domain-size sweep under hardware cache-miss
counters, which this host's tooling could not provide.

A structural fix — inlining values into the hash map and tracking insertion order separately — was
evaluated and declined. It would touch eight units that use `OrderedMap` directly or transitively,
each of which would need its gate-6 falsification, gate-9 fuzz campaign and gate-10 benchmark
redone; `MapCursor`'s frozen-id-resolved-against-live-slots discipline is a subtle invariant in its
own right, and this repository's history shows arena-and-id structures walked under mutation produce
bugs found late. Independently, this benchmark's op mix is 50% `set` / 25% `delete` / 25%
`getOrInsertWith`, so inlining would help only the read-shaped quarter — a rewrite should not be
expected to close the full gap. Full estimate and reasoning: the log.

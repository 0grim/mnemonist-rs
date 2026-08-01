# default-weak-map

Upstream: `default-weak-map.js` (108 LOC) · `test/default-weak-map.js` — **60 lines, 4 `it`
blocks, 11 assertion statements**.

Port: `crates/mnemonist-core/src/structures/default_weak_map.rs`. Bridge:
`crates/mnemonist-napi/src/default_weak_map.rs` (`WeakKey`, a genuine weak `napi_ref`). Shim:
`tests/bridge/default-weak-map.js`. Fuzz spec:
`crates/difffuzz/src/modules/default_weak_map.rs`.

`DefaultMap`'s twin, backed by a `WeakMap` instead of a `Map`: `get` manufactures and stores a
value from a factory when a key is unseen; `peek`/`set`/`has`/`delete`/`clear` delegate straight
to the backing structure. Read `crates/mnemonist-core/src/structures/default_weak_map.rs`'s own
module docs first — they carry most of the design reasoning; this file adds the six required
sections and states plainly, up front, what is and is not observable through this unit.

---

## What is and is not observable — read this before the rest

A JS `WeakMap` holds its keys **weakly**: an entry becomes eligible for reclamation the instant
nothing outside the map still references its key, and JavaScript gives no way to observe *when*
that happens — no `size`, no iteration, no count, nothing. `DefaultWeakMap` inherits that exactly:
its entire public surface is `clear`, `get`, `peek`, `set`, `has`, `delete` — six methods, each
about one still-referenced key at a time, none of them "the whole map."

That is not a testing inconvenience worked around here; it is designed around, per this batch's
brief. Consequences, stated plainly:

* **There is no "whole state" to compare**, ever, for any key that might have been collected.
  `crates/difffuzz/src/modules/default_weak_map.rs`'s `observations()` is empty — deliberately,
  not an oversight — and every comparison the differential fuzzer makes is a **return value**:
  `get`/`peek`/`has`/`delete`'s results and `set`'s `{"$self": true}` chaining value. That is the
  *entire* observable surface, faithfully covered, not a narrowed campaign.
* **Garbage-collection timing is never fuzzed for.** Forcing a deterministic collection from a
  test would itself be the non-determinism this brief says to design around rather than average
  out. The fuzz grammar's key pool (`fuzz/oracle.js`'s `WEAK_KEY_POOL`, eight real objects) is
  created **once**, at oracle start-up, and held by a plain module-level array for the whole
  process — so those objects are never eligible for collection during any campaign, and neither
  side of the comparison can ever observe an eviction, by construction. This is the honest choice:
  inventing an observation for "has this key been collected" would flake by definition (V8's GC
  schedule is not something either side controls), and a flaky check is worse than an absent one.
* **What *is* observable, and is fuzzed:** everything about a *still-referenced* key's behaviour —
  identity comparison (two different objects are two different keys, the same object is always
  the same key), the `get`/`peek`/`has`/`delete`/`set`/`clear` state machine per key, and the B-242
  defect below, which only concerns values, not key lifetime.

## What upstream tests

Four blocks:

```js
new DefaultWeakMap(null);              // throws /function/
map.get(one).push(1); map.set(two, [2]);
assert.deepStrictEqual(map.get(one), [1]);
assert.deepStrictEqual(map.get(unknown), []);   // reading creates
map.clear();
assert.deepStrictEqual(map.get(one), []);
// ...delete: set/has/delete/has/delete...
// ...peek: get(one).push(1), peek(one), peek(two), has(two)...
```

Characterising the shape of that coverage:

* **Every key is a fresh, distinct object literal** (`{}`), created inline. No key is ever reused
  across `it` blocks, and no two keys in the same block are ever the same object.
* **Every stored value is defined** — an array from the factory, or a literal. `undefined` is never
  stored as a value in the original suite, which is the same shape `default-map.md` describes for
  the identical reason: it is the whole route to B-242 (below), and it is untested.
* **A key is never overwritten** by `set` after already being present.
* **No object other than a genuine plain object is ever used as a key** — no function, no symbol,
  no primitive.
* **`map.get(k).push(v)` appears twice**, the same reference-return idiom `default-map`'s suite
  leans on.

## What upstream does NOT test

**The one real defect (B-242) and its consequences**

1. **`undefined` is never stored as a value**, so B-242 (the factory re-running on every `get` of
   such a key) is unreachable in the original suite.
2. **`has` vs. `get`'s disagreement on a stored `undefined` is never checked.**

**Identity — the entire reason a `WeakMap` exists**

3. **Two distinct objects are never compared as keys in a way that could reveal a false collision**
   (e.g. a naive port hashing by *content* rather than *identity* would still pass every block
   here, since no two keys in the suite are structurally equal either).
4. **A key is never deleted and then re-set.** The one `delete` block deletes once and stops.
5. **`clear()` on a map holding more than one key is never done** (the suite's `clear()` block has
   exactly one entry at the time it clears).

**Never called at all**

6. `inspect()` and the `nodejs.util.inspect.custom` symbol.
7. A function or a symbol as a key (real `WeakMap` accepts both; see "Deliberate divergences").
8. Any non-object key at all, on any method — so upstream's own asymmetry (`peek`/`has`/`delete`
   silently miss; `get`/`set` eventually throw) is entirely untested by the original suite, and is
   pinned in `crates/mnemonist-napi/src/default_weak_map.rs`'s own docs and by hand-probing rather
   than by a gate 4 test.

## What we test in addition

**`crates/mnemonist-core/src/structures/default_weak_map.rs` — 12 tests:**

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | the four blocks, as a baseline |
| `b_242_the_factory_re_runs_on_every_get_of_a_stored_undefined_value` | 1 — B-242, pinned call-by-call |
| `a_defined_value_written_by_the_factory_ends_the_b_242_re_run` | 1 |
| `a_re_triggered_factory_overwrites_in_place_rather_than_duplicating_the_key` | 1 — no duplicate identity leaks out of a re-triggered factory |
| `has_and_peek_disagree_on_a_stored_undefined` | 2 |
| `set_overwrites_an_existing_key_in_place` | — |
| `delete_distinguishes_a_missing_key_from_a_stored_undefined` | 2 |
| `clear_drops_every_entry` | 5 |
| `an_empty_map_reports_nothing` | — |
| `values_mut_reaches_every_stored_slot_including_the_undefined_ones` | — |
| `identity_not_content_decides_a_match_two_equal_but_distinct_keys` | 3 — pins that the matcher, not core, decides identity, and that two predicates that never consider each other equal never collapse to one entry |

**`crates/mnemonist-napi/src/default_weak_map.rs`** carries the object-identity and
weak-reference machinery (`WeakKey`) core cannot express; see "Bugs this found" for the one real
port defect found there during development, and the "Deliberate divergences" table for what is
scoped out.

**The differential fuzzer** covers every return-value question over a fixed, never-collected
8-object pool — see "Fuzz" below.

**Still untested, stated rather than glossed:** gap 6 (`inspect`, not bridged), gap 7 (function/
symbol keys — a deliberate divergence, below), and gap 8's `get`/`set` throw path in its exact
upstream ORDER (factory-then-throw) — this port throws *before* running the factory for a
non-object key, a stated, disclosed ordering divergence (below) rather than the identical
sequence, because reproducing the exact order would require calling this port's typed factory
argument with a non-object value its own signature refuses to carry.

## Bugs this found

**B-242 — `DefaultWeakMap.get` tests the *value*, not the key, so the factory re-runs on every
read of a key holding `undefined`.**
`status: verified against Node 24.18.1`.

```js
DefaultWeakMap.prototype.get = function(key) {
  var value = this.items.get(key);
  if (typeof value === 'undefined') {     // tests the VALUE, not "is the key present"
    value = this.factory(key);
    this.items.set(key, value);
  }
  return value;
};
```

The identical defect class as `default-map.js`'s B-40 — line 1 cannot distinguish "no such key"
from "the key is present and holds `undefined`" — but **without** B-40's `size++` side effect,
because a `WeakMap` has no `size` to drift. The consequence that remains: the factory keeps
re-running, and `has()` reports the key present the whole time even though `get()` behaves as if
it were not:

```text
m.set(key, undefined);
m.has(key);     // true
m.get(key);     // runs the factory (miss, by this line's own logic)
m.get(key);     // runs the factory AGAIN
m.get(key);     // and again
```

Reproduced rather than corrected: `DefaultWeakMap::peek` flattens "missing" and "stored
`undefined`" into one `None`, exactly mirroring `items.get(key)`'s own inability to tell them
apart, and the bridge's `get` re-runs the factory whenever `peek` misses — never checking `has`
first, which is the correction a careful porter would reach for and precisely why it would be a
defect (CLAUDE.md's bug-for-bug mandate). `write_from_factory`/`set` reuse the *matched* entry's
existing key predicate on a hit rather than allocating a fresh identity, which is what keeps a
re-triggered factory from leaking a new weak reference on every re-run — a correctness property
this port needs and upstream gets for free from a single native `WeakMap` slot.

**A real port defect, found during development by a direct probe under `node --expose-gc`, not by
any gate.** `mnemonist_core::structures::default_weak_map::DefaultWeakMap::delete`'s first cut
returned only the removed *value* (`Option<Option<V>>`), silently dropping the removed *key*
inline as part of the `Vec::remove` tuple's discarded half. At the bridge, the key
(`WeakKey`) owns a `napi_ref` that must be explicitly deleted with an `Env` — which core does not
have and the key's own `Drop` cannot reach — so the drop ran through Rust's ordinary `Drop`,
unreleased, printing this file's own leak warning ("a weak map key reference was dropped without
being released") the moment V8 actually collected the abandoned `JsDefaultWeakMap` instance that
had accumulated it. Not caught by the Rust unit suite (a plain `u32` mirror key has nothing to
leak), not by the original mocha suite (which asserts nothing about process-exit warnings), and
not by the differential fuzzer either (same reason — `FuzzKey` is a bare integer). Found by
running the original suite once under `node --expose-gc` and reading stderr, then bisected with a
small standalone script until the exact call sequence (`set`, `has`, `delete`, `has`, `delete` —
the original suite's own third block) was isolated. Fixed by having `delete` hand back
`Option<(K, Option<V>)>` — both halves — so the bridge can release the key too; pinned by
`delete_hands_back_the_removed_key_as_well_as_the_value`.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **Only plain objects are accepted as keys; functions and symbols are rejected**, with a message naming the limit. A real `WeakMap` accepts all three. `test/default-weak-map.js` never constructs a key any way but `{}`. Implementing napi's function/symbol reference paths for a distinction nothing here exercises would be unverifiable scope — the same judgement call `js_key.rs` makes for object keys in the `Map` family, mirrored in the opposite direction: there, object keys are out of scope because nothing tests them; here, they are the *entire point*, and it is function/symbol keys that are out of scope for the identical reason. |
| — | **A non-object key given to `get` is rejected immediately, before the factory runs — upstream runs the factory first and only fails at the internal `items.set`.** Verified against Node 24.18.1: `get(1)` on a fresh map calls the factory (with whatever side effects it has) and *then* throws `TypeError: Invalid value used as weak map key`. Reproducing that exact order would mean calling this port's typed factory (`FunctionRef<FnArgs<(JsSlot,)>, Received>`) with a value its own signature has no slot for. `peek`/`has`/`delete` all match upstream exactly for a non-object key (a quiet miss, never a throw, because a real `WeakMap.prototype.get`/`.has`/`.delete` don't throw for one either) — only `get`'s *ordering*, on the one path no upstream test reaches, differs. |
| — | **A collected key's entry is never proactively released.** No finalizer is registered per key to notice the moment of collection; a dead `WeakKey` (one whose `napi_ref` upgrade fails) simply never matches any future candidate again — the correct answer, since a caller could not present that exact object as an argument again either — but its stored *value* stays retained, taking a slot in the linear scan, until the whole `DefaultWeakMap` itself is finalized. Nothing upstream exposes can distinguish this from prompt reclamation (there is no `size`, no iteration), so this is a memory-shape divergence, not a behavioural one — and implementing per-key finalization for a distinction nothing can observe would be exactly the "building machinery no test can reach" CLAUDE.md and `js_key.rs` both warn against. |
| — | **`WeakKey` is a linear scan (O(n)), not a hash table.** `crate::structures::default_weak_map::DefaultWeakMap` takes an identity predicate per call rather than requiring `K: Hash + Eq`, because JS object identity has no Rust-expressible hash — the same conclusion `js_key.rs` reaches and declines to act on for `Map` keys (out of scope there); here it is unavoidable, because identity comparison is the entire reason this structure exists. Correct, not fast, and nothing about a 60-line test file or a `WeakMap`'s own contract asks for anything faster. |
| — | **`undefined` is spelled `None`**, exactly as in `default-map`, for the identical reason: it is what makes B-242 expressible and testable from pure Rust, and it gets `peek` right for free. |
| — | **`inspect()` is not ported.** It returns the inner `WeakMap`, which does not exist in this port, and nothing asserts on it. |

## Fuzz + bench

### Fuzz

```
module=default-weak-map  seed=42       cases=10931 ops=1089673 wall=60.0s divergences=0
module=default-weak-map  seed=20260801 cases=10814 ops=1090089 wall=60.0s divergences=0
```

Two campaigns, two seeds, **2.18M operations, zero divergences**.
Reproduce with `target/release/difffuzz --module default-weak-map --seed 42 --cases 10931`.

* **Op alphabet:** `get` (5, the mutating read and the only route to a factory call),
  `set` (4), `delete` (3), `peek`/`has` (2 each), `clear` (1). No cursor ops — this module has no
  iteration surface at all (see "What is and is not observable").
* **Key pool:** eight slots, mirrored on the Rust side as `FuzzKey(u8)` — an index, not an object,
  since `mnemonist-napi` is a `cdylib` and cannot be linked into this binary (identical reasoning
  to `default-map`'s own `FuzzKey`). `fuzz/oracle.js`'s `WEAK_KEY_POOL` is the real-object side:
  eight plain objects, created once at oracle start-up, held by a module-level array for the
  process's entire life, so none of them is ever eligible for collection during any campaign — see
  "What is and is not observable."
* **Values:** `undefined` (weight 2, the only route to B-242), `null`, small integers, `'v'`.
* **Constructors:** `undefined`/`null` factories, both already in `fuzz/oracle.js`'s shared
  `FACTORIES` table (added for `default-map`) and reused verbatim: both accept upstream's
  one-argument `(key) -> value` signature unchanged, since they ignore every argument they are
  called with regardless of arity.
* **Observable state, compared after every op:** none (`observations()` is empty — see "What is
  and is not observable"). Every comparison is a return value; this is the entire observable
  surface, not a narrowed one.
* **Deliberately excluded:** any observation of key collection/reclamation (impossible to fuzz
  honestly — see above); object keys with distinguishable identity but coincidental *structural*
  equality (every pool slot is a bare `{}`, so this grammar alone cannot distinguish "compares by
  identity" from "compares by deep equality" the way a real adversarial case would — that
  distinction is instead pinned by the core module's own
  `identity_not_content_decides_a_match_two_equal_but_distinct_keys` Rust test, which controls the
  matcher directly); a non-object argument to any method (bridge-specific, and this grammar only
  ever generates pool-object keys by construction).

### Falsification (gate 6)

**The assertion named first:** the probe script's `calls === 3` (mirroring the core Rust test
`b_242_the_factory_re_runs_on_every_get_of_a_stored_undefined_value`'s own `assert_eq!(calls, 3, ...)`),
run against the real compiled addon rather than against core directly — because the bridge is
where B-242's *composition* (peek-miss triggers the factory) actually lives; core's `peek`/
`write_from_factory` are neutral primitives a caller composes, the same way upstream's own bug is
a composition of two lines, neither wrong on its own.

**The sabotage:** `crates/mnemonist-napi/src/default_weak_map.rs`'s `get` changed to check
`has()` (key presence) before running the factory, instead of `peek()` (value definedness) — the
"fix" a careful porter would reach for.

**Confirmed red:** a direct script against the rebuilt addon (`set(key, undefined); get(key) x3`)
reported `calls === 0`, not `3` — even sharper than expected, since with this sabotage `has()` is
already `true` from the `set()` call, so the factory never runs even once.

**Confirmed green where expected, for a stated reason — and this IS the interesting finding for
this unit.** The original mocha suite stayed green (`4 passing`): it never counts factory
invocations, so it cannot see this class of bug either way. **The differential fuzzer *also*
stayed green** (`500 cases, 49416 ops, 0 divergences`) — and this is not a miss, it is a structural
fact stated up front in this document's own module docs and `default_map.rs`'s: *the differential
fuzzer compares `mnemonist-core` against upstream JS; the napi bridge is not in that loop at all.*
A bridge-only composition bug is invisible to it by construction, the same way B-31 was before this
port started holding core state in a `RefCell`. This is the sharpest illustration this batch has of
CLAUDE.md's own point: passing every available check is not the same as being correct, and knowing
*which* check would have to exist to catch a given class of bug is worth more than another green
campaign.

**Reverted; confirmed green again:** the same script reports `calls === 3`, and the original suite
still passes (`4 passing`).

### Bench

**Not run.** Gate 10 is deferred to the batched quiet pass (DESIGN.md §7.3). `default-weak-map` is
therefore **complete except gate 10** and correctly absent from `tests/scope.txt` until that pass
lands.

One thing to watch when it does: `WeakKey::matches` is O(n) per lookup (a linear scan with one
`napi_strict_equals` call per live entry), which is the honest cost of not having a hashable
identity — see "Deliberate divergences." Whether that shows up in a benchmark at any realistic map
size is a question for the measurement, not for this document.

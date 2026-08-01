# lru-cache

Upstream: `lru-cache.js` (436 LOC), `lru-map.js` (261 LOC), `lru-cache-with-delete.js` (287 LOC),
`lru-map-with-delete.js` (287 LOC) — **1,271 LOC across four files** · `test/lru-cache.js` —
**497 lines, 21 distinct `it` blocks (88 executions across the four variants), 122 assertion
statements**.

Port: `crates/mnemonist-core/src/structures/lru_cache.rs` — one generic engine,
`LruCache<IK, K, V>`, serving all four upstream files (see that file's own module docs for why).
Bridge: `crates/mnemonist-napi/src/lru_cache.rs`, `lru_cache_with_delete.rs`, `lru_map.rs`,
`lru_map_with_delete.rs`. Shims: `tests/bridge/lru-cache.js`, `lru-map.js`,
`lru-cache-with-delete.js`, `lru-map-with-delete.js`. Fuzz spec:
`crates/difffuzz/src/modules/lru_cache.rs` — four `ModuleSpec`s, one shared engine, matching the
port's own shape.

`test/lru-cache.js` `require`s all four upstream files and runs one `makeTests` suite against each
of them via a shared closure, so under DESIGN.md §1.1 this is **one unit**, and a missing sibling —
a bridge that only ported `LRUCache` — would fail the whole file with zero partial credit. This is
also the largest single unit ported so far: four source files, one 497-line test file, four bridge
crates, one fuzz spec covering all four.

---

## What upstream tests

Twenty-one distinct `it` blocks, run once per variant (`makeTests(Ctor, name)` called four times),
for 88 total executions. The shape of the coverage:

* **One long walkthrough** (`should be possible to create a LRU cache`) that is really the whole
  contract in one test: capacity, `set`, `size`, eviction at capacity, promotion on `get`, `peek`
  not promoting, and — the one place the two families' storage actually shows through the API —
  asserting `Object.keys(cache.items).length` for the object-backed pair and `cache.items.size` for
  the `Map`-backed pair. Every other block is a narrower slice of the same mechanics.
* **`setpop` gets three blocks**: the growth case (`null`), the eviction case
  (`{evicted: true, key: 'one', value: 1}`), and the overwrite case
  (`{evicted: false, key: 'three', value: 3}`). All three evicted/overwritten keys in these blocks
  are non-empty strings — see "Bugs this found".
* **`capacity = 1`** gets its own block: every `set` evicts the sole entry.
* **`Cache.from`** gets two blocks: from a `Map` (capacity guessed via `guessLength`), and with
  explicit `Uint8Array`/`Float64Array` `Keys`/`Values` classes, both as a direct constructor call
  and via `.from`.
* **`forEach` and `Symbol.iterator`** each get one block, and **neither ever mutates from inside the
  walk.** This is the single most consequential gap in the whole file — see below.
* **`delete`/`remove`** — gated to the two `-with-delete` variants — get eight blocks between them:
  basic delete/remove maintaining order, deleting an absent key, deleting down to empty and back,
  falsy stored values (`""`, `0`, `false`, `null`, an array, an object) round-tripping through
  `remove`, a custom missing-indicator argument, and two "healthy workout" blocks that interleave
  `set`/`get`/`delete` (respectively `remove`) across ten-plus operations on a five-slot cache.
  These two are the closest the original suite comes to a fuzz test, and they are the reason
  `delete`/`remove`'s hole-reuse path is well covered by gate 4 alone.
* **Invalid capacity** gets six sub-cases per variant (`undefined`, `{}`, `-1`, `true`, `1.01`,
  `Infinity`), all asserting only that *something* throws matching `/capacity/` — never which of
  the two capacity error messages, and never the exact wording.

## What upstream does NOT test

Everything below is reachable through the public API and never exercised by `test/lru-cache.js`.

**A mutating `forEach` callback — the single biggest gap, and where both port defects this unit
found were hiding**

1. **A callback that promotes (`set`s an existing key, splaying it to the front) the entry the walk
   is about to visit next.** Upstream's own loop reads `forward[pointer]` *after* the callback
   returns:
   ```js
   while (i < l) {
     callback.call(scope, values[pointer], keys[pointer], this);
     pointer = forward[pointer];   // after, not before
     i++;
   }
   ```
   A walk built on the wrong timing captures the *old* `forward[pointer]` before the callback's
   promotion relinks it. Found by this unit's own differential fuzzer on its first, un-logged
   campaign — see "Bugs this found".
2. **A callback that `delete`s a key the walk has not yet visited**, on the `-with-delete` pair. The
   frozen bound (`l = this.size`, captured at entry) does not shrink when the callback deletes
   something, so the walk can revisit a pointer whose slot was just unlinked. See "Bugs this
   found" — this one was caught by reading, before the fuzz grammar for this unit existed at all.
3. **A callback that deletes a key and a *later* `set` reuses that same freed pointer before the
   walk reaches it.** The walk then reads the NEW occupant, not the old one — upstream's own array-
   of-pointers algorithm has no way to tell "stale" from "reused" apart at that level, so this is
   not a bug on either side, just an unexercised interaction. Pinned in
   `crates/mnemonist-core/src/structures/lru_cache.rs`'s own test suite
   (`a_freed_pointer_reused_before_a_stale_walk_reaches_it_surfaces_the_new_occupant`).

**`setpop`'s falsy-key blind spot**

4. **An evicted key that is JS-falsy** (`0`, `""`, `false`, `NaN`, `null`, `undefined`). Upstream's
   own `setpop` decides whether to report an eviction with `if (oldKey)` — a truthiness check, not
   a definedness check — so a real eviction of a falsy key silently returns `null`. Every `setpop`
   block in the original suite evicts or overwrites a plain non-empty string. See "Bugs this
   found", B-140.

**Everything about the object-backed pair's key coercion beyond the one walkthrough line**

5. **`ToPropertyKey` on anything but a primitive.** `this.items[key] = pointer` coerces `key`
   through JS's property-key conversion; an object key would go through `toString`/`valueOf`. No
   test ever passes one. The bridge restricts index keys to what `JsKey` classifies (`undefined`,
   `null`, booleans, numbers, strings) for the same reason `default-map`'s bridge does — see
   `mnemonist_napi::lru_cache`'s module docs and "Deliberate divergences" below.
6. **Two different raw keys that coerce to the same property string** (`3` and `"3"`) are never
   both used against the same cache in the original suite, so the object-backed pair's whole reason
   to differ from the `Map`-backed pair — SameValueZero vs. `ToPropertyKey` — is asserted only
   indirectly, through the `items` length/size difference in the one walkthrough test, never through
   an actual collision.

**`Keys`/`Values` array classes**

7. **A narrowing class that changes what `this.K`/`this.V` hold** relative to what was inserted
   under (e.g. a `Uint8Array` truncating `300` to `44`) is exercised by the one specialized-cache
   block, but that block never forces an *eviction* of a narrowed key — so the re-derivation gap
   `mnemonist_core::structures::lru_cache`'s own docs describe (eviction re-reads `this.K[pointer]`,
   not the index key `set` was called with) is never actually triggered by the original suite. Ours
   is a Rust unit test instead
   (`eviction_re_derives_the_index_key_from_the_stored_key_and_can_leave_it_stale`).
8. **A `Keys`/`Values` argument that is neither `undefined` nor a function** — upstream's
   `typeof Keys === 'function' ? new Keys(capacity) : new Array(capacity)` silently falls back to a
   plain array for anything else (a string, a number, an object), and no test supplies one.

**Never called at all**

9. `has` after a `clear()`. `peek` on an evicted (not merely absent) key. `capacity`/`size`/`items`
   read on a freshly-cleared cache with a *pending* forEach walk still open over the old state.

## What we test in addition

**Rust native tests** — `crates/mnemonist-core/src/structures/lru_cache.rs` (13):

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_walkthrough`, `setpop_reports_none_overwritten_and_evicted`, `capacity_of_one_evicts_on_every_new_key`, `delete_and_remove_maintain_lru_order`, `peek_does_not_disturb_order`, `clear_resets_bookkeeping_but_a_stale_slot_is_never_reachable`, `keys_and_values_project_the_same_walk_differently` | the upstream blocks, as a baseline |
| `zero_capacity_is_refused` | the numeric half of the invalid-capacity guard |
| `a_deleted_slot_is_reused_by_the_next_insert` | the hole-reuse path the two "healthy workout" blocks exercise indirectly |
| `eviction_re_derives_the_index_key_from_the_stored_key_and_can_leave_it_stale` | gap 7 |
| `a_delete_of_the_walks_next_unvisited_pointer_yields_stale_data_not_a_panic` | gap 2 — the port defect this pins |
| `a_remove_of_the_walks_next_unvisited_pointer_yields_stale_data_not_a_panic` | the `remove` half of the same defect |
| `a_freed_pointer_reused_before_a_stale_walk_reaches_it_surfaces_the_new_occupant` | gap 3 |

**Differential fuzzer** — see "Fuzz + bench" for the campaigns. Its own grammar is worth describing
here because the module docs it lives next to (`crates/difffuzz/src/modules/lru_cache.rs`) explain
the *mechanism*; this is what it actually closes: gap 1 (found on contact — see "Bugs this found"),
gap 4/B-140 (found on contact too), gap 6 (the object-backed vs. `Map`-backed key pool deliberately
includes `Int(0)`/`Str("0")`, which collide for one family and not the other), and a
**self-check on the grammar itself** (`grammar_self_check`, two tests, no oracle involved) that
samples 400 generated programs and asserts a floor on how often `set`/`setpop` evict and `delete`
succeeds — see "Fuzz + bench" for the measured numbers.

**Still untested, stated rather than glossed:** gap 5 (`ToPropertyKey` on an object key — the bridge
does not implement it at all, by design, see "Deliberate divergences"), gap 8 (a non-function,
non-`undefined` `Keys`/`Values` argument — the bridge's `resolve_class` handles it but no test
exercises the path), and gap 9. No `tests/boundary/lru-cache.js` was written for this unit; the
differential fuzzer and the Rust native tests are the whole of what fills these gaps, which is a
narrower base than modules with a boundary spec (e.g. `heap`) have.

## Bugs this found

One upstream defect, confirmed against Node 24.18.1, plus one already-known upstream inconsistency
this unit's own bridge code had documented but never written up, plus two real port defects — found
before and during fuzzing, not by it in the second case — both fixed.

### B-140 — `setpop` silently drops the eviction report when the evicted key is falsy

`status: verified against Node 24.18.1` · `lru-cache.js`, `lru-map.js` (and both `-with-delete`
siblings, via prototype copy) · found by the differential fuzzer's first campaign against this
grammar

`setpop`'s own eviction branch is:

```js
if (oldKey) {
  return {evicted: true, key: oldKey, value: oldValue};
}
else {
  return null;
}
```

`if (oldKey)` is a **truthiness** check, not `oldKey != null`. So evicting a key that is JS-falsy —
`0`, `""`, `false`, `NaN`, `null`, `undefined` — reports `null`, exactly as if nothing had been
evicted, even though the entry really was displaced and the caller's new key/value really did take
its slot:

```js
var cache = new LRUCache(3);
cache.set(0, 'a'); cache.set(1, 'b'); cache.set(-1, 'c');   // full
cache.setpop('d', 'e');   // evicts key 0 -- but returns null, not {evicted:true,...}
```

Reproduced in the port: `mnemonist_core::structures::lru_cache::LruCache::set_pop` itself always
reports every eviction correctly (it has no notion of JS truthiness at all — a Rust `Option` is not
falsy), so the bug is reproduced at the bridge, in `is_js_truthy`
(`crates/mnemonist-napi/src/lru_cache.rs`), which all four `setpop` methods now gate the `Evicted`
arm on. **A port that reported every eviction, as the pre-fix bridge did, is more correct than
upstream and is therefore a defect** per CLAUDE.md's bug-for-bug mandate. `test/lru-cache.js`'s
three `setpop` blocks all evict or overwrite a non-empty string key, so gate 4 never touched this
path; the fuzz grammar's key pool includes four JS-falsy raw keys (`Int(0)`, `Bool(false)`, `Null`,
`Undefined`) out of ten; it found the divergence on the third generated case.

### B-142 — `lru-map.js`'s own `.from` names the wrong module in its error

`status: verified against Node 24.18.1` · `lru-map.js`

```js
throw new Error('mnemonist/lru-cache.from: could not guess iterable length. ...');
```

`lru-map.js:241` — a copy-paste artefact from `lru-cache.js`, whose own `.from` has the identical
line with the correct module name. Confirmed against the other two files: `lru-cache-with-delete.js`
and `lru-map-with-delete.js` both get their own module name right. So the bug is specific to this
one file, not systemic to the family. Reproduced verbatim in
`crates/mnemonist-napi/src/lru_map.rs`'s `CANNOT_GUESS` constant, which is commented at the point of
use to say so. Not independently fuzzable (it fires from `Cache.from`'s argument-arity resolution,
before a cache exists at all, so it is an `init`-time error in the oracle protocol — see
`fuzz/oracle.js` — rather than an op comparison); found by reading, the same way B-70..B-79 were
for `heap`.

### Two defects in the port, both fixed, one found by design review and one by the fuzzer's first campaign

Neither is upstream's fault; both are recorded here rather than only in a commit message, following
the precedent `docs/modules/heap.md` set for defects a gate never caught.

**1 — `delete`/`remove` nulling `this.K[pointer]`/`this.V[pointer]`, which upstream never does.**
`LruCache::unlink` used to set both slots to `None` as part of unlinking a deleted pointer from the
list. Upstream's `delete`/`remove` never touch `this.K`/`this.V` at all — only the linked-list
splice and the hole record. So a `keys()`/`values()`/`entries()`/`forEach` walk whose frozen `size`
bound had not yet reached a pointer, when a callback (or an interleaved op, for the lazy iterators)
deleted exactly that pointer, hit the walk's own `.expect("a pointer reachable from head within
size steps is always live")` — which the nulling had just made false — and **panicked**. Found by
reading, before any fuzz campaign for this unit had run at all: the shape (a hole-bearing
`-with-delete` variant, a walk left open across a mutation) was exactly what CLAUDE.md's brief for
this unit named as the interesting territory, so it was checked directly with a scratch Rust probe.
Confirmed to panic; fixed by not nulling either slot in `unlink`, and by changing `remove` to
`.clone()` the value instead of `.take()`-ing it (which independently zeroed it) — see
`LruCache::unlink`'s and `LruCache::remove`'s doc comments for the reasoning in full. Pinned by three
Rust unit tests, including one confirming that a pointer freed and then *reused* before a stale walk
reaches it correctly surfaces the new occupant rather than stale data — because that is a real
possible outcome too, and both are upstream's actual (unglamorous) behaviour.

**2 — `forEach` advancing its pointer before the callback ran, where upstream advances after.**
Both the fuzz spec's own `$forEach` handling and the napi bridge's `for_each_entries` originally
opened an `Entries` walk via `Sequence`/`CursorState` — the same machinery `keys()`/`values()`/
`entries()` correctly use, because their lazy-iterator closures advance their own pointer before
ever returning control to whatever called `.next()`, which is exactly `Sequence::slot`'s eager-
advance timing. `forEach` is different: upstream's callback runs **while control is still inside its
own loop body**, one statement *before* `pointer = forward[pointer]`. Reusing the eager-advance
walk for `forEach` reproduced the *iterators'* timing for a method whose real timing is the
opposite. Found by the differential fuzzer's very first campaign against this grammar (0 logged
campaigns in `fuzz/log.txt` before the fix; the eight campaigns recorded in "Fuzz + bench" all
post-date it): a `$forEach("set", "arg1,arg0", ...)` program — the very shape CLAUDE.md's brief
called out ("interleaved with mutation") — disagreed on the third callback invocation, port seeing
`[undefined, 1]` where upstream re-saw `["w", true]`. Fixed by `ForEachWalk`
(`mnemonist_core::structures::lru_cache`), which splits "read the current position" from "advance,
reading `forward` live" into two calls the caller controls, so the caller's own mutation always runs
between them — exactly upstream's loop shape. `test/lru-cache.js`'s own `forEach` block never
mutates from inside the callback, so gate 4 could not have found this; the minimised repro is
checked in at `crates/difffuzz/proptest-regressions/lru-cache.txt` with a provenance header.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| D-89 | **`this.K[pointer]`/`this.V[pointer]` are left stale after `delete`/`remove`, never nulled.** | The fix for the panic above; see "Bugs this found". Reproducing upstream's own lack of a null-out is what makes a walk left open across a delete return upstream's actual (stale, not `undefined`) answer instead of crashing. |
| D-90 | **`forEach` is `ForEachWalk`, not `Sequence`/`CursorState`.** | The fix for the timing bug above. `keys`/`values`/`entries` correctly stay on `Sequence`, because their own timing already matches upstream's lazy-iterator closures; only `forEach`'s callback-before-advance shape needed a different primitive. |
| D-91 | **The object-backed pair's index key is restricted to what `JsKey` classifies** (`undefined`, `null`, booleans, numbers, strings) — an object key is rejected rather than run through `ToPropertyKey`. | `JsKey` was built for the `Map`-backed pair and for `default-map`, and no test in the original suite ever supplies an object key to either family. Implementing `ToPropertyKey`'s full object-coercion path (`toString`/`valueOf`, `Symbol.toPrimitive`) for a path nothing exercises would be unverifiable scope; see gap 5. |
| D-92 | **The fuzz grammar never narrows a key through an `ArrayClass`.** | `index_of` and `to_index` are the same function for every one of the four fuzz specs (no `Keys`/`Values` array class is ever generated), so the eviction re-derivation gap `mnemonist_core::structures::lru_cache`'s own docs describe — where a narrowing store can make eviction's re-read of `this.K[pointer]` disagree with the key `set` was called with — is *unreachable by this grammar, by construction*. It is real (see the Rust unit test that pins it) and is exercised only there, not by fuzzing. |
| D-93 | **`lru-map`/`lru-map-with-delete`'s fuzz spec omits `items` from `observations()`.** | Upstream's `this.items` is a genuine `Map` there, encoded by the oracle as an ORDER-SENSITIVE list. `mnemonist_core`'s index is a plain `std::collections::HashMap`, whose iteration order is unrelated to insertion order and would drift from a real `Map`'s on nearly every op — not because of a port defect, but because nothing about a lookup-only index needs to track insertion order. Comparing it in full would manufacture a divergence out of an implementation detail. The object-backed pair's `items` (a plain object, compared as an order-independent JSON object) has no such problem and is compared in full — see the fuzz module's own docs. This is exactly the judgement call the real bridge already made (`mnemonist_napi::lru_map`'s own `items` getter returns only `{size: N}`, for the identical reason). |
| — | **B-142 is reproduced verbatim, not fixed.** | `lru-map.js`'s `.from` names the wrong module in one error message; `lru-map.rs`'s bridge raises the identical wrong string. |

## Fuzz + bench

### Fuzz

```
module=lru-cache               seed=42       cases=4665 ops=711010  wall=60.0s divergences=0
module=lru-cache               seed=20260801 cases=4869 ops=731558  wall=60.0s divergences=0
module=lru-cache-with-delete   seed=42       cases=5027 ops=755972  wall=60.0s divergences=0
module=lru-cache-with-delete   seed=20260801 cases=5076 ops=759840  wall=60.0s divergences=0
module=lru-map                 seed=42       cases=5534 ops=837947  wall=60.0s divergences=0
module=lru-map                 seed=20260801 cases=5649 ops=844111  wall=60.0s divergences=0
module=lru-map-with-delete     seed=42       cases=5559 ops=834690  wall=60.0s divergences=0
module=lru-map-with-delete     seed=20260801 cases=5988 ops=896591  wall=60.0s divergences=0
```

Eight campaigns, two seeds, **6.37M operations, zero divergences** — against a build that already
carries both fixes from "Bugs this found". Both defects were found by this same grammar before any
of these eight campaigns were run at all (an un-logged, un-timed first pass during development), so
these eight measure the grammar *after* the interesting bugs, not instead of them. Reproduce with
e.g. `target/release/difffuzz --module lru-cache-with-delete --seed 42 --cases 5027`.

* **Op alphabet:** `get` (weight 8, the heaviest of any op — it is the mutating read, and an LRU's
  whole point is that a read changes recency), `peek`/`has` (2 each, the non-mutating controls),
  `set` (6), `setpop` (3), `clear` (1), `$iter`/`$next`/`$spread` (2/4/2, the lazy-iterator lifecycle
  ops), `$forEach` (2, `for_each_strategy` over a small mutation table). The `-with-delete` variants
  add `delete` (4) and `remove` (3), both weighted above `clear` specifically because interleaving
  them with eviction is where B-140's sibling defect and the two port defects above were found.
* **Constructor alphabet:** capacity `1..=6` and nothing else — deliberately small relative to the
  op-count ceiling (`program_len` widened to `1..300`), so a generated program cycles the ring many
  times over at every capacity in range. DESIGN.md's own warning is explicit: a campaign whose
  capacity is large relative to its op count proves only that a map stores things.
* **Key pool:** ten keys mixing `Str`, `Int`, `Bool`, `Null` and `Undefined` (mirroring `JsKey`'s
  primitive shapes), including the one collision unique to this family — `Int(0)` and `Str("0")` are
  the same key for the object-backed pair (`ToPropertyKey` coerces both to `"0"`) and two different
  keys for the `Map`-backed pair (SameValueZero never conflates them) — and four JS-falsy values
  (`Int(0)`, `Bool(false)`, `Null`, `Undefined`), which is what made B-140 reachable on the third
  generated case.
* **Observable state, compared after every op:** `capacity`/`size`/`head`/`tail` always; the
  object-backed pair's full `items` (every live key's pointer, an order-independent JSON object —
  see D-93 for why the `Map`-backed pair's own `items` is left out).

**How often eviction actually fired** — measured directly, not inferred from the weights, by
`grammar_self_check` (`crates/difffuzz/src/modules/lru_cache.rs`, no oracle, no `node`): over 400
generated programs (up to 300 ops each),

```
lru-cache grammar (no delete): 60,220 ops, 5,760 evictions (9.6% of ops)
lru-cache-with-delete grammar: 63,235 ops, 3,176 evictions (5.0%), 1,329 successful deletes (2.1%)
```

Both are healthy: only `set`/`setpop` can ever evict at all (roughly a quarter of the alphabet's
weight), so 5–10% of *all* ops evicting means somewhere near half of every `set`/`setpop` call
actually displaces something — this is not a grammar that mostly proves a map stores things. The
`-with-delete` variant's lower eviction rate is expected: every successful `delete` shrinks the live
set and hands a hole back to `insert_new`, which the next `set` reuses *before* growth resumes, so
some inserts that would have evicted under the plain grammar instead fill a hole. Both self-check
tests assert a floor on these figures (20:1 and 40:1/100:1 respectively) so a future change to the
weights that regresses back toward "write-only" fails loudly rather than silently.

### Falsification of the port (gate 6)

**The assertion the sabotage had to break was named first:** the last `assert_eq!` in
`structures::lru_cache::tests::reproduces_the_upstream_walkthrough` —
`entries(&cache) == vec![("four", 4), ("two", 5), ("three", 3)]`, which depends on `cache.get("four")`
having promoted `"four"` to the front a few lines earlier — and the equivalent upstream assertion,
`test/lru-cache.js`'s own `Array.from(cache.entries())` check right after `cache.get('four')`.

**The sabotage:** for an LRU the sharp target is recency, not storage, so `LruCache::get` had its
`self.splay_on_top(pointer);` call commented out — the read still returns the right value, it just
stops moving anything.

**Confirmed red, in all three places a promotion-on-read failure could be caught:**

* The named Rust assertion: `left: [("two", 5), ("four", 4), ("three", 3)]` vs.
  `right: [("four", 4), ("two", 5), ("three", 3)]` — `"four"` never moved.
* The original suite: `72 passing, 16 failing` (down from 88 passing) — every block that reads an
  entry and then asserts on order went red.
* **The differential fuzzer noticed too**, and fast: `target/release/difffuzz --module lru-cache
  --seed 42 --cases 200` found a divergence in **74 operations, 0.4 seconds**, minimised to nine
  operations:

  ```
  divergence in observable state after op #24: get(1)
    head:
      port:     1
      upstream: 0
    tail:
      port:     0
      upstream: 1
  ```

  `head`/`tail` disagreeing immediately after a `get` is exactly what a broken promotion looks like
  from the outside — the sharpest possible confirmation that the grammar's heavy `get` weighting
  (see "Fuzz + bench") is pulling its weight.

**Reverted; confirmed green again** at all three: the named assertion passes, `88 passing` on the
original suite, and a 200-case replay of the same seed comes back `0 divergences`.

**Nothing was found to be blind here.** Every instrument this unit has — the Rust unit test, the
original suite, and the differential fuzzer — caught the sabotage independently, which is the
outcome gate 6 exists to distinguish from "a green light that was never capable of going red."

### Bench

**Not run.** Gate 10 is deferred to a serial pass on an idle machine (DESIGN.md §7.3): other agents
were working on this repository while this unit landed, and a contended run has inflated both sides
2–3× before (see `docs/modules/heap.md`). `lru-cache` is therefore **complete except gate 10** and is
deliberately *not* in `tests/scope.txt`; `tests/verify.sh` will say so, which is the intended state
rather than an oversight.

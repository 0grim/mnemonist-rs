# bi-map

Upstream: `bi-map.js` (195 LOC) · `test/bi-map.js` — **189 lines, 12 `it` blocks, ~35 assertion
statements**.

Port: `crates/mnemonist-core/src/structures/bi_map.rs`. Bridge: `crates/mnemonist-napi/src/bi_map.rs`.
Shim: `tests/bridge/bi-map.js`. Fuzz spec: `crates/difffuzz/src/modules/bi_map.rs`.

`BiMap` keeps two `Map`s in lockstep — `items` (key → value) and `inverse.items` (value → key) — so
a lookup works in either direction and `set`/`delete` keep both sides a true bijection.
`InverseMap` is not a second data structure upstream; it is a thin object that shares `set`/`delete`/
`clear` with `BiMap` (literally `InverseMap.prototype.set = BiMap.prototype.set`, one function bound
to two receivers) and delegates its six read-only methods straight onto the *other* `Map`. The port
mirrors that: one `BiMap<K>` backs both directions, and the bridge's `JsBiMapInverse` is a live view
over the same `RefCell`, not a copy.

---

## What upstream tests

Twelve `it` blocks, and every one of them is a happy path:

* **`set` three times over**: a fresh key/value pair, a key already pointing elsewhere, and a value
  already claimed by another key — the two mid-branches of the four-way constraint resolution.
  Never both collisions in the same call.
* **`size`/`inverse.size` checked after every `set`**, always in agreement, because nothing in the
  suite ever desynchronises them on purpose.
* **`delete`** — one call, one existing key, checked from both directions afterwards
  (`has('one') === false`, `inverse.has('hello') === false`).
* **`clear`** — one call, one entry, and the assertions afterwards are `map.size === 0` and
  `map.has('one') === false`. **`map.inverse.size` is never read after `clear`.**
* **`get`/`has`**, each direction, on a map with one entry.
* **Iteration**: `forEach`, `keys()`, `values()`, `entries()`, `for...of` — each pair tested from
  both `map` and `map.inverse`, with two entries, checked in insertion order.
* **`BiMap.from`**, one `Map` with two pairs.

## What upstream does NOT test

**The exact gap that let B-120 through the door.** `clear`'s own `it` block asserts `map.size` and
`map.has`, never `map.inverse.size`. Upstream's shared `clear` function resets only the counter
belonging to whichever side called it (`this.size = 0`), leaving the other stale — invisible to a
suite that only ever calls `clear()` from the forward side and only ever checks the forward
counter afterwards.

**Never called at all:**

1. **`inverse.clear()`.** The only `clear` call in the file is `map.clear()`; the mirror-image
   staleness (`map.size` left stale after `map.inverse.clear()`) has no test either.
2. **Three-way collisions**: `set` where the key was pointing elsewhere *and* the value was claimed
   by another key, in the same call. The suite's "handle constraints" block tests each collision
   in isolation, on a fresh map.
3. **`delete` on a missing key.** The one `delete` call in the file always finds its key.
4. **Reinsertion order.** Nothing deletes a key and re-adds it to check where it lands (`OrderedMap`
   inheritance: a re-added key moves to the end, the same as `default-map`).
5. **`inverse.set`/`inverse.delete`** called directly (as opposed to read through `.get`/`.has`).
   The suite only ever writes through the forward `map`, and reads back through `.inverse`.
6. **Iteration on an empty map**, or after a `clear()`.
7. **`forEach`'s `scope` argument.**
8. **`inspect()`** and its `nodejs.util.inspect.custom` symbol.

## What we test in addition

`crates/mnemonist-core/src/structures/bi_map.rs` — 12 tests:

| Test | Closes gap |
|---|---|
| `reproduces_the_upstream_suite` | 1:1 port of all twelve upstream blocks, as a baseline |
| `clear_desyncs_size_from_inverse_size_b_120` | 1 — B-120, both directions, plus the healing-on-next-mutation property |
| `clear_called_on_the_inverse_view_also_empties_the_forward_map` | 1 — the underlying-maps half (both empty regardless of direction) |
| `set_can_rebind_both_sides_of_the_bijection_in_one_call` | 2 |
| `delete_on_a_missing_key_reports_it_and_changes_nothing` | 3 |
| `a_deleted_key_reinserted_moves_to_the_end` | 4 |
| `the_inverse_view_supports_the_full_method_set` | 5 — `set_reverse`/`delete_reverse` exercised directly |
| `rebinding_a_key_releases_its_old_value_from_the_inverse` | — the two-branch case in isolation |
| `rebinding_a_value_releases_its_old_key_from_the_forward_map` | — the other two-branch case |
| `set_is_a_no_op_when_the_exact_relation_already_exists` | — insertion order is untouched on a no-op |
| `an_empty_map_reports_nothing` | 6 |

Gaps 7 and 8 are stated rather than closed — see Deliberate divergences.

## Bugs this found

### B-120 — `BiMap.prototype.clear` resets only ONE of its two size counters

`status: VERIFIED against Node 24.18.1` · found by differential fuzzing (proptest, seed 42), on the
very first campaign run against this module.

`BiMap`/`InverseMap` share one `clear`:

```js
function clear() {
  this.size = 0;
  this.items.clear();
  this.inverse.items.clear();
}
```

Both underlying `Map`s are genuinely emptied regardless of which side calls it, but only `this.size`
is reset. `bimap.clear()` leaves `bimap.inverse.size` at whatever it was; `bimap.inverse.clear()`
leaves `bimap.size` stale instead:

```js
var m = new BiMap(); m.set('a', 'a');
m.clear();
m.size            // 0
m.inverse.size    // 1, STALE — items.size and inverse.items.size are both 0
```

The staleness is not permanent — the next `set`/`delete` on either side resynchronises both counters
from the live maps (`this.size = this.items.size; this.inverse.size = this.inverse.items.size;`),
which is why the upstream suite, whose only `clear` call is the last thing it does with that map, has
never seen it. `delete` on an absent key, though, returns `false` **before** touching either counter,
so a no-op `delete` right after a `clear()` leaves the stale counter stale for one more op — the case
the second fuzzing round caught (below).

**Found in two rounds, because the first fix over-corrected:**

1. The initial port derived both `size()`/`inverse_size()` from `OrderedMap::len()`, so `clear()`
   incidentally zeroed both — a real defect (more correct than upstream). Caught in **18 cases
   (0.3s)** on `set("a","a"); clear();`.
2. The fix added two real stored counters, reset asymmetrically — but resynchronised both
   unconditionally after every `delete` call. Since a no-op `delete` (absent key) is reachable right
   after a genuine `clear()` and upstream's `del` does not touch either counter on that path, the
   unconditional resync "healed" the still-stale counter one operation early. Caught in **177 cases
   (0.3s)** on `set("a","a"); clear(); delete("a");` — the very next campaign run against round 1's
   fix.

Fixed by resynchronising `delete`/`delete_reverse` only when something was actually removed. Both
seeds are committed with provenance in `crates/difffuzz/proptest-regressions/bi-map.txt` — see "Fuzz"
below for what that file's un-labelled first entry turned out to be.

**Strong candidate**, and worth reading past the bug itself: the module doc for
`mnemonist_core::structures::bi_map` had already analysed and *named* this exact defect in prose,
including the reproduction above, before the implementation caught up to it. A doc comment
describing intended behaviour is a claim, not evidence that the code behind it does that — this is
the reminder this project's own process left for itself.

## Deliberate divergences

| # | Divergence | Why |
|---|---|---|
| — | **`InverseMap` is a view, not a second value.** | Upstream constructs a real second object whose six generic methods delegate to `Map.prototype[name].apply(this.items, ...)`. One `BiMap<K>` backs both directions here; the bridge's `JsBiMapInverse` holds a `SharedReference` to the *same* `RefCell<Core>` the `JsBiMap` owns, so a write through either object is visible to the other, exactly as upstream's shared `Map`s are. |
| — | **`size`/`inverse_size` are real stored counters, not `OrderedMap::len()`.** | Required to reproduce B-120 at all — a derived counter cannot desync from anything. |
| — | **`inspect()` is not ported.** | A Node display convenience with no upstream assertion, the same call made for `bit-set`. |
| — | **`forEach`'s third callback argument** is the bridge object, not upstream's inner `Map`. | Same divergence made once for the whole T3 family (`docs/modules/default-map.md`); there is no Rust equivalent to hand out. |

## Fuzz + bench

### Fuzz

```
module=bi-map seed=42  cases=11778  ops=1178193  wall=90.0s  divergences=0
```

**1.18M operations, zero divergences**, on the tree with B-120 fixed. Reproduce with
`target/release/difffuzz --module bi-map --seed 42 --cases 11778`.

* **Op alphabet:** `set(k, v)` (weight 5) · `delete(k)` (3) · `get(k)` (3) · `has(k)` (2) ·
  `clear()` (1).
* **Keys and values share one six-item pool**, mixed strings and numbers, so `set` collides with an
  existing key, an existing value, or both far more often than a wide space would by chance — that
  collision handling is the entire point of the module. `clear`/`delete` are weighted in because
  B-120's reinsert-after-delete-and-clear interactions are easiest to reach right after one.
* **Observable state:** `size`, `items` (the real `Map`), and `inverse` — `{size, items: {$map:
  [...]}, inverse: {$self: true}}`, because `instance.inverse.inverse === instance` and the oracle's
  generic `encode()` special-cases exactly that circular reference. No oracle change was needed.
* **Deliberately excluded:** `instance.inverse.*` called directly (the oracle's `op` dispatch cannot
  reach a nested `instance.inverse.set(...)`, though every forward op still mutates and every
  observation still reads both sides, so the bijection invariant is fully checked); cursor lifecycle
  ops (`bi-map`'s cursor is `default-map`'s `OrderedMap` cursor, already fuzzed there); `forEach`
  (not yet in this alphabet — see `fuzz/log.txt`).

### The orphan regression seed, resolved

`crates/difffuzz/proptest-regressions/bi-map.txt` existed in the repository **before** the spec had
ever been compiled or run as part of `cargo test` — an earlier run's process was interrupted, leaving
the file uncommitted and its meaning unrecorded. Its first line:

```
cc 70b0df5065314c3bffe40687fe27d07184199cdf317c0a4fc17b89b1b0a1fb64 # shrinks to Program { ctor: [], ops: [Op { name: "set", args: [String("a"), String("a")] }, Op { name: "clear", args: [] }] }
```

Two things made it worth distrusting rather than trusting on sight: the same hash also appears in
this repo's `fixed-stack.txt`, `hashed-array-tree.txt` and `static-disjoint-set.txt` corpora, which is
explained by proptest's own mechanics — the persisted value is the master RNG seed at the case index
the failure occurred, and that chain advances identically regardless of which module or strategy is
running, so an unrelated module failing at the same case count under the same top-level `--seed 42`
produces byte-identical output. And the spec it names had no `#[test]` anywhere in
`tests/differential.rs` — nothing in the committed history had ever run it as a gate.

Replaying it directly against the current build (`difffuzz --module bi-map --seed 42 --cases 60`)
answered the question: the seed decodes to exactly the program in its own comment, and that program
diverges — B-120, round one, verbatim. **It was a real, previously uncaptured divergence, not
noise** — an earlier run (or an ad hoc run of the already-registered CLI binary) had found B-120
and never got the chance to report it. The corpus file now carries a provenance block explaining
both entries; the campaign above is clean against the fixed tree, including both persisted seeds
replayed first.

### Falsification of the port (gate 6)

**Named first:** `clear_desyncs_size_from_inverse_size_b_120`'s assertion
`assert_eq!(forward.inverse_size(), 1, "clear() must NOT resync inverse_size — B-120")`.

**The sabotage:** `BiMap::clear` given back the line round 1 above (before the fix) was missing —
`self.inverse_size = 0;` alongside `self.size = 0;` — reintroducing the exact "more correct than
upstream" defect fuzzing first caught.

**Confirmed red**, at exactly the named assertion: `left: 0, right: 1`. Reverted; **confirmed green
again**: all 11 `bi_map` unit tests pass, `cargo test --workspace` clean.

### Bench

`bench/results.json` → `modules["bi-map"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.
Protocol: 3 warmup + 10 measured, interleaved A/B/A/B, batches of K = 1000, 10,000 samples/side.

**`mixed-1e6`** — 1e6 mixed `set`/`get`/`delete` (50/25/25) over a shared 1e6-value domain for both
key and value (`K = u32`; drawing both from one domain makes `set`'s four-branch constraint
resolution — B-120's own subject — fire under load rather than only on the cheap "brand new pair"
path), xorshift32 seed 42:

| metric | port | upstream | |
|---|---|---|---|
| p50 ns/op | 118.1 | **102.9** | 1.15× slower |
| p99 ns/op | 322.3 | **288.2** | 1.12× slower |
| RSS delta MB | **60.1** | 212.8 | |
| structure-only RSS delta MB | **1.4** | 9.8 | |
| startup ms | **0.6** | 16.5 | 25× (reported separately; not throughput) |

**Another loss, on both p50 and p99** — stated plainly alongside `default-map`'s. **Unconfirmed
cause:** `BiMap::set` maintains two `OrderedMap`s in lockstep (`items` and `inverse`) and, on the
rebinding paths this workload's shared key/value domain deliberately exercises, does up to two
extra `delete` calls beyond the two `set`s every relation needs — a real, structural reason this
module could cost more per op than a single `Map`. That is a plausible account, not a confirmed
one: it has not been checked against a metric (e.g. counting how often each of `set`'s four branches
actually fires in this workload) that would let it be falsified rather than merely asserted.

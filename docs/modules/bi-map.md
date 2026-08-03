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

`crates/mnemonist-core/src/structures/bi_map.rs` — 12 tests, starting with a 1:1 port of all
twelve upstream blocks as a baseline. They close B-120 in both directions plus the
healing-on-next-mutation property (gap 1), a `set` that rebinds both sides of the bijection in one
call (gap 2), `delete` on a missing key (gap 3), reinsertion order (gap 4), the inverse view's full
method set exercised directly rather than only read through (gap 5), and iteration on an empty map
(gap 6). Also covered: each of the two-branch rebinding cases in isolation, and that `set` is a
true no-op — insertion order untouched — when the exact relation already exists. Full
test-to-gap mapping: `docs/modules/evidence/bi-map.md`.

Gaps 7 and 8 are stated rather than closed — see "Deliberate divergences".

## Bugs this found

### B-120 — `BiMap.prototype.clear` resets only ONE of its two size counters

Verified against Node 24.18.1; found by differential fuzzing on the first campaign run against this
module.

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
so a no-op `delete` right after a `clear()` leaves the stale counter stale for one more op.

Reproduced faithfully: `size()`/`inverse_size()` are real stored counters (not derived from
`OrderedMap::len()`, which cannot desync from anything), reset asymmetrically by `clear`, and
resynchronised by `delete`/`delete_reverse` only when something was actually removed — the second
of those two conditions took a second fuzzing round to get right; see the log for the round that
over-corrected and how it was caught. Both regression seeds are committed in
`crates/difffuzz/proptest-regressions/bi-map.txt`, one of which predates this module's own test
being wired into `cargo test` — the log records how that was confirmed to be a genuine capture and
not noise.

**Worth reading past the bug itself:** the module doc for `mnemonist_core::structures::bi_map` had
already analysed and *named* this exact defect in prose, including the reproduction above, before
the implementation caught up to it. A doc comment describing intended behaviour is a claim, not
evidence that the code behind it does that.

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

Keys and values share one six-item pool, mixed strings and numbers, so `set` collides with an
existing key, an existing value, or both far more often than a wide space would by chance — that
collision handling is the entire point of the module. Observable state is `size`, `items`, and
`inverse` (with the oracle's generic circular-reference handling covering `instance.inverse.inverse
=== instance` for free). Full grammar and exclusions: `docs/modules/evidence/bi-map.md`.

**Falsified (gate 6):** the sabotage removed `self.inverse_size = 0;` from `clear` — reintroducing
the exact "more correct than upstream" defect fuzzing first caught. Confirmed red at the named
assertion in `clear_desyncs_size_from_inverse_size_b_120` (`left: 0, right: 1`); reverted and
confirmed green again across all 11 `bi_map` unit tests and a clean `cargo test --workspace`.

### Bench

`bench/results.json` → `modules["bi-map"]`. Methodology: `bench/methodology.md`.
Host: AMD Ryzen 5 7600X, 12 threads, WSL2, Node 24.18.1, rustc 1.97.1, quiet serial pass.

**`mixed-1e6`** — 1e6 mixed `set`/`get`/`delete` (50/25/25) over a shared 1e6-value domain for both
key and value (drawing both from one domain makes `set`'s four-branch constraint resolution —
B-120's own subject — fire under load rather than only on the cheap "brand new pair" path):
the port is slower on both p50 and p99, and uses far less memory (60.1 MB RSS delta versus 212.8
MB). **This is the least trustworthy p50 figure in the port**: across six runs it has ranged from
1.14× to 1.59× slower, where every other module's ratio reproduces to about 1% — read it as
"slower, by somewhere between a little and a half" rather than a single number. p99 is comparatively
stable at 1.12× slower. Full table and run history: evidence file.

A doubled-hashing shape — `link` reading `primary.get(&key)` and then unconditionally writing
`primary.set(key, value)`, for both `primary` and `secondary` — was found and fixed by updating an
existing slot in place through `OrderedMap::get_mut` instead. It bought nothing measurable: six
alternating runs put the port at 169.9 ns before and 164.6 ns after, a 3% gap inside a 10%
run-to-run spread. The change is kept because one lookup is not worse than two, not because a
speedup is claimed.

The base regression's cause is unconfirmed: `BiMap::set` maintains two `OrderedMap`s in lockstep
and, on the rebinding paths this workload's shared key/value domain deliberately exercises, does up
to two extra `delete` calls beyond the two `set`s every relation needs. Plausible, not confirmed —
it has not been checked against a metric (e.g. counting how often each of `set`'s four branches
actually fires in this workload) that would let it be falsified rather than merely asserted.

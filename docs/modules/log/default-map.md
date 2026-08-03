# default-map — working log

Chronological. See `docs/modules/default-map.md` for the current-state document and
`docs/modules/evidence/default-map.md` for the gate artifacts.

## Fuzz oracle: number encoding fault (found during initial campaign, this series)

Not an upstream bug; recorded because it would have produced a false divergence report. JavaScript
has one number type and JSON has one number syntax, so `1` serialises as `1` and never as `1.0`.
`serde_json` *does* distinguish the two and compares them unequal, so the Rust side's `json!(1.0)`
disagreed with the oracle on every integral key. Caught on the very first run of the grammar, before
any real result was recorded. Fixed by `number_json`, which encodes a double the way
`JSON.stringify` does. The seed is committed.

## Falsification-method defect: compaction test walked the wrong direction (found while writing core tests, this series)

The first version of `a_compaction_under_a_live_cursor_does_not_disturb_the_walk` deleted the
entries **ahead** of the cursor. That does force a compaction — but it removes only slots the
cursor had not yet reached, so the cursor's physical index stays *accidentally* correct and the
test passed against a `locate` that was deliberately broken to return its unvalidated hint.
Rewritten to delete the entries **behind** the cursor, where compaction shifts every remaining
entry left, and then confirmed red against the same sabotage. Rewriting it also exposed a real
out-of-range index panic: the hint validation read `map.slots[hint - 1]` before checking
`hint <= slots.len()`, so a hint left past the end of a shrunken vector panicked instead of being
rejected. This is the same lesson gate 6 itself carries, one level down: a falsification test that
cannot fail is just a second green light, and that applies to the tests as much as to the gate.

## PORTBUG-1 — `&self` on a `Freeze` type was `noalias readonly` (fixed 2026-08-01)

This bridge held a bare core value, so `&self` compiled to a `noalias readonly` pointer and LLVM
was entitled to hoist reads across the JS callback — which it did. It now holds `RefCell<Core>`,
which is not `Freeze`, and every `&mut self` method became `&self` + `borrow_mut()`. The borrow is
taken per step and released before the callback runs, so a re-entrant callback never meets an
outstanding borrow. See `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`.

## Divergence withdrawn: re-entrant factory/forEach callback (WITHDRAWN 2026-08-01)

The divergences table previously carried this row:

> ~~**A re-entrant factory or `forEach` callback is not supported.**~~ **WITHDRAWN 2026-08-01 —
> both are now supported.** This row said fixing it "means interior mutability throughout and that
> is a decision for the whole bridge, not for one module". That decision was then forced by PORTBUG-1,
> which turned out to be the same exposure miscompiling rather than merely aliasing. The bridge now
> holds `RefCell<Core>`, `forEach` re-borrows per step, and `get` runs the factory between its read
> and its write exactly as upstream does — so a callback or factory that calls back into the same
> map behaves as upstream's. Verified differentially in `tests/boundary/reentrancy.js`.

Removed from the current document's divergences table because it is no longer a divergence; the
capability is now supported and documented as such.

## `$forEach` — the op that was missing (added 2026-08-01, PORTBUG-1)

`default-map`'s grammar had no `forEach` op at all. That omission is what let PORTBUG-1 — a `forEach`
callback mutating the collection it is walking — through 4.37 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it. `$forEach(method, rule, limit)` was
added to close that hole; see the current document for what it covers now.

## Bench cause investigation: double hash lookup hypothesis (refuted 2026-08-02)

This doc's own prior note speculated about two candidate mechanisms for the `mixed-1e6` regression
before any number existed:

* **`Rc<str>` keys versus V8 strings** — does not apply to this benchmark. That concern is about
  the bridge's string-keyed instantiation; this benchmark links `mnemonist-core` directly with bare
  `u32` keys (per `methodology.md`, never through N-API), so no `Rc<str>` allocation exists on this
  path at all. Ruled out by construction, not by measurement.
* **`get_or_insert_with` on a hit does two hash lookups, not one** — `slot_of` then `entry_at`
  (`DefaultMap::try_get_or_insert_with`), because the borrow from the first has to end before a
  factory that might re-enter can run. This was a plausible-sounding explanation for a chunk of the
  p50 gap, and it is **refuted, 2026-08-02** — by reading `OrderedMap::entry_at`
  (`crates/mnemonist-core/src/map/mod.rs`) and then by measurement. `entry_at(slot)` is
  `self.slots.get(slot)` — a plain `Vec` index, not a second `HashMap::get`. There is only ever one
  hash lookup on the hit path (`slot_of`), the same one lookup `peek` does.
  `bench/runner/src/default_map.rs::run_probe_peek`/`run_probe_hit` (reachable via
  `bench-runner --default-map-probe`) time the two directly, over the same prefilled 1,000,000-key
  map, same keys, no factory ever invoked:

  | variant | p50 ns/call |
  |---|---|
  | `peek` (one hash lookup, `OrderedMap::get`) | 118.224 |
  | `get_or_insert_with` hit path (`slot_of` + `entry_at`) | 117.432 |

  The two are equal within run-to-run noise (0.7% apart) — exactly what "no second hash lookup
  exists" predicts, and the opposite of what "two hash lookups" predicts. **Verdict: refuted.**

  Both numbers are far higher than a single `u32` hash computation should cost, which points at the
  actual mechanism: at a 1,000,000-key domain, `OrderedMap`'s internal `HashMap<K, usize>` no longer
  fits comfortably in cache, and a uniformly-random key (as this workload draws) makes close to
  every lookup a real DRAM access rather than an L2/L3 hit — consistent with, though not proven to
  single-handedly cause (this was not isolated further), the earlier observation that `mixed-4e6`'s
  regression ratio (1.17×) is *smaller* than `mixed-1e6`'s (1.42×): a bigger domain should make a
  structural per-op cost proportionally *more* visible, not less, but it is exactly what a shared,
  domain-size-driven memory-latency floor on both sides would produce, since it shrinks the
  *relative* size of whatever Rust-specific constant sits underneath it. Recorded as a lead for a
  follow-up investigation, not as a second confirmed finding — the probe above establishes the "no
  double hash lookup" refutation with a direct measurement; the cache-miss account of the *actual*
  cost is consistent with the data but was not isolated with its own falsifying metric (e.g. a
  domain-size sweep instrumented with hardware cache-miss counters, which this host's tooling could
  not provide).

## Structural fix costed and declined (2026-08-03)

The structural fix the cache-miss account implies — values inline in the hash map, insertion order
tracked separately — was scoped rather than dismissed. `OrderedMap` is used directly by seven units
(`default-map`, `bi-map`, `fuzzy-map`, `multi-set`, `set`, `multi-map`, `inverted-index`) and
transitively by an eighth (`fuzzy-multi-map`); `bk-tree` and `default-weak-map` only *mention* it in
doc comments and are unaffected. All eight are complete through all ten gates, so each would need
its gate-6 falsification redone against the new internals, its gate-9 campaign re-run, and its
gate-10 benchmark re-measured on an idle machine. Estimate: 3.5–6 h implementation, 2.5–4.5 h
re-verification, 0.5–1 h documentation.

Declined on two grounds, neither of them the estimate alone. First, `MapCursor`'s discipline — a
frozen `next_id` resolved against live slots each step, surviving a compaction that renumbers
physical indices — is a genuinely subtle invariant, and this repository's own history shows that
arena-and-id structures walked under mutation (`critbit-tree-map`, `fixed-critbit-tree-map`)
produce bugs found late rather than early. Note also that `MapCursor`'s discipline is *not* the
same as `crate::cursor`'s frozen-length/live-element hybrid capture; a replacement must reproduce
this one specifically.

Second, and independent of time: this benchmark's op mix is 50% `set`, 25% `delete`, 25%
`getOrInsertWith`. Inlining values clearly helps only the read-shaped quarter — `set` and `delete`
must still touch a second structure to maintain insertion order and tombstones under any design
that keeps the cursor guarantees. So a successful rewrite should not be expected to close the full
gap, and the honest prior on it closing *most* of it is well under even.

Before anyone attempts this, run the domain-size sweep with cache-miss counters named above. It is
the falsifying measurement this account never got, it costs hours rather than days, and it would
establish whether the two-structure layout is the bottleneck before a day is spent replacing it.

No fix applicable at this time: there is nothing to fix in the "two lookups" sense — the
hypothesised second lookup does not exist, and `try_get_or_insert_with`'s hit path is already a
single lookup plus an O(1) index.

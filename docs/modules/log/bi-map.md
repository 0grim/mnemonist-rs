# bi-map — working log

Chronological. See `docs/modules/bi-map.md` for the current-state document and
`docs/modules/evidence/bi-map.md` for the gate artifacts.

## B-120 found and fixed in two rounds

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
seeds are committed with provenance in `crates/difffuzz/proptest-regressions/bi-map.txt`.

## The orphan regression seed, resolved

`crates/difffuzz/proptest-regressions/bi-map.txt` existed in the repository **before** the spec had
ever been compiled or run as part of `cargo test` — an earlier run's process was interrupted, leaving
the file uncommitted and its meaning unrecorded. Its first line:

```
cc 70b0df5065314c3bffe40687fe27d07184199cdf317c0a4fc17b89b1b0a1fb64 # shrinks to Program { ctor: [], ops: [Op { name: "set", args: [String("a"), String("a")] }, Op { name: "clear", args: [] }] }
```

Two things made it worth distrusting rather than trusting on sight: the same hash also appears in
this repo's `fixed-stack.txt`, `hashed-array-tree.txt` and `static-disjoint-set.txt` corpora, which
is explained by proptest's own mechanics — the persisted value is the master RNG seed at the case
index the failure occurred, and that chain advances identically regardless of which module or
strategy is running, so an unrelated module failing at the same case count under the same top-level
`--seed 42` produces byte-identical output. And the spec it names had no `#[test]` anywhere in
`tests/differential.rs` — nothing in the committed history had ever run it as a gate.

Replaying it directly against the current build (`difffuzz --module bi-map --seed 42 --cases 60`)
answered the question: the seed decodes to exactly the program in its own comment, and that program
diverges — B-120, round one, verbatim. It was a real, previously uncaptured divergence, not
noise — an earlier run (or an ad hoc run of the already-registered CLI binary) had found B-120
and never got the chance to report it. The corpus file now carries a provenance block explaining
both entries; the campaign in the current document is clean against the fixed tree, including both
persisted seeds replayed first.

## Bench re-measurement (2026-08-03)

The original `mixed-1e6` measurement put p50 at 118.1 ns port vs 102.9 ns upstream, 1.15× slower.
Re-measured 2026-08-03: 1.51× slower — this benchmark turned out to be the least trustworthy in the
port. A whole-suite pass on an idle machine, and a spot-check in isolation afterwards, both put this
module well below the 1.15× first recorded.

The same doubled-hashing shape `multi-set` had was found here too — `link` reads `primary.get(&key)`
and then unconditionally writes `primary.set(key, value)`, and does it again for `secondary`, so
`bi-map` pays it twice per call. It was rewritten to update an existing slot in place through
`OrderedMap::get_mut`, skipping the closing `set` for whichever side already held the key.

The rewrite needed care in one place. It writes `primary` early and then, in the second block, calls
`primary.delete(&current_key)`. Were `current_key` ever equal to `key`, that would delete the entry
just written, where the original's trailing `set` would have re-inserted it. It cannot: the block
returns early when the slot already holds `key`, so reaching the delete proves `current_key != key`.
`set_can_rebind_both_sides_of_the_bijection_in_one_call` is exactly that case.

And it bought nothing measurable. Six runs alternating the old and new code under identical
conditions put the port at 169.9 ns before and 164.6 ns after — a 3% gap inside a 10% run-to-run
spread. The change was kept because one lookup is not worse than two, but no speedup is claimed for
it, and the mechanism is stated in the source without a magnitude.

Those same six runs are why the p50 figure carries a caveat the rest of the port's table does not:
this workload's ratio spanned 1.14× to 1.59× across them, where every other module reproduces to
about 1%. Read it as "slower, by somewhere between a little and a half".

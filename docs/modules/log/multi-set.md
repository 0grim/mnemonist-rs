# multi-set — working log

Chronological. See `docs/modules/multi-set.md` for the current-state document and
`docs/modules/evidence/multi-set.md` for the gate artifacts.

## Fuzz spec harness bugs: `add`/`remove`/`edit` echoed the wrong return value (found and fixed this series)

Two harness bugs were found and fixed while getting the `multi-set` fuzz campaign clean. Neither is
a port or upstream defect; both are in the fuzz spec's own comparison logic
(`crates/difffuzz/src/modules/multi_set.rs`). The spec's `apply()` for `add`/`remove` originally
always echoed back `{"$self": true}`/`{undefined}` regardless of the sign-flip delegation D-164
describes, and `edit`'s `apply()` always echoed `{"$self": true}` regardless of whether `a` was
present. Both were caught by the very first case the campaign generated once the harness ran at all.

## Bench: `mixed-1e6` p50 regression traced to a redundant hash lookup in `add`, fixed 2026-08-03

An earlier whole-suite pass on an idle machine put this module at **1.29× slower** at p50 — the
figure this document had previously recorded (1.2× faster) had been measured in a different
session, where the JavaScript baseline sits up to 20% away from where it sits here. Being in the
loss column is what got the module read line by line.

`add` did `items.get(&item)` and then, unconditionally, `items.set(item, ...)` — two hash lookups of
the same key on every call, on the operation that is half this workload's mix. Upstream has no
choice: a JS `Map` cannot look up and hand back a handle to update in place. Fixed by reaching
`OrderedMap::get_mut` instead, which can: `set` on an existing key is already an in-place
`mem::replace` into the same slot, so bumping the multiplicity through the `&mut f64` preserves
insertion order identically. `remove`'s "still positive afterwards" path took the same fix; its
"drops to zero" path is unchanged, since deleting is not something `get_mut` can do.

Measured over four runs: the port's own p50 moved from 24.80 ns (pre-fix, matching the "1.29×
slower" measurement) to 16.13–16.37 ns (post-fix), a 0.7% spread on the port side — **1.36× faster
than upstream**, back out of the loss column. The current document states this post-fix figure
directly.

An even earlier figure, from before either the "1.2× faster" or "1.29× slower" measurements above,
had recorded this workload's port p50 as 19.0 ns/op against upstream's 22.3 (also "1.2× faster" by
coincidence of rounding). That number predates the whole-suite idle-machine re-run that found the
regression and is superseded by the measurements in this entry.

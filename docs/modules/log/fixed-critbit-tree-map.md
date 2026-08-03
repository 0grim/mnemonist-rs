# fixed-critbit-tree-map — working log

Chronological. See `docs/modules/fixed-critbit-tree-map.md` for the current-state document and
`docs/modules/evidence/fixed-critbit-tree-map.md` for the gate artifacts.

## Bench: two per-`set` allocations traced and fixed, 2026-08-03

`set` declared `let mut ancestors: Vec<usize> = Vec::new()` and `let mut path: Vec<bool> =
Vec::new()` fresh on every call and dropped both at the end. Any walk that descends through even one
internal node pushes into them, so once the tree is non-trivial that is two heap allocations on
essentially every insert — and `set` is half this workload's operation mix.

Both are now struct fields, cleared on entry rather than reallocated. Neither is observable outside
a single `set` call: cleared on entry, filled to that call's own traversal depth, read only within
the same call. The struct derives `Debug` and `Clone` but not `PartialEq`, and nothing formats it,
so carrying a stale scratch buffer into a clone changes nothing.

Six runs alternating the old and new code put the port's p50 at **393–424 ns before and 280–307 ns
after**, about 28%. At the time, this took the module from reading 1.11× slower than upstream to
1.18× faster.

This fix predates the residual p50 loss (~1.06–1.08×, attributed tentatively to `BoundedSlots`'
bounds-check overhead) now stated as the current finding in the document — that residual was
measured on the build that already included this allocation fix, and is a separate, smaller,
unconfirmed cost on top of it.

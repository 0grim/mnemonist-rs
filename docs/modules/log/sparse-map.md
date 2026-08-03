# sparse-map — working log

Chronological. See `docs/modules/sparse-map.md` for the current-state document and
`docs/modules/evidence/sparse-map.md` for the gate artifacts.

## PORTBUG-1 — `&self` on a `Freeze` type was `noalias readonly` (fixed 2026-08-01)

This bridge held a bare core value, so `&self` compiled to a `noalias readonly` pointer and LLVM was
entitled to hoist reads across the JS callback — which it did. It now holds `RefCell<Core>`, which
is not `Freeze`, and every `&mut self` method became `&self` + `borrow_mut()`. The borrow is taken
per step and released before the callback runs, so a re-entrant callback never meets an outstanding
borrow. See `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`.

## `$forEach` — the op that was missing (added 2026-08-01, PORTBUG-1)

`sparse-map`'s grammar had no `forEach` op at all. That omission is what let PORTBUG-1 — a `forEach`
callback mutating the collection it is walking — through 2.65 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it. `$forEach(method, rule, limit)` was
added to close that hole; see the current document for what it covers now.

## `new()` validation ordering noticed via an allocation abort (found this series)

Upstream throws for `length > 2^32`, and reaches that throw inside `getPointerArray` *before*
`new Values(length)` runs. Allocating first and validating after was the port's original shape; it
turned an `Err` into a 34 GB allocation abort, which is how the ordering got noticed. Fixed by
validating before allocating, matching upstream's order; now stated as a deliberate divergence
rationale in the document rather than as a defect, since the current behaviour matches upstream.

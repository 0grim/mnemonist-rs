# queue — working log

Chronological. See `docs/modules/queue.md` for the current-state document and
`docs/modules/evidence/queue.md` for the gate artifacts.

## `$forEach` — the op that was missing (added 2026-08-01, B-31)

`queue`'s grammar had no `forEach` op at all. That omission is what let B-31 — a `forEach` callback
mutating the collection it is walking — through 4.13 M clean operations: an op alphabet that omits a
method omits every bug reachable only through it. `$forEach(method, rule, limit)` was added to close
that hole; see the current document for what it covers now.

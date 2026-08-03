# stack — working log

Chronological. See `docs/modules/stack.md` for the current-state document and
`docs/modules/evidence/stack.md` for the gate artifacts.

## Claim withdrawn: `Stack.of` routing through `Stack.from(arguments)` does not exercise the `[object Arguments]` clause (WITHDRAWN)

An earlier comment said routing `Stack.of` through `Stack.from(arguments)` makes the original suite
exercise branch 1's `[object Arguments]` clause. This was tested directly: deleting the clause
leaves all 22 assertions green. A modern `arguments` object carries `Symbol.iterator` and falls
through to branches 3/4 with the same numeric second argument, so the clause is observable only for
something claiming the tag *without* being iterable — which nothing in the original suite does.
Corrected in both source comments that made the claim, rather than quietly dropped. This is also the
falsification recorded in the document as "a third attempt that stayed green": a falsification that
cannot fail is just a second green light, and this one was informative precisely because it failed
to fail.

## `$forEach` — the op that was missing (added 2026-08-01, PORTBUG-1)

`stack`'s grammar had no `forEach` op at all. That omission is what let PORTBUG-1 — a `forEach` callback
mutating the collection it is walking — through 4.40 M clean operations: an op alphabet that omits a
method omits every bug reachable only through it. `$forEach(method, rule, limit)` was added to close
that hole; see the current document for what it covers now.

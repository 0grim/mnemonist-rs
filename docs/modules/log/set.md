# set — working log

Chronological. See `docs/modules/set.md` for the current-state document and
`docs/modules/evidence/set.md` for the gate artifacts.

## Claim withdrawn: `disjunct` writing order does not affect the result (WITHDRAWN)

An earlier draft of this port's documentation said `disjunct` adds `B \ A` **before** deleting
`A ∩ B` "so `{1,2}` disjunct `{2,3}` is `[1, 3]` and not `[3, 1]`". **That is false**, and it was
caught by sabotaging exactly that — reordering only the *writes* to delete first, while still
testing `!A.has(member)` against the original `A` — and watching nothing go red.

Reordering only the writes leaves both the result and its order unchanged, because a member of
`B \ A` is appended at the end either way and a shared member is gone either way. `test/set.js`
stayed at 16 passing and `tests/boundary/set.js` stayed fully green.

What *is* load-bearing, established in the same investigation, is that the `!A.has` **test** runs
before any deletion — not the write order. Delete first and every member of `A ∩ B` passes the
test, is re-added, and the answer becomes `A ∪ B`. That sabotage turns `test/set.js`'s `#.disjunct`
block red, and it is now pinned separately by
`set.rs::disjunct_decides_what_to_add_before_it_deletes_anything` and by the corrected boundary
spec. The trace is still emitted add-then-delete in the current document, because that is the
sequence of calls upstream makes — faithfulness with no test able to see it, labelled as such rather
than justified with a benefit it does not have.

# bit-vector — working log

Chronological. See `docs/modules/bit-vector.md` for the current-state document and
`docs/modules/evidence/bit-vector.md` for the gate artifacts.

## B-31 — `&self` on a `Freeze` type was `noalias readonly` (fixed 2026-08-01)

This bridge held a bare core value, so `&self` compiled to a `noalias readonly` pointer and LLVM was
entitled to hoist reads across the JS callback — which it did. It now holds `RefCell<Core>`, which
is not `Freeze`, and every `&mut self` method became `&self` + `borrow_mut()`. The borrow is taken
per step and released before the callback runs, so a re-entrant callback never meets an outstanding
borrow. See `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`. The one place this fix could not be
applied in its usual form (the growth policy, which is JavaScript called from inside `grow`) is
described as a current divergence in the document rather than here, since it is still the shape of
the bridge today.

## `$forEach` — the op that was missing (added 2026-08-01, B-31)

`bit-vector`'s grammar had no `forEach` op at all. That omission is what let B-31 — a `forEach`
callback mutating the collection it is walking — through 3.23 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it. `$forEach(method, rule, limit)` was
added to close that hole; see the current document for what it covers now.

## Fuzz campaigns accidentally launched twice, concurrently (found this series)

Both `bit-vector` fuzz campaigns recorded in the document were, at one point, accidentally launched
a second time while the first was still running. The duplicate runs are commented out in
`fuzz/log.txt` rather than deleted, and rather than summed into the reported totals: a pair sharing a
seed largely generates the same programs, so summing would double-count rather than add signal. The
withdrawn numbers are kept visible in `fuzz/log.txt` because the pairs are an accidental measurement
of the contention effect that is the reason gate 10 (benchmarks) is deferred to a quiet serial pass:
same seed, same wall budget, same machine, overlapping in time gave 21,852 vs 20,657 cases on one
pair and 9,617 vs 10,603 on the other. A wall-clock-bounded campaign is not reproducible in case
count; only `--cases` is — which is why the document's own reproduction command uses `--cases`, not
a duration.

## Bench regression traced to `ToInt32`, fixed alongside `bit-set` (2026-08-03)

This module shares `split` (`crates/mnemonist-core/src/structures/bits.rs`) with `bit-set`, whose
`ToInt32` fast-path fix is described in full in `docs/modules/log/bit-set.md`. The `mixed-1e6` p50
here moved from a tie (8.20 ns/op against upstream's 8.3) to 6.42 ns/op — about 1.30× faster — with
no `bit-vector`-specific change required, since the fix lives entirely in the shared `split`
function.

This also answered a question this document had previously recorded as open: whether the shared
store's `Rc<RefCell<Vec<u32>>>` borrow — paid on every `set`/`reset`/`flip`, which upstream does not
pay — was a non-negligible cost on an operation that is otherwise a load, an OR and a store. The
borrow is still there and the module is now faster than upstream, so whatever the borrow costs, it
was not what stood between this port and a win; the index conversion was. The `RefCell` buys exact
reproduction of `clear`/`reallocate` detaching an open cursor and is still paying for itself.

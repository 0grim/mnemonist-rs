# bit-set — working log

Chronological. See `docs/modules/bit-set.md` for the current-state document and
`docs/modules/evidence/bit-set.md` for the gate artifacts.

## PORTBUG-1 — `&self` on a `Freeze` type was `noalias readonly` (fixed 2026-08-01)

This bridge held a bare core value, so `&self` compiled to a `noalias readonly` pointer and LLVM was
entitled to hoist reads across the JS callback — which it did. It now holds `RefCell<Core>`, which
is not `Freeze`, and every `&mut self` method became `&self` + `borrow_mut()`. The borrow is taken
per step and released before the callback runs, so a re-entrant callback never meets an outstanding
borrow. See `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`.

## `$forEach` — the op that was missing (added 2026-08-01, PORTBUG-1)

`bit-set`'s grammar had no `forEach` op at all. That omission is what let PORTBUG-1 — a `forEach`
callback mutating the collection it is walking — through 3.92 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it. `$forEach(method, rule, limit)` was
added to close that hole; see the current document for what it covers now.

## Bench regression traced to `ToInt32` on every hot-path index, fixed 2026-08-03

**This is the module the port was predicted to win largest on, and on the first measurement it did
not** — both p50 and p99 were ~10–12% slower than V8's own typed-array path over three independent
metrics (p50, p99, min), which ruled out a single unlucky batch. The original explanation was
unconfirmed: `BitSet::set`/`reset`/`get`/`test` each go through `Words::set_bit`/`get_bit`
(`crates/mnemonist-core/src/structures/bits.rs`), an extra call frame LLVM might not inline as
aggressively as V8 inlines a monomorphic `Uint32Array` element access at this op's simplicity.

**Confirmed 2026-08-02, and refined.** `bench/runner/src/bit_set.rs`, reachable via
`bench-runner --bit-set-probe`, ran three variants of the identical op stream: the real `BitSet`
(`Rc<RefCell<Vec<u32>>>` behind `Words`, `i64` indices through a real `ToInt32`), a bare `Vec<u32>`
with plain `usize` indices and no `RefCell` at all, and a third variant between the two — the same
bare `Vec<u32>`, still no `RefCell`, but indices still pushed through the exact `f64`-based
`to_int32`/`rem_euclid` conversion `Words::split` used. Results (see evidence file for the table):
the isolated gap split roughly 73%/27% between the `RefCell`/`Words` wrapper layer and the
`to_int32` conversion alone — the wrapper was the larger piece, consistent with what was named, but
`to_int32`'s call to `f64::rem_euclid` (real floating-point division, not a missed-inlining
artefact) was a second, previously undocumented contributor nearly a third the size of the first.
Both bare variants beat upstream's own published p50 (7.935 ns) outright — the overhead was larger
than the entire measured regression.

**Fixed 2026-08-03 — the index conversion was the whole margin of the disclosed regression.**
`split` in `crates/mnemonist-core/src/structures/bits.rs`, reached by every one of `set`, `reset`,
`flip` and `get`, converted its `i64` index to `f64` and ran JavaScript's full `ToInt32`: `trunc`,
then `rem_euclid(2^32)`, then a sign fixup. That path exists for indices outside `i32`'s range —
exactly what upstream's out-of-range reads reach, and those are reproduced bug-for-bug — but
`ToInt32` is the *identity* for any value already inside that range: `trunc` is a no-op on an
integral value, and `rem_euclid` of a value already in `[-2^31, 2^31)` returns it unchanged once the
sign fixup is applied.

`split` now tries `i32::try_from` first and falls back to the float path only when the index really
does not fit. The equivalence was checked over 2.6 million values including every boundary
(`i32::MIN`, `i32::MAX`, ±2^31, ±2^32) rather than argued from the definition alone.

Six runs alternating the old and new code: the port's p50 was 8.66–8.70 ns before and 5.86–5.93 ns
after, with upstream steady at 7.85–7.91 ns throughout — an unambiguous ~32% port-side improvement.
This module now reads about 1.31× faster than upstream where it read about 1.10× slower.
`bit-vector` shares `split` and moved with it, from a tie to about 1.30× faster.

The `mixed-1e6` table in the evidence file predates this fix; `bench/results.json` is the current
source of truth and reflects the post-fix state where it has been re-run.

## Structural fix (removing the remaining RefCell/to_int32 overhead) not attempted

A `usize` fast path guarded by "index is non-negative and small" is conceivable but has not been
attempted: both remaining layers are load-bearing rather than incidental — the
`Rc<RefCell<Vec<u32>>>` is `Words`'s own re-entrancy story, shared with `BitVector` (whose `length`
is mutable and whose cursor must keep reading a `clear()`'d array), and the `f64`-based `to_int32`
is what makes a negative index drop cleanly rather than wrap through `usize` the way a naive Rust
port would. Such a change would be a `crates/mnemonist-core` behaviour-preserving optimisation, not a
local tweak, and would need bit-set's fuzz campaign and bench figures re-run before it could stand.

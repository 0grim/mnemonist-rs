# sparse-set — working log

Chronological. See `docs/modules/sparse-set.md` for the current-state document and
`docs/modules/evidence/sparse-set.md` for the gate artifacts.

## Harness defect: proptest `TestRunner` reuse under-ran every batch after the first (found while porting this module, fixed 3120085)

Not an upstream bug; recorded here because it invalidated evidence this project had already
published for a different module. proptest's `TestRunner` counts successes for its whole lifetime
and loops `while successes < config.cases`, so the campaign driver's reuse of one runner across
batches meant **every batch after the first executed no new cases at all** — only the persisted
regression corpus, which proptest replays before the (now empty) main loop, and then spun at 100%
CPU until the deadline. The recorded `static-disjoint-set` campaigns of "16,666 cases" were 32
genuinely new programs plus two saved seeds re-run ~8,300 times each. Measured decisively: with the
corpus file removed, a 120-second campaign dropped from 16,666 cases to 32.

It surfaced here only because `sparse-set` had no corpus yet, so instead of quietly repeating two
programs the driver spun visibly and `--duration 20` reported 32 cases in 20.0 seconds. Both
`static-disjoint-set` campaigns have been re-run and re-logged; the superseded lines are kept in
`fuzz/log.txt` under a correction block rather than deleted. Pinned by
`every_batch_generates_new_cases`, which runs with **no** corpus so that the only way past `batch`
cases is a batch that really generated.

## PORTBUG-1 — `&self` on a `Freeze` type was `noalias readonly` (fixed 2026-08-01)

This bridge held a bare core value, so `&self` compiled to a `noalias readonly` pointer and LLVM was
entitled to hoist reads across the JS callback — which it did. It now holds `RefCell<Core>`, which
is not `Freeze`, and every `&mut self` method became `&self` + `borrow_mut()`. The borrow is taken
per step and released before the callback runs, so a re-entrant callback never meets an outstanding
borrow. See `crates/mnemonist-napi/src/cursor.rs`'s `CellCursor`.

## `$forEach` — the op that was missing (added 2026-08-01, PORTBUG-1)

`sparse-set`'s grammar had no `forEach` op at all. That omission is what let PORTBUG-1 — a `forEach`
callback mutating the collection it is walking — through 2.94 M clean operations: an op alphabet
that omits a method omits every bug reachable only through it. `$forEach(method, rule, limit)` was
added to close that hole; see the current document for what it covers now.

## Bench cause investigation: `mixed-4e6` p99 discrepancy is noise, not a finding (2026-08-02)

`bench/results.json`'s `mixed-4e6` p99 entry (330.395 port vs 328.803 upstream, ratio 1.00) did not
match the table published alongside it (an earlier run). Investigated 2026-08-02: this is noise, not
a finding, and was left in the generated array rather than hand-edited out. 330.395 vs 328.803 is a
0.48% difference — three orders of magnitude below the ~32% p99 swing this same host's own
methodology document records between otherwise-clean runs of upstream alone. `bench/drive.js`'s
regression check has no noise floor: any port figure that exceeds upstream's by even a fraction of a
nanosecond is mechanically listed, which is the right default (hiding a regression scores worse than
disclosing one) but means a ratio of 1.00 can appear for no reason other than which side's
measurement landed a few nanoseconds higher on a given pass. Given the instruction to over-report
rather than under-report, the array entry stays exactly as `bench/drive.js` computed it — editing
generated JSON by hand to remove an inconvenient entry would be a worse failure mode than an
over-inclusive one. Every other regression this module and the other three investigated
(`bit-set`, `default-map`, `heap`) carry is well outside this band and should be read as real.

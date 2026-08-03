# static-interval-tree — working log

Chronological. See `docs/modules/static-interval-tree.md` for the current-state document and
`docs/modules/evidence/static-interval-tree.md` for the gate artifacts.

## Fuzz oracle harness bug: float round trip, shared root cause with `vector` (found this series, fixed)

Query results are `Vec<(f64, f64)>` pairs built from generated `i32` bounds carried as `f64`.
`serde_json`'s default float parser is not always correctly rounded for the oracle's JSON
responses; the same `float_roundtrip` fix that `vector` needed applies here too, since this
module's query results round-trip through the identical wire protocol. Confirmed fixed by the same
scratch test recorded in `docs/modules/log/vector.md` and by both campaigns in the current document
running clean.

## Bench: `LENGTH` parameter for `mixed-1e5` tuned down from 10% to 0.1% of the domain

The first attempt at the `mixed-1e5` workload used intervals with `LENGTH` at 10% of the domain.
Average matches per query ran into the thousands, and collecting (cloning, position-weighting)
that many hits per call made a 200,000-op pass take **22 seconds** — the same shape as
`bit_set.rs`'s `rank` trap, where a benchmark accidentally measures collection cost rather than the
structure's own per-op cost. At `LENGTH` 0.1% of the domain, `intervalsContainingPoint` averages
~101 matches per call and `intervalsOverlappingInterval` averages ~202 — both a real, meaningful
fraction of the 100,000-interval tree pruned around, not 0 and not "the whole set" — and the
benchmark runs in a normal timeframe. The current document states the 0.1% figure as the workload
definition.

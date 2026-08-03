# vp-tree — working log

Chronological. See `docs/modules/vp-tree.md` for the current-state document and
`docs/modules/evidence/vp-tree.md` for the gate artifacts.

## Bench: sequential-domain construction found to be a fixed-pivot-quicksort worst case, `size` reduced from 300,000 to 50,000

Construction sorts by distance from a vantage point using upstream's own fixed-pivot quicksort, and
sequential input is that algorithm's classic worst case. A 300,000-item sequential build measured
over 45 seconds of CPU time before this was caught, which is what led to shuffling the domain for
the `mixed-5e4` workload rather than using `0..size` in order.

Even after shuffling, construction stays measurably superlinear — verified against a standalone
probe of `bench/upstream/vp-tree.js` itself, which took a comparable ~2 seconds building 80,000
shuffled items, confirming this is a genuine property of the ported algorithm over a
one-dimensional metric, not a Rust-only regression. `size` was reduced from the initial 300,000 to
50,000 for exactly this reason — large enough to be a real workload, small enough that construction
time does not dominate the benchmark. The current document states the shuffled-domain methodology
and the 50,000-item size as the current workload definition.

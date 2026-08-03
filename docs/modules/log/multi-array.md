# multi-array — working log

Chronological. See `docs/modules/multi-array.md` for the current-state document and
`docs/modules/evidence/multi-array.md` for the gate artifacts.

## Bench: `get`'s p50 regression investigated across three sessions

**A split result observed first, p50 loses (1.9×), p99 wins.** `MultiArray::get` walks the pointer
chain and materialises a `Vec<f64>` on every call, one bounds-checked array write per step; upstream
does the same walk over a plain `Array`, whose access V8 can speculate on more aggressively once the
shape is monomorphic. That was a plausible account of where the p50 gap came from, not originally a
confirmed one.

**Confirmed 2026-08-02 — partially, with a real number attached rather than left as "plausible".**
`bench/runner/examples/multi_array_alloc_probe.rs` builds a 20,000-index array with 25 values/bucket
(this workload's own steady-state ratio) and, with an allocation-counting global allocator, isolates
`get` from `multiplicity` over 250,000 calls of each. Results (see evidence file for the table):
`get` does exactly one allocation per call; a bare `Vec::with_capacity(25)` plus fill costs 34.88 ns
of `get`'s 50.96 ns.

**Re-measured 2026-08-03, after narrowing the bookkeeping.** `tails`, `lengths` and `pointers` moved
from `Vec<usize>` to `Vec<u32>`. `pointers` is read once per step of every bucket walk, over an array
that reaches megabytes on this workload, so halving its element width was expected to cut the
cache-miss cost of that walk.

Measured, it bought **3.9%** on the port's own p50 — 50.25 ns to 48.29 ns. That is real and it is
kept, but it is not enough to call the width the cause of the regression: the falsifier named before
the measurement was "if the cost is the allocator traffic of returning a fresh container per call
rather than the width of the walk, this shows little or no effect", and 3.9% is nearer to little.
The probe above already found `get` doing exactly one allocation per call and a bare
`Vec::with_capacity(25)` costing 34.88 ns of the 50.96, which points the same way.

The gate-10 ratio at that point read 1.62× slower rather than the original 1.9×, but most of that
move was the JavaScript baseline, which measured 26.4 ns in the earlier session and 29.81 ns in this
one on unchanged code. Within one session the harness reproduces to 0.9%; across sessions it does
not, so the port's own 3.9% was the figure to trust at that point.

**Then a second change, found by looking at `get` rather than at the walk — 17%.** `get` allocated
its result with `vec![0.0; length]`, which memsets `length * 8` bytes that the following `length`
steps then overwrite one by one. It now builds with `Vec::with_capacity` and `push`, reversing at
the end (the walk runs tail-to-head, so the reverse restores upstream's order over bytes already in
cache), and matches the `Storage` discriminant once per call instead of once per element.

Measured across four runs, the port's own p50 landed between 38.3 and 40.2 ns against 48.3 before —
a spread of about 5% on the port side against 5.3% on the JavaScript side, which is why the port's
own figure is the one quoted in the current document. **50.2 → 48.3 → 38.3 ns** across the two
changes; in the final whole-suite pass the ratio reads 1.31× slower, the figure now stated as
current.

**Verdict on the allocation hypothesis: confirmed as a real, measured cause — not shown to be the
sole one.** The published p50 gap is 23.8 ns/op averaged over *all* three ops in the mix; `get` is
25% of the mix, so fully explaining the whole-workload gap from `get` alone would need roughly 4×
that, ≈95 ns of `get`-specific excess cost. The measured 50.14 ns/call accounts for a bit over half
of that back-of-envelope figure — a real, substantial contribution, but the arithmetic does not
support calling it the entire explanation, and no probe was run against `set` to check whether it
also carries part of the gap.

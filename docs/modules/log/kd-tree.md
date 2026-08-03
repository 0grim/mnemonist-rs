# kd-tree — working log

Chronological. See `docs/modules/kd-tree.md` for the current-state document and
`docs/modules/evidence/kd-tree.md` for the gate artifacts.

## Bench: `k_nearest_neighbors` per-call allocation cost investigated and fixed

**A real, measured loss on p50/p99/min in the original gate-10 pass — the sharpest among this group
of modules.** Isolated with a standalone probe (200,000 calls of each method alone, same tree, both
sides): `nearest_neighbor` alone was 331 ns/call against upstream's 620 ns (the port winning,
consistent with the rest of this group), but `k_nearest_neighbors` alone was 6.6 µs/call against
upstream's 2.1 µs — a genuine reversal, and disproportionate: this port's own k-NN path cost **20×**
its own `nearest_neighbor`, where upstream's cost only **3.4×** its own. `recurse_knn`
heap-allocated a fresh 3-element `Vec<f64>` per node visited into `FixedReverseHeap`'s backing
store — a plausible mechanism (V8's generational GC can bump-allocate the equivalent short-lived
array far more cheaply than repeated small `malloc`s), consistent with where the two sides' costs
diverge, but originally not confirmed with a profiler or allocation count, so it was labelled a
hypothesis rather than a finding.

**Confirmed 2026-08-02**, with an allocation-counting global allocator (no `perf`/`cargo flamegraph`
available on this host — see the investigation's own report for the full tool inventory).
`bench/runner/examples/kd_tree_alloc_probe.rs` builds the same shape of tree (100,000 scattered 2-D
points) and runs 20,000 calls of each method alone, counting real heap allocations rather than
reading the source and assuming them. Results (see evidence file for the table): zero allocations
for `nearest_neighbor`, essentially one allocation per node visited for `k_nearest_neighbors`
(≈499 of them per call, each the 3-element `Vec<f64>` `recurse_knn` pushes) — the self-ratio this
probe measured, 22.2×, reproduces the 20× the original gate-10 run found (a different, unmatched
PRNG and a smaller call count, so exact agreement was not expected; the same order of magnitude is
what confirmed it). **Verdict: confirmed.** The per-node allocation was real, measured directly
rather than inferred, and its count tracked the disproportionate cost one-for-one. RSS and startup
still favoured the port throughout.

**Fixed 2026-08-03**, by the route the investigation proposed: `k_nearest_neighbors` and
`linear_k_nearest_neighbors` now hold `[f64; 3]` and `[f64; 2]` instead of `Vec<f64>`. Both are
`Copy`, so the `Store::get`/`set` clone on every sift step is a stack copy rather than a `malloc`.
`TupleComparator` gained a matching `Comparator<[T; N], E>` impl, appended beside the `Vec<T>` one
rather than replacing it; the two are the same lexicographic rule, and since every tuple here is
exactly `N` long the "shorter than `N`" case only the `Vec` impl needs is unreachable from either.

The same allocation probe, re-run unchanged, showed 499.3 allocations per call become 7.0 (a 71×
reduction) and time per call fall 4.2×, from 8,071 ns to 1,909 ns. The self-ratio fell from 22.2× to
5.4×, against upstream's own 3.4×. This is the metric that would have falsified the explanation had
it been wrong: had the cost been the pointer chase through `lefts`/`rights`/`pivots`, or
`squared_distance`'s per-dimension loop, removing the allocations would have left the time where it
was.

The full gate-10 workload, re-measured in the same session as every other figure in this port's
performance table, moved from 2049.08 ns/op (port) vs 939.76 ns/op (upstream) — upstream 2.2×
faster — to 755.10 ns/op vs 927.85 ns/op — port 1.23× faster. Both numbers are now in the current
document/evidence file as the final state.

One caution against reading the swing as larger than it is. The upstream figure above, 939.76 ns,
and this pass's 927.85 ns are the *same code on the same host*; an intervening run of the same
workload measured 1159.03 ns, 22% away from both. Back to back within one session the harness
reproduces to 0.9% on each side, so the honest statement of the improvement is the port's own
time — 2049 ns to 755 ns — rather than the ratio's move from 2.2× slower to 1.23× faster, which
carries that drift.

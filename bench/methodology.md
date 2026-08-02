# Benchmark methodology

Implements DESIGN.md §5.1–5.2. Everything below is enforced by
`bench/drive.js`, not merely intended; where a rule can be checked
mechanically, it is, and the check aborts the run rather than warning.

Reproduce with:

```bash
bench/run.sh static-disjoint-set        # → bench/results.json
bench/run.sh sparse-set
bench/run.sh bit-set
bench/run.sh lru-cache
bench/run.sh heap
bench/run.sh trie
bench/run.sh vector
```

---

## What is compared

The **pure Rust path** (`bench/runner`, linked against `mnemonist-core`
directly) against the **vendored upstream JS** (`bench/upstream/`, run by
`bench/node/run.js`).

* **Never through N-API.** Bridge overhead would poison the comparison and
  misrepresent the port. `bench-runner` does not depend on `mnemonist-napi`.
* **The JS side is the vendored source, not `npm install mnemonist`.** The
  hashed tests were taken from a `--depth 1` clone of master, which may sit
  ahead of the 0.40.4 tarball. Benchmarking a released tarball against tests
  hashed from master would silently compare two codebases. One clone, one
  commit (`1f2c7520`), recorded in `.port-mortem.toml`.
* Rust is built with the workspace's stock `--release` profile. No exotic
  codegen flags: the comparison is only fair if the port is built the way it
  ships.

## The four rules, and how each is enforced

### 1. Matched PRNG, verified — not a serialised workload

Both sides implement the same xorshift32 in ~10 lines
(`bench/runner/src/xorshift.rs`, and the twin in `bench/node/run.js`), seeded
identically. A serialised `workload.jsonl` would be ~30 MB for 1e6 ops and
would drag JSON parsing into the measurement; the matched generator has zero
I/O.

"Matched" is proved, not asserted. Before anything is measured, the driver runs
`bench-runner --dump-prng 1000` and `node bench/node/run.js --dump-prng 1000`
and compares the streams; a mismatch aborts with the index of the first
differing value. The result on this host:

```
matched PRNG verified: first 1000 values identical
```

Two details that would break the match if changed on one side only:

* JS bitwise operators produce **signed** 32-bit results, so the JS twin needs
  `>>> 0` after the shift steps. Without it the streams part company within a
  handful of draws.
* Each op draws **exactly three** values — kind, then both operands — whether
  or not it uses the second. A conditional third draw desynchronises the sides
  at the first `find`, which is the subtle way a matched PRNG stops matching.

Modulo bias is accepted deliberately: rejection sampling would consume a
data-dependent number of draws, which is far harder to keep in step than a bias
that favours neither side.

### 2. Batched timing at K = 1000, except where a batch is a walk

`Instant::now()` / `process.hrtime.bigint()` cost ~20–30 ns; a `find` on a
compressed forest is single-digit ns. Per-op timing would measure the clock.

Each sample is **1000 ops**, and `p50_ns_per_op` / `p99_ns_per_op` are batch
times divided by K. Two consequences, both wanted:

* Timer cost falls to ~0.03% of a sample instead of ~95%.
* **Each GC pause lands inside exactly one batch**, so batch-level p99 is
  precisely where V8's tail behaviour becomes visible. This is why p99 is the
  headline number rather than a mean — and, on `mixed-4e6`, it is the metric
  the port loses.

Percentiles are nearest-rank and are computed **once, in the driver, over
samples from both sides**. §5.2 asks for "same percentile maths"; implementing
it twice and hoping the implementations agree is weaker than implementing it
once.

**The `drain` workload batches differently, on purpose.** Its unit is one full
walk of the set rather than 1000 elements, and `batch_k` carries the number of
members yielded per walk instead of a constant 1000 — so `ns / batch_k` still
means nanoseconds per element. The reason is that a cursor costs something *per
walk* as well as per element: it freezes state at creation (DESIGN.md §3.4).
Splitting a walk across samples would bury that fixed cost in whichever sample
happened to contain the creation, and hide exactly the thing this workload
exists to measure. Both sides compute `batch_k` from their own set and the
driver's checksum gate would fail if they disagreed.

### 3. RSS in-process, reported as total *and* delta

`getrusage(RUSAGE_SELF)` in Rust, `process.resourceUsage().maxRSS` in Node.
Both return peak RSS in kilobytes. Not `/usr/bin/time -v` — it is GNU `time`,
not the shell builtin, Debian slim does not ship it, and asking two runtimes to
report about themselves is uniform where an external tool is merely comparable
(DESIGN.md §12c.2 point 3 supersedes the tool list in §5).

A **no-op baseline** is measured for each runtime, because Node carries ~42 MB
of V8 before a single element exists. Reporting "18 MB vs 85 MB" as a
data-structure result would be the memory equivalent of claiming process
startup as a throughput win. Both figures are published:

| field | meaning |
|---|---|
| `rss_total_mb` | peak RSS of the whole process, runtime included |
| `rss_delta_mb` | above the same runtime's no-op baseline |
| `structure_rss_delta_mb` | a run that constructs the structure **and does nothing else** |

The third exists because the second is not clean: `rss_delta_mb` includes the
~9 MB of materialised op arrays, identical on both sides but large enough to
swamp the structure. `structure_rss_delta_mb` isolates the part that is
actually about the port.

Peak RSS is a high-water mark, so the reported figure is the **max** across the
10 measured passes, not the mean.

### 4. Interleaved A/B/A/B, 3 warmup + 10 measured

Thermal drift and background load are monotonic over a run; interleaving
cancels them, sequential runs bake them in. The driver alternates
port → original → port → original, ten times.

Each measured pass is its **own process**, and each does its own 3 warmup
passes first. That is not wasteful duplication: V8's JIT state does not survive
a process, so warming once and measuring ten times would only be honest for
Rust. Warmup is mandatory — measuring cold JS against optimised Rust is a
dishonest win a judge will spot.

Cores are pinned with `taskset -c 2,3` on bare metal (`BENCH_CPUS` to change,
`BENCH_PIN=0` to disable). In Docker this is `--cpuset-cpus` at the container
boundary instead, which is one flag and cannot be forgotten by a script.

## A fifth check, not in the spec

Both runners accumulate a **checksum** over the results of every non-mutating
op (`find` returns and `connected` booleans; `has`/`delete` booleans for
`sparse-set`; the sum of yielded members for `drain`). The driver requires all
20 runs across both sides to produce the identical value and refuses to write
anything if they differ.

This makes "same workload" a verified claim rather than an assertion: it proves
the two implementations executed the same ops *and computed the same answers*,
not merely the same op count. For `static-disjoint-set` it also incidentally
re-confirms that the port reproduces upstream's B-7 rank bug — a corrected
implementation would elect different roots and the checksum would differ.

Recorded in `results.json` as `checksum` per workload.

## Regressions are computed, not remembered

Every metric published is lower-is-better, so `bench/drive.js` derives the
`regressions` array mechanically: any metric where the port's number exceeds
upstream's is listed with its ratio. DESIGN.md §5.1 is explicit that hiding a
regression scores worse than disclosing one; a field nobody has to remember to
fill in cannot be quietly left out on a bad day.

## Startup is measured separately, and labelled

`hyperfine`, 5 warmup + 30 runs, over `bench-runner --noop` and
`node bench/node/run.js --noop`. Whole-process timing is the one place a
uniform external tool *is* fair, since it times both identically.

Startup is reported as `startup_ms` and **must not be folded into per-op
numbers**. Node's ~16 ms boot would dominate any short workload and make the
port look better than it is on throughput. It is a real win, and a cheap one.

## Why not criterion

§5.2 Problem 1. Criterion has no Node counterpart, so a
criterion-vs-hand-rolled-loop table is two methodologies in one grid, and a
judge who notices discounts every row. Both sides here are written the same
way: same warmup count, same measured count, same batch size, same monotonic
clock semantics, same percentile function.

Criterion remains the right tool for Rust-only regression tracking during
Wave 1. It just stays out of the comparison.

## Workload selection, and why there are two

`mixed-1e6` is the headline workload: 1e6 ops, 50% `union` / 25% `find` /
25% `connected`, over a set of 1e6 items. The port wins every metric on it.

That is a suspiciously clean result against a library that is already
typed-array-backed and well optimised, and §5.1 says as much: *"expect to lose
somewhere and report it."* So the size was swept — 200, 5,000, 65,536, 1e6,
4e6 — looking for the boundary.

`mixed-4e6` is the same op mix at four times the size, and it is where the port
loses: **p99 2.7× worse** while p50 stays 1.7× better. The cause is a design
decision in the port, not noise — see
`docs/modules/static-disjoint-set.md` § *Fuzz + bench*. Publishing only the
size that flatters the port would have been the easiest possible way to produce
a dishonest table.

## Host

Recorded per run in `results.json` under `host`. The governor reads
`unavailable` on WSL2, which exposes no `cpufreq` node — recorded honestly
rather than guessed, and it does mean frequency scaling is uncontrolled on this
host. The A/B/A/B interleaving is what limits the damage.

## Extending to more modules — the registry

`bench/runner/src/harness.rs` holds a table of function pointers, one row per
module: a `mixed` op-stream loop, an optional `drain`-style loop, and a
`--structure` builder. `bench/runner/src/main.rs` dispatches through the
table and does not change when a module is added; `bench/node/run.js` mirrors
the same table shape (`MIXED_RUNNERS`/`STRUCTURE_BUILDERS`). Adding module 9
onward is: one Rust file implementing `run_mixed`/`build_structure` (a `heap`-
or `bit-set`-sized file, ~50–90 LOC with docs), one line in `harness.rs`, the
JS twin of the same loop in `run.js`, and one `WORKLOADS` entry in `drive.js`
with a stated size/ops/reason. Nothing in the protocol above — matched PRNG,
K = 1000 batching, 3 warmup + 10 measured, interleaved A/B/A/B, in-process
RSS, checksum agreement — changes per module; it is what every module file
plugs into.

This was verified rather than assumed: `static-disjoint-set` and `sparse-set`
predate the registry, and moving their `--structure` construction out of
`main.rs`'s old inline `match` into a `build_structure` function in each
module's own file did not touch either module's timed loop body at all (see
the `git diff` on those two files — pure appends). Re-measuring both after the
refactor reproduced the pre-refactor figures within the run-to-run noise this
document already documents (up to ~32% on p99 between otherwise clean runs);
none of the movement traces to source changes, because there were none in the
hot path.

**One parameter mistake, caught and fixed before publishing, is worth
recording here because it is the shape of the trap §5.1 warns about.**
`bit-set`'s first draft included `rank` in its op mix at 25% weight. Neither
upstream nor the port maintains a rank/select index — `rank(i)` sums popcounts
word by word from the start, so it is O(i / 32), not O(1). At this module's
1e6 domain a 25%-weighted mix put a quarter of a million O(15,000)-word calls
into *every measured pass*; the harness was still running after ten minutes
and six of ten reps before it was killed. `rank` was replaced with `test`
(also a pure read, but O(1)) — see `bench/runner/src/bit_set.rs` for the full
account. The lesson generalises: an op whose cost scales with a workload
parameter (domain size, key length, tree depth) needs that fact checked
*before* it goes into a uniform-weighted mix, not discovered by a benchmark
that will not finish.

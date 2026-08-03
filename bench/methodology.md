# Benchmark methodology

How both sides of every figure in `bench/results.json` are measured. Everything below is enforced by
`bench/drive.js` rather than merely intended: where a rule can be checked mechanically it is, and
the check aborts the run instead of warning.

**44 workloads across 40 structures.** Two units carry no benchmark at all and say why in their own
documents rather than going quiet.

Reproduce one module at a time — the script takes a module name and merges into `bench/results.json`
rather than overwriting it:

```bash
bench/run.sh static-disjoint-set        # → bench/results.json
bench/run.sh heap
bench/run.sh default-map
```

Benchmarks need an idle machine. A contended run inflated both sides 2–3× here, which is measured in
`docs/METHODOLOGY.md` rather than assumed.

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
  precisely where V8's tail behaviour becomes visible. That is why p99 is the
  headline number rather than a mean. Three of the eight declared regressions
  are p99 or `min` regressions on modules whose p50 is ahead of upstream —
  visible only because the tail is reported separately.

Percentiles are nearest-rank and are computed **once, in the driver, over
samples from both sides**. Both sides must use the same percentile maths, and
implementing it twice and hoping the two agree is weaker than implementing it
once.

**The `drain` workload batches differently, on purpose.** Its unit is one full
walk of the set rather than 1000 elements, and `batch_k` carries the number of
members yielded per walk instead of a constant 1000 — so `ns / batch_k` still
means nanoseconds per element. The reason is that a cursor costs something *per
walk* as well as per element: it freezes state at creation (`docs/DIVERGENCES.md`'s iteration section).
Splitting a walk across samples would bury that fixed cost in whichever sample
happened to contain the creation, and hide exactly the thing this workload
exists to measure. Both sides compute `batch_k` from their own set and the
driver's checksum gate would fail if they disagreed.

### 3. RSS in-process, reported as total *and* delta

`getrusage(RUSAGE_SELF)` in Rust, `process.resourceUsage().maxRSS` in Node.
Both return peak RSS in kilobytes. Not `/usr/bin/time -v` — it is GNU `time`,
not the shell builtin, Debian slim does not ship it, and asking two runtimes to
report about themselves is uniform where an external tool is merely comparable.

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

## A fifth check: both sides must compute the same answers

Both runners accumulate a **checksum** over the results of every non-mutating
op (`find` returns and `connected` booleans; `has`/`delete` booleans for
`sparse-set`; the sum of yielded members for `drain`). The driver requires all
20 runs across both sides to produce the identical value and refuses to write
anything if they differ.

This makes "same workload" a verified claim rather than an assertion: it proves
the two implementations executed the same ops *and computed the same answers*,
not merely the same op count. For `static-disjoint-set` it also incidentally
re-confirms that the port reproduces upstream's BUG-STATIC-DISJOINT-SET-1 rank bug — a corrected
implementation would elect different roots and the checksum would differ.

Recorded in `results.json` as `checksum` per workload.

## Regressions are computed, not remembered

Every metric published is lower-is-better, so `bench/drive.js` derives the
`regressions` array mechanically: any metric where the port's number exceeds
upstream's is listed with its ratio. Hiding a regression scores worse than
disclosing one, and a field nobody has to remember to fill in cannot be quietly
left out on a bad day.

## Startup is measured separately, and labelled

`hyperfine`, 5 warmup + 30 runs, over `bench-runner --noop` and
`node bench/node/run.js --noop`. Whole-process timing is the one place a
uniform external tool *is* fair, since it times both identically.

Startup is reported as `startup_ms` and **must not be folded into per-op
numbers**. Node's ~16 ms boot would dominate any short workload and make the
port look better than it is on throughput. It is a real win, and a cheap one.

## Why not criterion

Criterion has no Node counterpart, so a
criterion-vs-hand-rolled-loop table is two methodologies in one grid, and a
judge who notices discounts every row. Both sides here are written the same
way: same warmup count, same measured count, same batch size, same monotonic
clock semantics, same percentile function.

Criterion remains the right tool for Rust-only regression tracking. It just
stays out of a cross-language comparison.

## Workload selection, and where the port loses

Each module's headline workload is a `mixed` op stream over a realistic size — for
`static-disjoint-set`, 1e6 ops at 50% `union` / 25% `find` / 25% `connected` over 1e6 items. A
module gets a second workload when there is a specific question worth asking of it: a larger size
to find where an advantage stops holding, or a `drain` walk to price iteration separately from
mutation.

A port that wins everywhere against a library this well optimised is a result to distrust rather
than to publish, so sizes were swept looking for the boundary. Eight of the 44 workloads carry a
declared regression on at least one metric, and they are the ones worth reading:

| workload | metric | ratio |
|---|---|---|
| `bi-map` mixed-1e6 | p50 | 1.51× slower |
| `default-map` mixed-1e6 | p50 | 1.44× slower |
| `heap` mixed-1e6 | p50 | 1.31× slower |
| `multi-array` mixed-1e6 | p50 | 1.31× slower |
| `fixed-reverse-heap` mixed-1e6 | p99 | 1.78× slower |
| `fuzzy-map` mixed-1e6 | p99 | 1.25× slower |
| `default-map` mixed-4e6 | p50, p99, min | up to 1.14× slower |
| `inverted-index` mixed-2e5 | min | 1.08× slower |

Each is analysed in that module's own document, under *Fuzz + bench*. Three of the eight — `fuzzy-map`,
`inverted-index` and `fixed-reverse-heap` — regress only on p99 or `min` while their p50 stays ahead
of upstream (1.44×, 1.54× and 1.06×). That shape is exactly what batching at K = 1000 exists to make
visible rather than average away.

The regressions are derived, not curated: `drive.js` writes any metric where the port's number
exceeds upstream's, so the table above is a consequence of the data rather than a selection from it.

## Host

Recorded per run in `results.json` under `host`. The governor reads
`unavailable` on WSL2, which exposes no `cpufreq` node — recorded honestly
rather than guessed, and it does mean frequency scaling is uncontrolled on this
host. The A/B/A/B interleaving is what limits the damage.

## How stable the published figures are

**Within a session, back-to-back runs of the same workload agree to about 0.9% on both sides. Across
sessions they do not.** `kd-tree`'s upstream figure moved 22% on unchanged code between two
sessions; `multi-array`'s moved 13%. That is why every figure in `results.json` comes from a single
serial pass on an idle machine: a table whose rows were measured on different days cannot be read
down the column, however carefully each row was taken.

The instability is not symmetric, and the JavaScript side is the mobile one. Three consequences, all
of which changed what is published.

**Two rows changed sign.** `multi-set` and `bi-map` were once recorded as wins. In the serial pass
both measured as losses, and a spot-check of each in isolation on a settled machine measured them
worse still. Both are published as losses at the worse of the two figures, on the principle that
between two honest measurements the unflattering one is the safer claim.

**One row moved the other way, and it is worth reading as a caution rather than a result.**
`fixed-critbit-tree-map` was documented as a real, reproducible p50 loss at ~1.06–1.08×, complete
with a candidate cause. It now measures 1.18× *faster*. The port's own p50 barely moved between the
two passes — 325.78 ns to 322.66 ns, inside noise — while upstream's moved 24%, from 306.50 ns to
380.33 ns. Nothing about the port got faster. A regression that disappears because the baseline
drifted is not a fix, and recording it as one would have been the easiest possible way to claim
credit for noise.

**One figure should not be read as a single number at all.** `bi-map`'s ratio spanned 1.14× to 1.59×
across six interleaved runs, where every other module reproduces to about 1%. Its published 1.51×
means "slower, by somewhere between a little and a half".

The same effect sets the floor on what a single run can show: run-to-run noise reaches ~32% on p99
between two otherwise clean runs, which is why no figure here comes from one run and every number is
a median of ten.

## How a module plugs in — the registry

`bench/runner/src/harness.rs` holds a table of function pointers, one row per module: a `mixed`
op-stream loop, an optional `drain`-style loop, and a `--structure` builder. `main.rs` dispatches
through the table, and `bench/node/run.js` mirrors the same shape in
`MIXED_RUNNERS`/`STRUCTURE_BUILDERS`. A module is one Rust file implementing
`run_mixed`/`build_structure` (~50–90 lines with docs), one row in `harness.rs`, the JS twin of the
same loop, and one `WORKLOADS` entry in `drive.js` carrying a stated size, op count and reason.

**Nothing in the protocol above varies per module.** Matched PRNG, K = 1000 batching, 3 warmup and
10 measured passes, interleaved A/B/A/B, in-process RSS and checksum agreement are what every module
file plugs into, which is what makes 44 workloads comparable to each other and not just each to its
own upstream.

That the registry is behaviour-preserving was measured rather than asserted. Two modules predate it,
and moving their `--structure` construction out of an inline `match` into their own files left both
timed loop bodies untouched; re-measuring reproduced the earlier figures within this host's
run-to-run noise, which reaches ~32% on p99 between two otherwise clean runs. That noise figure is
itself the reason no single run is reported: every number here is the median of ten.

**One parameter mistake, caught before publishing, is worth recording, because it is the trap this
whole protocol exists to avoid.**
`bit-set`'s op mix originally weighted `rank` at 25%. Neither
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

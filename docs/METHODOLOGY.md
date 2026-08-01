# How this port was verified

A port can claim equivalence in two ways. It can assert it, or it can build instruments capable of
disproving it and report what they found. This document describes the instruments, what they cover,
and — the part that took longest to learn — **what they cannot see**.

Figures below are current as of writing and derived, not hand-maintained; `scripts/status.sh`
regenerates them.

---

## 1. The original tests are the contract, and they are unmodified

`tests/original/` holds the upstream `mnemonist` test suite exactly as published. All **47 files are
SHA-256 hashed** in `tests/SHA256SUMS`, and `sha256sum -c` runs as a gate on every commit. If a
single byte changes, the build fails and the change must be explained rather than absorbed.

This matters because the easiest way to pass someone else's tests is to edit them. Hashing removes
that option deliberately, including from ourselves.

The tests run **unmodified** against the Rust build through a thin napi bridge: `tests/run.sh`
assembles a work tree, publishes the compiled addon as a resolvable package, and points mocha at the
original files. **680 upstream specs pass.**

### What a "unit" is

A unit is **the require-closure of one upstream test file**, not a source module.

`test/lru-cache.js` requires four modules — `LRUCache`, `LRUMap`, `LRUCacheWithDelete`,
`LRUMapWithDelete`. Porting three of them scores nothing, because the file fails at `require` before
a single assertion runs. Scoping by source module would have let us claim partial credit that does
not exist; scoping by closure means a unit is done only when its whole test file executes.

---

## 2. The Definition of Done is executable

Ten gates, specified in `planning/DESIGN.md` §1.1 and **enforced by `tests/verify.sh`**, which is
driven by `tests/scope.txt` — the file naming units we claim are finished.

The check runs in the direction that can fail: for every unit claimed done, it asserts the evidence
exists. A unit cannot be listed without a bridge shim, a green original test, a divergence doc
containing six required sections, a recorded falsification, a logged fuzz campaign, and benchmark
figures including an explicit regressions array.

`verify.sh` was itself falsified before being trusted: adding an unported unit to `scope.txt` failed
six gates, and deleting a `regressions` array failed gate 10. A verifier nobody has tried to break
is an assumption.

**Consequence we accept:** `scope.txt` lags reality. A unit complete in every respect but the
benchmark stays out of scope and `scripts/status.sh` reports it as `pend`. That understates progress
and is the honest picture.

---

## 3. Native tests, and why they are not enough

**721 Rust tests** cover the ported structures directly — including boundary cases upstream never
reaches, which are catalogued per module under *What upstream does NOT test*.

They are necessary and insufficient for one structural reason: **they were written by whoever wrote
the implementation**, against the same understanding of the source. A misreading of upstream
produces a matching test and a green light. That is what the next instrument exists to break.

---

## 4. Differential fuzzing against the real library

`crates/difffuzz` generates operation sequences with `proptest`, replays each against both our Rust
implementation and **real upstream JavaScript running in Node**, and compares observable state after
every operation. Divergences are minimised by proptest's shrinker and persisted as regression seeds.

**108 logged campaigns · 38 modules · 117.3M operations · zero divergences.** Every campaign line
lives in `fuzz/log.txt` with its seed, so any of them can be replayed exactly.

Three design points that decide whether such a harness means anything:

**The oracle runs the real thing.** Not a reimplementation of upstream's semantics — upstream's own
source, in Node, over a line-delimited JSON protocol. A hand-written model of what we believe
upstream does would encode our misunderstandings on both sides of the comparison.

**A campaign that runs no operations is a failure, not a pass.** `difffuzz` exits with a distinct
code when `ops == 0`, and the differential tests assert `report.ops > 0`. "Zero divergences out of
zero comparisons" is a true statement and a worthless one — see §7.

**The grammar must be shown to reach the interesting state.** This is the discipline that separates a
green campaign from a meaningful one, and it is enforced per structure by a `grammar_self_check`
that measures, with no oracle attached:

| structure | what must actually happen | measured |
|---|---|---|
| `lru-cache` | eviction, or it is just a map | 9.6% of ops evict; 5.0% evict / 2.1% delete on the with-delete grammar |
| `fibonacci-heap` | consolidation, which only fires on `extractMin` | 16,815 tree merges over 400 programs; 369/400 saw one |
| `trie` | keys sharing prefixes | pool of 8 where 5 are a strict prefix of another, plus a 2,000-sample dynamic check |
| `multi-map` family | one key genuinely holding several values | 25,761 multi-value-bucket steps, 4,157 drains to zero |
| `inverted-index` | documents sharing tokens | 99.6% of posting lists span >1 document |

A trie fuzzed with random long strings, or an LRU whose capacity exceeds its operation count, yields
a clean campaign that proves only that the structure can store things.

**Where a path does not exist, we say so.** `fibonacci-heap`'s brief demanded the cascading-cut path
be exercised. Upstream has no `decreaseKey`, no `delete`, no `mark` and no cut — it implements the
consolidation half of the structure and not the amortisation half. The honest answer to "make X
fire" is sometimes "X is not there", and it is only available to someone who reads the source
instead of tuning the grammar until the report looks right.

---

## 5. Falsification: gate 6

Every unit must show its tests are **capable of failing**. The protocol is fixed: name the assertion
the sabotage should break **before** running it, confirm red, revert, confirm green.

Naming the target first is the whole point. A sabotage chosen after seeing what breaks is a
description of the tests, not a test of them.

The gate has caught itself twice, and both results were kept rather than replaced with an easier
target:

- **`_utils` named three sabotages; two stayed green.** Relaxing the k-way tie-break and reversing
  `merge_two`'s swap condition left every assertion passing. Reported as findings about
  tie-invariance and swap-invariance in our own tests.
- **`fibonacci-heap` predicted one would stay green, and it did.** Flipping `push`'s `<=` tie-break
  is invisible to assertions because the values it reorders are *equal* — no expected-value check
  can observe it. The differential fuzzer caught it in 425 cases.

That second result is the clearest evidence here that the two instruments are not redundant. Every
other argument for the fuzzer is that it covers more of the same ground; this is a defect class the
assertions cannot express at all.

**A falsification that stays green is not automatically a failed gate.** It can be a true statement
about which instrument covers what. The failure mode is staying green and nobody asking why.

---

## 6. Benchmarks: gate 10

Matched workloads driven by an identical xorshift32 sequence on both sides, batch-timed, interleaved
A/B/A/B, 3 warmup and 10 measured rounds, with in-process RSS via `getrusage` and
`resourceUsage().maxRSS`.

**Benchmarks require an idle machine, and this is measured rather than assumed.** A contended run
inflated both sides 2–3× here; upstream's own p99 swung 32% between clean runs; a timing-sensitive
test flaked under agent contention and passed in isolation. Gate 10 therefore cannot be pipelined
with other work and runs as a serial pass on a quiet machine.

`bench/results.json` requires an explicit `regressions` array per workload. **A regression must be
stated, never absent** — `verify.sh` fails a unit whose benchmark entry omits the field, so "we were
slower here" cannot be expressed by silence.

---

## 7. What these instruments cannot see

The most useful thing this project learned is that **passing your own verification is not the same
as being correct**. `planning/NOTES.md` catalogues ten occasions where a confident green signal was
answering a different question than the one intended. Three worth stating here:

**A fuzz spec that never ran, reporting clean.** `fuzzy-map`'s harness matched hash-factory names
`"identity"`/`"lower"` while the oracle registers `fuzzyIdentity`/`fuzzyLower`. Every case panicked
at construction, and the campaign reported zero divergences *truthfully* — zero disagreements out of
zero comparisons. Nothing was broken; the arithmetic was correct; the number meant nothing. After
the fix: 1,210,496 real operations.

**Our own decoder manufacturing divergences.** Both `vector` specs opened with 1-ULP disagreements
indistinguishable from genuine port bugs. `serde_json`'s default float parser is not
correctly-rounded — parsing `38403.356486892444` lands one ULP from `f64::from_str`. A differential
fuzzer that decodes its oracle's numbers wrongly invents findings.

**The layer gap — a blind spot no amount of fuzzing closes.** `difffuzz` compares `mnemonist-core`
against upstream. **The napi bridge is not in that loop at all.** When a sabotage was planted in the
bridge's `get`, a direct script went red, the upstream suite stayed green, and the fuzzer stayed
green *and was right to*. Every defect that lives in the bridge — retention, borrow discipline,
argument marshalling, factory composition — is invisible to the fuzzer **by construction**, and
needs reading, boundary tests, or review instead.

We arrived at that conclusion three separate times before naming it: through a soundness bug where
`&self` on a frozen type let LLVM hoist reads across a re-entrant JS callback; through an
independent review that found three defects every gate had passed, one of which aborted Node with
`SIGABRT`; and finally through a gate designed to expose it.

### Things deliberately not tested, and why

- **GC timing in `default-weak-map`.** Its key pool is created once and held for the oracle
  process's lifetime, so no key is ever collectible mid-campaign. A `WeakMap`'s entries vanish when
  the collector decides; a differential test depending on *when* would flake. **A flaky red is worse
  than a narrow green — it teaches you to ignore the instrument.**
- **`intersectionUnique` with NaN (D-106).** A known gap in our own code; its `allow_nan` flag stays
  off for that function only, so its campaign is green over a region excluding a known
  disagreement. Recorded rather than hidden.
- **`trie` cursors across `delete` (D-201).** Upstream's iterator holds a live object reference;
  ours is path-based because it must resume across the FFI boundary. An architectural divergence,
  accepted and documented, with the grammar split so the two regimes do not mix.

---

## 8. Reproducing any of this

```bash
cargo test                       # 721 native tests
./tests/run.sh                   # 680 upstream specs, unmodified, through the bridge
./tests/verify.sh                # the ten gates, per unit in scope
sha256sum -c tests/SHA256SUMS    # the originals are untouched
cargo run -p difffuzz --release -- --module <name> --seed <n> --duration 60
```

Every line in `fuzz/log.txt` carries its seed and is replayable exactly. **Withdrawn campaigns are
commented out with their reason rather than deleted**, so a number we later found overstated stays
visible as a correction instead of disappearing from the record.

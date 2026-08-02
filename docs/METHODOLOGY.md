# How this port was verified

A port can claim equivalence with the library it replaces, or it can build instruments capable of
disproving that claim and report what they found.

Every unit of this port passes **ten gates** before it is considered finished. They are not a
checklist written afterwards — they are executable. `tests/verify.sh` reads the list of units
claimed complete and asserts, for each one, that the evidence exists. A unit cannot be claimed
without it.

---

## What counts as one unit of work

**A unit is the complete require-closure of one upstream test file, not one source module.**

This is forced by the harness rather than chosen. Every `require('../x.js')` in an upstream test
file sits at the top of the file, so one missing module throws before a single assertion runs and
the whole file fails with zero partial credit.

Most test files map to one module. These do not:

| Upstream test file | Modules it requires |
|---|---|
| `_utils.js` | `typed-arrays` + `binary-search` + `hash-tables` + `iterables` + `merge` |
| `lru-cache.js` | `lru-cache` + `lru-map` + `lru-cache-with-delete` + `lru-map-with-delete` |
| `multi-map.js` | `multi-map` + `vector` |
| `heap.js` | `heap` + `utils/comparators` |
| `kd-tree.js` | `kd-tree` + `utils/comparators` |

Porting three of the four LRU variants would score nothing. Scoping by source module would have
allowed claims of partial credit that does not exist.

---

## The ten gates at a glance

| # | Gate | What it asserts |
|---|---|---|
| 1 | **Closure ported** | every module the test file requires exists in the native crate |
| 2 | **Native crate stands alone** | `#![forbid(unsafe_code)]`, zero dependencies, builds and tests with Node absent |
| 3 | **Bridge and shim** | the compiled crate is reachable from JavaScript under the name upstream expects |
| 4 | **Original tests green, unmodified** | the upstream suite passes against the Rust build |
| 5 | **Originals provably untouched** | SHA-256 of every upstream test file still matches |
| 6 | **Falsification** | sabotaging the Rust turns the suite red — the tests can fail |
| 7 | **Native tests** | Rust tests covering what upstream's suite does not reach |
| 8 | **Divergence document** | every difference from upstream written down, in a fixed structure |
| 9 | **Differential fuzzing** | generated programs replayed against real upstream JS, zero divergences |
| 10 | **Benchmarks** | measured against upstream, with regressions stated rather than omitted |

Always, in addition: `cargo fmt --all --check` and `cargo clippy --all-targets -- -D warnings` clean.

---

## Gate 1 — the closure is ported

**Purpose.** Prevent claiming a unit that cannot actually run.

**How it runs.** The unit enters the scope list only once every module its test file requires
compiles into the native crate.

**What it caught.** `_utils.js` was estimated as a cheap unit. Its closure turned out to include
`merge.js` — 563 lines whose k-way path drives a Fibonacci heap that was not yet ported — and two
`binary-search` functions taking a JavaScript comparator. The gate forced that discovery before the
unit was claimed rather than after.

---

## Gate 2 — the native crate stands alone

**Purpose.** The deliverable is a Rust crate, not a JavaScript wrapper. The bridge to Node exists to
prove equivalence and must not leak into the product.

**How it runs.** `mnemonist-core` declares `#![forbid(unsafe_code)]`, has a **one-line dependency
tree**, and is built and tested with Node absent from the machine. All JavaScript-value handling —
`undefined` versus `null`, truthiness, `SameValueZero` key identity, array-class preservation — is
confined to the separate `mnemonist-napi` crate.

**What it caught.** It repeatedly pushed design the right way. In `fuzzy-map` the hash functions are
JavaScript callbacks; the gate forced the core to accept an already-hashed key and kept the callback
machinery in the bridge. Without it, a `napi` type would have leaked into the crate a Rust user
depends on.

---

## Gate 3 — bridge and shim

**Purpose.** Let the *unmodified* upstream tests resolve `require('../lru-map.js')` and reach Rust.

**How it runs.** A napi binding per module, plus a JavaScript shim per required module.
`tests/run.sh` assembles a work tree, publishes the compiled addon as a resolvable package, copies
the byte-identical originals in, and points mocha at them.

**What it caught.** That shims are per *module*, not per unit — the `lru-cache` unit needs four
shims for one test file. It also caught a fresh-clone-only failure: `npm install` prunes packages
its manifest does not mention, so publishing the addon before installing dependencies deleted it on
the first run of a clean checkout and worked on every run after.

---

## Gate 4 — the original tests pass, unmodified

**Purpose.** The primary equivalence evidence. Tests written for the JavaScript library pass against
Rust without alteration.

**How it runs.** `./tests/run.sh` — currently **702 upstream specs passing**.

**What it caught.** Everything a hand-written port forgets. It is also the gate that makes gate 6
necessary: a suite that passes tells you nothing until you have shown it can fail.

---

## Gate 5 — the originals are provably untouched

**Purpose.** The easiest way to pass someone else's tests is to edit them. This removes that option
for anyone maintaining the port, not only for an outside reviewer.

**How it runs.** All **47 upstream test files are SHA-256 hashed** in `tests/SHA256SUMS`, verified
by `sha256sum -c` on every commit. One changed byte fails the build.

**What it caught.** Nothing, which is the point. It is a commitment device, and its value comes from
having been fixed at the start of the project rather than negotiated later.

---

## Gate 6 — falsification

**Purpose.** Prove the tests are capable of failing. A green suite is evidence only if red was
reachable.

**How it runs.** A fixed protocol: **name the assertion the sabotage must break before running it**,
apply the sabotage, confirm red, revert, confirm green. Naming the target first is essential — a
sabotage chosen after seeing what breaks describes the tests rather than testing them.

**What it caught.** The gate exists because of a real miss. The first falsification attempt on
`StaticDisjointSet` sabotaged an `if x_root == y_root` branch that the test never takes, because
every union in it merges distinct sets. It stayed green and proved nothing.

Since then it has three times produced a *green* result that was investigated rather than discarded,
each with a different cause:

| Case | Cause | What it revealed |
|---|---|---|
| `fibonacci-heap` — flipping `push`'s `<=` tie-break | The assertions **cannot express** the defect: the values it reorders are *equal*, so no expected-value check can observe it | The differential fuzzer caught it in 425 cases. The two instruments are not redundant |
| `default-weak-map` — sabotaging the bridge's `get` | The sabotage sits in a layer the fuzzer does not drive | A structural blind spot — see *What these instruments cannot see* |
| `fixed-critbit-tree-map` — corrupting the critical-bit isolator | The corruption is a **self-consistent mirror**: the same wrong direction function drives insert and lookup, so every query answer still matches | A property of the structure, not the harness. The native test, which checks visitation order, failed as predicted |

A fourth case is the one the gate exists for, and unlike the three above it found a weakness in the
port's own tests rather than a fact about the instruments. Disabling `kd-tree`'s "search the other
side of the splitting plane" branch stayed green against the upstream-pinned fixture — that
particular query resolves through the primary descent alone — and stayed green against the first
native test written for it, a diagonal arrangement of points that happened to share the same
weakness. Two independent tests, blind to the same defect, for the same reason. The differential
fuzzer's dense-grid grammar caught it immediately; the native test was then rebuilt on a grid,
confirmed red, reverted, and confirmed green.

A falsification that stays green is not automatically a failed gate — it can be a true statement
about which instrument covers what. The failure mode is staying green and nobody asking why.

---

## Gate 7 — native tests

**Purpose.** Cover what upstream's suite does not reach. Each divergence document lists those gaps
explicitly.

**How it runs.** `cargo test` — currently **764 tests**.

**What it caught.** A great deal, with one structural limitation: these tests were written by
whoever wrote the implementation, against the same reading of the upstream source. A misreading
produces a matching test and a green light. Gates 4 and 9 exist to break that symmetry.

---

## Gate 8 — the divergence document

**Purpose.** Make every difference from upstream visible, rather than left for a user to discover.

**How it runs.** One document per unit in `docs/modules/`, each with six required sections: *What
upstream tests*, *What upstream does NOT test*, *What we test in addition*, *Bugs this found*,
*Deliberate divergences*, *Fuzz + bench*. `tests/verify.sh` greps for each heading by name and fails
the unit if any is missing. There are **39 such documents; the shortest is 141 lines**.

**What it caught.** Writing *what upstream does not test* per module is what surfaced most of the
upstream bugs found in this port — the question forces an adversarial reading of the original's
tests rather than a trusting one. It also caught cases where the port was more correct than
upstream, which under a fidelity requirement is a defect: `MultiSet`'s size counter had to be kept
as a tracked value rather than a derived one, so that upstream's drift on a failed delete reproduces
instead of silently healing.

---

## Gate 9 — differential fuzzing

**Purpose.** Compare against the real library on inputs nobody wrote a test for.

**How it runs.** Operation sequences are generated with `proptest`, replayed against both the Rust
implementation and **real upstream JavaScript running in Node**, comparing observable state after
every operation. Divergences are minimised by the shrinker and persisted as regression seeds.
Currently **116 logged campaigns across 42 modules, 124.5 million operations, zero divergences**.
Every line in `fuzz/log.txt` carries its seed and replays exactly.

Three design decisions determine whether such a harness means anything:

**The oracle runs the real thing** — upstream's own source in Node, over a line-delimited JSON
protocol, not a model of what upstream is believed to do. A reimplementation would encode the same
misunderstandings on both sides of the comparison.

**A campaign that runs no operations is a failure, not a pass.** The runner exits with a distinct
code when zero operations executed, and the tests assert it. "Zero divergences" over zero
comparisons is a true statement and a worthless one.

**The grammar must be shown to reach the interesting state.** Each module carries a
`grammar_self_check` that measures this with no oracle attached:

| Structure | What must actually happen | Measured |
|---|---|---|
| `lru-cache` | eviction, or it is merely a map | 9.6% of operations evict |
| `fibonacci-heap` | consolidation, which only fires on extract-min | 16,815 tree merges over 400 programs; 369/400 saw one |
| `trie` | keys sharing prefixes | pool of 8 where 5 are a strict prefix of another |
| `multi-map` family | one key genuinely holding several values | 25,761 multi-value-bucket steps; 4,157 drains to zero |
| `inverted-index` | documents sharing tokens | 99.6% of posting lists span more than one document |
| `fixed-critbit-tree-map` | capacity actually exceeded | ~60% of 500 sampled programs exceed it; 100% reach the resulting crash |

A trie fuzzed with random long strings, or an LRU whose capacity exceeds its operation count,
produces a clean campaign proving only that the structure can store things.

**What it caught.** Bugs the upstream suite does not reach — for instance that `BiMap`'s `clear()`
resets only one of its two size counters, found from a two-operation program. It also caught defects
in the port itself before any campaign was logged: a linked-list `forEach` advancing its cursor
*before* the callback ran where upstream advances after, and an `inverted-index` `clear()` that
rebinds its backing arrays rather than clearing them in place.

**Where a path does not exist, that is stated directly.** One requirement called for the Fibonacci
heap's cascading-cut path to be exercised. Upstream has no `decreaseKey`, no `delete`, no `mark` and
no cut — it implements the consolidation half of the structure and not the amortisation half. The
honest answer to "make X fire" is sometimes "X is not there", available only to someone who reads
the source instead of tuning the grammar until the report looks right.

---

## Gate 10 — benchmarks

**Purpose.** Establish what the port actually costs, and state regressions rather than omit them.

**How it runs.** Matched workloads driven by an identical xorshift32 sequence on both sides, batch
timed, interleaved A/B/A/B, 3 warmup and 10 measured rounds, with in-process peak-RSS sampling on
each side. Results are keyed per unit in `bench/results.json`, and **every workload must carry an
explicit `regressions` array** — `tests/verify.sh` fails a unit whose entry omits the field, so a
slowdown cannot be expressed by silence.

**Benchmarks require an idle machine, and this is measured rather than assumed.** A contended run
inflated both sides two- to threefold; upstream's own p99 swung 32% between otherwise clean runs; a
timing-sensitive test flaked under load and passed in isolation. Gate 10 therefore cannot run
alongside other work, and executes as a serial pass on a quiet machine.

**What it caught.** An early measurement attributed a memory improvement to a mechanism that a
follow-up metric did not support — the improvement was real, the explanation was not. Performance
claims are now checked against a metric that would falsify them, and anything unconfirmed is
labelled unconfirmed.

---

## What these instruments cannot see

This project's own verification instruments passed while measuring something other than correctness
on three separate occasions:

**A fuzz specification that never ran, reporting clean.** One module's harness referred to its hash
factories by names the oracle did not register. Every generated case failed at construction, and the
campaign reported zero divergences *truthfully* — zero disagreements out of zero comparisons.
Nothing was broken, the arithmetic was correct, and the number meant nothing. After the fix, the
same campaign executed 1,210,496 real operations.

**A decoder manufacturing divergences.** Two specifications opened with 1-ULP disagreements
indistinguishable from genuine port bugs. The JSON library's default float parser is not
correctly-rounded: parsing `38403.356486892444` lands one unit in the last place away from Rust's
own `f64` parser. A differential fuzzer that decodes its oracle's numbers wrongly invents findings.

**The layer gap.** The differential fuzzer compares the *native crate* against upstream; the napi
bridge is not in that loop. When a sabotage was planted in the bridge, a direct script went red, the
upstream suite stayed green, and the fuzzer stayed green, correctly so. Every defect living in the
bridge — reference retention, borrow discipline, argument marshalling, factory composition — is
invisible to fuzzing by construction, and needs reading, boundary tests or review instead.

That conclusion was reached three separate times before it was named: through a soundness bug where
`&self` on a frozen type let the optimiser hoist reads across a re-entrant JavaScript callback;
through an independent review that found three defects every gate had passed, one of which aborted
Node with `SIGABRT`; and finally through a falsification designed to expose it.

---

## What is deliberately not tested, and why

- **Garbage-collection timing in `default-weak-map`.** Its fuzz key pool is created once and held
  for the oracle process's lifetime, so no key is ever collectible mid-campaign. A `WeakMap`'s
  entries vanish when the collector decides; a differential test depending on *when* would flake,
  and a flaky red is worse than a narrow green, since it teaches the team to distrust the instrument.
- **`intersectionUnique` with `NaN`.** A known gap in the port's own code. Its campaign runs with
  `NaN` generation disabled *for that function only*, leaving it green over a region that excludes a
  known disagreement. Recorded in the module document rather than hidden.
- **Trie cursors across deletion.** Upstream's iterator holds a live object reference; the port's is
  path-based because it must resume across the language boundary. An architectural divergence,
  accepted and documented, with the fuzz grammar split so the two regimes do not mix.

---

## Reproducing any of this

```bash
cargo test                       # native tests
./tests/run.sh                   # upstream specs, unmodified, through the bridge
./tests/verify.sh                # the ten gates, per unit claimed complete
sha256sum -c tests/SHA256SUMS    # the originals are untouched
scripts/status.sh                # derived status: coverage and per-unit evidence

cargo run -p difffuzz --release -- --module <name> --seed <n> --duration 60
```

Every campaign line in `fuzz/log.txt` carries its seed and replays exactly. Withdrawn campaigns are
commented out with their reason rather than deleted, so a figure later found to be overstated stays
visible as a correction instead of disappearing from the record.

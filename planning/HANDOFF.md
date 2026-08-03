# HANDOFF — read this first if you are a new session

A Rust port of the JS library `mnemonist`, entered in Port Mortem 2026. Kickoff was
2026-07-31 18:00 UTC; **code freeze 2026-08-03 18:00 UTC**.

## Orientation, in this order

1. `CLAUDE.md` (repo root) — auto-loads. Working rules and the traps that have already cost time.
2. `scripts/status.sh` — **live derived state**. Nothing in it is hand-maintained, so nothing in it
   is stale. Trust it over this file for numbers.
3. `planning/ROADMAP.md` — the governing arithmetic (coverage 40% and proportional; rigor 50%).
4. `planning/DESIGN.md` §1.1 — the **Definition of Done**. Ten gates. Everything else is detail.
5. `planning/NOTES.md` — upstream bug candidates and the capture log for the write-up.
6. `planning/ADMIN-GUIDANCE.md` — **organiser statements, quoted verbatim.** What the deliverable
   is, and what the Bug Catcher prize requires. Cite it before arguing from assumption about either.

Judge-facing documents already written: `docs/METHODOLOGY.md` (the ten gates, what each caught, and
what the instruments cannot see) and `docs/ARCHITECTURE.md` (the crate split, and where fidelity
cost idiom). Both are deliberately self-contained — no references to `planning/`, no bug or decision
identifiers that mean nothing without their registries. **Keep them that way.**

`git log` is dense and deliberately so; the merge commits carry the reasoning.

## Position as of 2026-08-02

**The port is complete and gated. 42 of 42 upstream test files ported; 40 units pass all ten gates**
— `tests/verify.sh` reports 247 checks green, 94% of the repo by upstream test weight. 799 Rust
tests, 733 upstream harness specs. One branch, clean tree.

The two units not in scope are **excluded with stated reasons, not pending**:

- **`default-weak-map`** — keys must be objects and entries vanish at the collector's discretion, so
  a timing figure would measure V8's garbage collector rather than the structure.
- **`_utils`** — a require-closure of five unrelated pure-function files with no shared instance.
  The bench harness wires one function per module name; representing all five with one would
  misstate it, and splitting them would no longer be `_utils` in `results.json`.

Both are recorded in their own module docs. Two argued exclusions beat 42 rows where two quietly
mean nothing.

## What remains: the write-up, and only the write-up

Nothing further needs porting, fuzzing or benchmarking. In priority order:

1. **`BUGS.md`** — the Bug Catcher submission. See the open item below; it is a separate prize and
   the highest value per token remaining.
2. **`DECISIONS.md`** — from ~137 candidates plus the reconciliation described below. The biggest
   editorial job. Group thematically (JS number semantics, `undefined` vs absent, iteration order,
   re-entrancy), not by identifier.
3. **`README.md`** — write last; it mostly frames and links the others. Lead with what the
   organisers confirmed the deliverable is: a standalone Rust crate, with the original JS suite as
   the equivalence proof rather than a runtime dependency.
4. **Refresh figures** in `METHODOLOGY.md` and `ARCHITECTURE.md`. Both predate gate 10 having any
   results; `METHODOLOGY.md`'s gate 10 section in particular still says the benchmark caught little.
   It now has real material, including several losses.
5. Optional if budget allows: Dockerfile, CI, demo script (`DESIGN.md` §12), and the "dual port"
   idea — corrected variants of the six sites where fidelity cost idiom, verified by invariants
   rather than differential comparison, **added alongside and never modifying the faithful
   implementations**, which would put 124.5M fuzz operations of evidence at risk.

## Benchmark results worth carrying into the write-up

Not a clean sweep, and that is the point. Confirmed losses: **`kd-tree` 2.2×/1.8× slower** —
isolated to `k_nearest_neighbors`, since `nearest_neighbor` alone wins; **`default-map`** loses p50
and p99, re-checked at 4× domain and real; **`bi-map`** loses both; `multi-array` splits;
`fixed-critbit-tree-map`, `fixed-reverse-heap` and `bit-set` lose narrowly. Every unconfirmed cause
is labelled unconfirmed.

Three benchmark parameters were rejected before publishing — a `rank` op that was pathological
rather than representative, a `symspell` vocabulary that produced no near-misses, and `critbit` keys
that left every critical bit at byte 0. `vp-tree`'s superlinear construction was checked against
upstream's own JS and confirmed inherited rather than introduced.

## Open items that must not be forgotten

- **Bug Catcher prize — a named submission deliverable, not a by-product.** Claim it in the
  submission form with *clear repro steps, what the original does wrong, and how your port handles
  it*, reviewed at judging. We hold **72 candidates, 57 verified against Node 24.18.1**, and the
  `NOTES.md` entry shape already carries repro + why-the-suite-misses-it + how-we-handle-it. Three
  jobs: (1) **verify or demote the 7 unverified** — one disprovable claim discredits the rest, an
  asymmetry that makes silence cheaper than a maybe; (2) **rank them**, since 72 undifferentiated
  entries will not be read — the admins single out bugs *"surfaced when the original tests disagree
  with correct behavior"*, so a defect whose upstream test **asserts** the wrong result outranks one
  the tests merely miss; (3) lift the best into `BUGS.md`.
- **Unregistered divergences — do this before assembling `DECISIONS.md`.** Four module docs were
  reconciled into the registry as DIV-BK-TREE-1–323, and each doc's table now carries its D-number. **Later
  batches may have reintroduced the problem**, so re-run the check: any divergence row numbered `—`
  rather than `D-nnn` exists only in the module doc and will be dropped when `DECISIONS.md` is built
  from the registry. The docs are the more complete source.
- **Four duplicate D-numbers** — DIV-PROJ-2, DIV-SORT-1, DIV-SORT-2, DIV-LRU-CACHE-1 each appear twice, from a merge collision
  before D-ranges were allocated per batch. Untouched deliberately: renumbering means *editing*
  existing entries, which is how the first collision happened. Fix during the `DECISIONS.md` pass,
  when nothing else is writing to the file.
- **DIV-UTILS-3 — the one substantive caveat left.** `intersectionUnique`'s k-way path never used a heap
  and has a NaN-sentinel gap; its fuzz campaign runs with `allow_nan` off **for that function only**,
  so it is green over a region excluding a known disagreement. Closing it needs no new unit — it is
  a gap in our own code.
- **DIV-TRIE-MAP-2 — accepted, not a defect.** `trie`'s cursor-versus-delete divergence: upstream's iterator
  holds a live object reference, ours is path-based because it must resume across the FFI boundary.
  State it in `DECISIONS.md` as architectural rather than revisiting it.
- **Gate 7 flaked once**, reporting FAILING while `cargo test` by hand was green seconds later. It
  now prints the failing test names and a line saying a green second attempt is not a passing gate.
  The underlying flake is **not fixed** — a timing-sensitive fuzz-harness test is the suspect. If it
  recurs, read what it names; do not re-run until green.
- **One unmerged experimental branch**, `worktree-agent-aa33c59bb65f37e8a`: a generic benchmark
  runner driving every module through `difffuzz`'s executor. It **failed calibration** — 2.4–3.6×
  slower than the hand-written benchmarks because `apply` returns a `serde_json::Value` and mutating
  ops allocate a chaining envelope. Kept for the record; delete once we are sure we will not revisit.

## When a background agent dies

It has happened four times — process exit or a stall watchdog. Transcripts survive on disk.

1. **Preserve first.** `git -C .claude/worktrees/agent-<id> status --porcelain`; if anything is
   loose, commit it labelled **UNVERIFIED** — snapshotted by the orchestrator, not by the agent that
   wrote it, so nothing in it has been compiled or run. Never treat it as evidence.
2. **Resume, do not relaunch.** `SendMessage` to the agent id picks up from its transcript with its
   context intact, far cheaper than a fresh brief.
3. Tell it what you snapshotted and that it must re-verify anything that snapshot touched.

**Commit-per-module is the mitigation that actually worked.** The batch briefed that way stalled
halfway and lost nothing — six units were already committed. Agents without that line left 39 and 8
loose files.

## Traps that have already cost this project time

- **`cargo build` does not compile `#[cfg(test)]` blocks.** Run `cargo test`. Bitten repeatedly.
- **A pipeline's exit status is its last command's** — `cargo clippy | tail` reports `tail`'s
  success and shipped a red commit. Never pipe verification.
- **Merging: appending at the end of a shared list is necessary and not sufficient.** Ten split
  blocks across ten merges under parallel agents. The compiler catches them; `git` does not. After
  resolving, run `cargo test`, not `cargo build` — three compiled fine and only failed under test.
  Sequential agents have produced **zero** such repairs across the last nine merges.
- **rust-analyzer diagnostics go stale mid-merge.** They reported conflict markers in files that had
  none. `grep -rln '^<<<<<<< '` is authoritative; a successful compile is authoritative.
- **Reading one line is not reading the code.** A wildcard match arm in the bench runner looked like
  a live trap that would benchmark the wrong structure; an allowlist seventy lines earlier already
  rejected unknown modules. The claim was wrong and had to be retracted.
- **Route by task shape.** Sonnet has done every port and every benchmark batch since the heap tier
  at roughly a quarter the cost of Opus, and has repeatedly found defects beyond its brief.

## The one idea worth not losing

**Ten times** a confident green signal turned out to be verifying something other than what was
believed. The table is in `NOTES.md`. The sharpest: a fuzz spec whose oracle-side names never
matched, so every case panicked at construction and the campaign reported zero divergences
*truthfully* — zero disagreements out of zero comparisons.

The countermeasure is now standard in every brief: **make the instrument prove it reached the state
that matters.** Measured eviction rates for the LRU; a prefix pool where 5 of 8 entries are a strict
prefix of another; 195,920 consolidation merges for the Fibonacci heap; a 50% fill ratio with a
0.028% false-positive pool for the Bloom filter; 98.4% non-empty searches for symspell.

Gate 6 has now stayed green four times, each investigated to a different cause: the assertions could
not express the defect; the sabotage sat in a layer the fuzzer does not drive; the sabotage was a
symmetry nothing observable could distinguish; and — the outcome the gate exists for — **the test
was simply inadequate and was rebuilt**.

That is the strongest material the write-up has: **passing your own verification is not the same as
being correct.**

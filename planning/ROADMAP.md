# ROADMAP

**Live status is not in this file.** Run `scripts/status.sh` — it derives coverage, per-unit
evidence, in-flight worktrees and repo health from `tests/scope.txt`, `fuzz/log.txt`,
`bench/results.json`, `docs/modules/` and git. Nothing there is hand-maintained, because the
hand-maintained log in `NOTES.md` went stale within three hours and nobody noticed.

This file holds what a script cannot derive: **what we are doing next, and why.**

Companions: `DESIGN.md` (how everything works, incl. §1.1 Definition of Done) ·
`DECISIONS-CANDIDATES.md` (divergences feeding the submission) · `NOTES.md` (raw capture +
upstream bug candidates).

---

## The governing arithmetic

**Coverage is 40% of the score and scores proportionally. Rigor is 50%.**

Going from 60% to 100% coverage gains at most 16 of the 40 functionality points. The 30%
behavioural-equivalence and 20% code-quality categories come almost entirely from the DoD
artifacts — differential fuzzing, honest benchmarks, divergence docs, `forbid(unsafe_code)` — as do
all four bonuses.

**So the gates are worth more than the coverage they would buy.** Any plan that trades gate rigor
for module count is a losing trade, and this is the sentence to re-read at hour 55.

### Budget, measured at H+8

~65h to freeze, minus ~21h sleep, minus ~12h reserved at CP4 for the batched benchmark pass, docs,
video and submission → **~32 working hours**.

Remaining after Wave 1: **30 modules, 9,960 LOC**, plus 886 LOC of utils. Tier estimate:

| Tier | Modules | Est. |
|---|---|---|
| T2 heaps — comparator callbacks across FFI | 5 | ~5h |
| T3 maps — `JsKey` design + modules | 11 | ~14h |
| T4 tries — **needs new cursor machinery (D-38)** | 5 | ~8h |
| T5 spatial/probabilistic | 8 | ~12h |
| utils incl. `merge` (563 LOC) | 3 | ~4h |
| | | **~43h against ~32h** |

**A full port is ~1.35× over budget** before counting incidents, and we averaged one incident every
~2.5h in the first eight (session death, API error mid-agent, WSL wedge). Do not pre-commit to the
whole repo: that commitment is what would pressure us into cutting gates.

**Commit instead to maximal coverage subject to the DoD holding.** Let the gates pace the work.

---

## Tier order, by unlock value

Every tier is a *bridge* capability, not a core one — the pattern has held three times now.

| Tier | Unlocks | Test lines | Share |
|---|---|---|---|
| **T3 — JS `Map` semantics** | `lru-cache` 497 · `multi-map` 381 · `multi-set` 361 · `set` 194 · `bi-map` 189 · `fuzzy-multi-map` 189 · `fuzzy-map` 161 · `default-map` 111 | **2,083** | **26%** |
| T5 — spatial/probabilistic | kd/vp/bk-tree, symspell, passjoin, bloom, critbit ×2 | 1,209 | 15% |
| T4 — tries/strings | `trie-map` 305 · `trie` 254 · `multi-array` 238 · `inverted-index` 126 · `suffix-array` 113 | 1,036 | 13% |
| T2 — comparator callbacks | `heap` 232 · `fixed-reverse-heap` 123 · `fibonacci-heap` 115 · `static-interval-tree` 95 | 565 | 7% |
| utils | `_utils.js` (needs `binary-search`, `hash-tables`, `merge`) | 389 | 5% |

**T3 first.** One capability, larger than all of Wave 1, and the pilot path is unblocked.

**Realistic target: Wave 1 + T3 ≈ 57% with everything green.** Reassess at CP3 (H+52).

---

## T3 — what it actually is (verified, not assumed)

Every T3 module wraps a JS `Map` internally — checked across `default-map`, `set`, `bi-map`,
`fuzzy-map`, `multi-map`, `lru-cache`, `lru-map`. So the capability is precisely **reproduce `Map`
semantics in Rust with JS values as keys**:

- **SameValueZero** equality — `NaN` matches `NaN` as a key, `+0` and `-0` are one key
- **Guaranteed insertion order**, which `std::HashMap` does not provide
- **Object keys by identity**, not structure
- delete-then-reinsert moves a key to the end

**Core keeps a zero-dependency tree**, so no `indexmap` — an insertion-ordered map is ours to build
from `std`. Core is generic over `K: Hash + Eq`; the bridge supplies `JsKey`. Core never sees a JS
value.

**Open question, deliberately delegated with the tests in hand:** object identity across napi may
be unreachable or unnecessary. If upstream's tests only use string and number keys, scope to what
is reachable and document the limit — do not build machinery no test can reach.

### Pilot order
`default-map` (111 test lines, 162 LOC, single `Map`) → **stop and review the design** → `set` (194)
→ `lru-cache` family (497 across 4 modules). Hardest primitive on the simplest host; the same
principle paid off for the cursor and for `forEach`.

---

## Known blockers, not just work

**`default-weak-map` may be unportable.** `WeakMap` entries vanish when the key is garbage
collected; Rust cannot observe JS GC. Holding napi `Reference`s means entries never vanish — a
divergence with no faithful alternative. 60 test lines. Plan: documented divergence or honest
exclusion, not a heroic attempt.

**D-38 — pointer-chasing cursors are missing machinery, not another module.** `trie`, `trie-map`
and `linked-list` walk by pointer, and `Sequence` requires position to be an ordinal. Second cursor
abstraction needed before any of T4's 1,036 lines are reachable.

**All-or-nothing units punish running out of time mid-cluster** (§1.1). `_utils` needs five modules
and 886 LOC are still missing; the `lru-cache` unit needs four. 4/5 through at freeze scores zero.

**Cross-tier units.** Under §1.1 a unit is a test file's *require-closure*, which cuts across the
wave boundaries in §6:
- `multi-map` unit = `multi-map` + **`vector`** (Wave 1)
- `multi-set` unit = `multi-map` + `multi-set` + **`fixed-reverse-heap`** (T2)
- `kd-tree` unit = `kd-tree` + `utils/comparators`
- `sort` unit = `sort/quick` + `sort/insertion` + `utils/typed-arrays`

§6's wave lists predate this definition and are wrong where they disagree with it.

---

## Working agreements

- **`tests/scope.txt` never contains a module whose evidence is not real.** This is the property
  that makes the submission credible; a breadth push must not be allowed to erode it.
- **Gates 1–9 pipeline; gate 10 does not** (§7.3). Benchmarks need an idle machine — measured.
  Batch them into one quiet serial pass.
- **Isolated worktrees per agent**, shared-file edits additive only. Four branches merging into
  three registry files is the practical ceiling on parallelism.
- **Stop-and-review before a design is inherited.** Applied to the cursor, to `forEach`, and now to
  `JsKey`.
- **Rules live in `CLAUDE.md`, not in prompts.** It auto-loads for every agent in this repo. Hand-
  repeating the same lessons in each brief is how they drift — the `cargo clippy | tail -5` bug
  shipped because one agent's hand-written check differed from another's.

### Branch and merge policy

Observed failure: three agents produced **five** branches across three naming schemes, two of them
created *on top of* their auto-generated worktree branch and left behind.

- Agents **commit to the branch their worktree already has** and create no others (`CLAUDE.md`).
- The orchestrator merges with **`--no-ff` and a descriptive message**, then **deletes the branch**.
  Branch names therefore never reach the final repository — only the merge commit message, which
  the orchestrator writes.
- Merge commits are kept rather than rebased away: parallel agent work genuinely happened in
  parallel, and the graph showing that is honest history rather than clutter. Rebasing would also
  rewrite commits, which P5 forbids.
- Merge order: oldest branch point first, so conflicts surface one batch at a time.

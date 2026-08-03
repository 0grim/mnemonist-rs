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

### Budget: what the H+8 estimate got wrong

The original estimate said a full port was **~1.35× over budget** — ~43h of work against ~32h
available — and concluded: *do not pre-commit to the whole repo.*

That was wrong, and the reason it was wrong is worth keeping.

**At H+23 the port is 83% done with ~47h to freeze.** The estimate was not off because the work was
smaller than thought; it was off because it priced agent-hours as if they were serial human hours.
Three things changed the arithmetic:

1. **Ports run in parallel with the orchestrator's own work.** An agent porting `lru-cache` costs
   wall-clock it does not consume from anyone.
2. **Tier-first ordering compounded harder than modelled.** Each landed tier turned later units
   from "design a new capability" into "follow an existing reference".
3. **Model routing cut the per-unit cost roughly fourfold** without measurable quality loss — see
   Working agreements.

**The conclusion it drew was still right for the wrong reason.** Refusing to pre-commit to the whole
repo kept the gates from being squeezed, and the gates are what the score actually rewards. Had we
committed early to 100% coverage, the pressure would have landed on gate 6 and gate 9 — the two
that are easiest to satisfy dishonestly.

**The commitment remains: maximal coverage subject to the DoD holding.** Coverage is now the
*cheap* axis; the expensive and more valuable one is the submission write-up, which does not exist
yet.

### What remains

| # | batch | lines | bug IDs | status |
|---|---|---|---|---|
| 1 | `fibonacci-heap` + close DIV-UTILS-2 | 115 | BUG-FIBONACCI-HEAP-1–239 | merged |
| 2 | `default-weak-map`, `linked-list`, `inverted-index` | 325 | BUG-INVERTED-INDEX-1–259 | merged |
| 3 | `critbit-tree-map`, `fixed-critbit-tree-map` | 294 | BUG-FIXED-CRITBIT-TREE-MAP-1–279 | **in flight** |
| 4 | `vp-tree`, `kd-tree` | 344 | DIV-PROJ-62–299 | not started |
| 5 | `passjoin-index`, `symspell`, `multi-array` | 639 | DIV-PROJ-63–319 | not started |

Then: **benchmark pass** (gate 10, idle machine), scope the benched units, re-scope `sparse-set`,
and the submission — README, `DECISIONS.md`, demo, Dockerfile, CI. `DESIGN.md` §12 has the specs.

**The submission is now the binding item, not the port.** 105 decision candidates and 69 bug
candidates sit in two working files written by a dozen agents in their own idioms; no judge will
open either. Turning them into a document that reads start-to-finish is real work and is where the
rigor score actually lands. **If it comes to a choice, drop batch 5 rather than the write-up.**

---

## Tier order, by unlock value

> **Historical — every tier below has landed.** Kept because the ordering reasoning drove the
> project and was largely borne out: unblocking really was worth more than the module it shipped
> with. Read it as the record of why things happened in this order, not as work outstanding. For
> what is outstanding, see *What remains* above.

Every tier is a *bridge* capability, not a core one — the pattern has held three times now.

| Tier | Unlocks | Test lines | Share |
|---|---|---|---|
| **T3 — JS `Map` semantics** | `lru-cache` unit 497 · `multi-map` 381 · `multi-set` 361 · `bi-map` 189 · `fuzzy-multi-map` 189 · `fuzzy-map` 161 · `default-map` 111 | **1,889** | **24%** |
| **`set` — native `Set` free functions** *(was miscounted as T3)* | `set` 194 | 194 | 2% |
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

### Answered by the Phase 1 pilot — three corrections to what §3.3 assumed

Audited against the vendored sources, not inferred:

- **`set.js` is not a T3 module.** Zero `new Map(`, six `new Set(`, and it exports free functions
  (`intersection`, `union`, `difference`, `symmetricDifference`, `isSubset`, `isSuperset`) over
  native `Set`s. It is a boundary-coercion problem — read JS `Set`s, return a JS `Set` — not a
  storage one, and probably much cheaper than T3. **Its 194 lines came out of the T3 total.**
- **`lru-cache` and `lru-cache-with-delete` are backed by a plain `{}`, not a `Map`.** Their index
  therefore *string-coerces* keys while `entries()` reads the raw key array, and
  `test/lru-cache.js:65` asserts both halves. Only `lru-map`/`lru-map-with-delete` are Map-backed —
  so the 497-line unit needs **both** mechanisms, not one.
- `sparse-map` was called T0-not-T3; it does contain one `new Map(` (likely a `.from()` static), so
  that claim is not propagated. Moot in practice — it is already ported and green as T0.

**Object keys are deliberately not implemented.** Every key reaching a `Map` across all ten
T3-family test files is a **string or a number** — no object, `NaN`, `-0`, boolean, `null`,
`undefined` or Symbol anywhere. `fuzzy-map` accepts objects publicly but hashes them to strings
first. The two implementable identity designs (hidden `Symbol` tag: O(1) but mutates the caller's
object and fails on frozen ones; `napi_strict_equals` association list: O(n) plus a strong
reference to every key ever seen) are recorded in DESIGN §3.8 for when a module needs one.

**The harder problem turned out to be values, not keys.** `map.get('one').push(1)` mutates a stored
array in place, so values must be the caller's actual objects — but `napi_create_reference` rejects
a primitive at NAPI_VERSION 9 (measured: it failed 2 of 7 upstream assertions on first run), and one
V8 global handle per stored value would mean a million handles for a million-entry `lru-cache`.

### Pilot order
`default-map` (111 test lines, 162 LOC, single `Map`) → **stop and review the design** → `set` (194)
→ `lru-cache` family (497 across 4 modules). Hardest primitive on the simplest host; the same
principle paid off for the cursor and for `forEach`.

---

## Known blockers, not just work

**PORTBUG-1 — RESOLVED.** Six bridges converted to `inner: RefCell<Core>`; `static_disjoint_set` and
`hashed_array_tree` verified immune against the real precondition (can JS run while a `&self`
method is on the stack), not against "has no `forEach`". Repro falsified both ways: pre-fix
`[1,2,3,4]`, post-fix `[1,2]`. `tests/boundary/reentrancy.js` pins it with 22 specs, 8 of which
fail on the pre-fix build — and only against a **release** build, because the hoist is an
optimisation. `sparse-set` stays descoped until gate 10 measures the borrow-flag cost.

Kept below because the *shape* recurs: any new bridge taking a callback has the same exposure, and
`bk-tree`'s distance function is the next instance.

<details><summary>original entry</summary>

**PORTBUG-1 is systemic across the bridge, not a `sparse-set` defect.** A `#[napi]` method taking `&self`
on a `Freeze` type is `noalias readonly`, so LLVM hoists reads across a re-entrant JS callback. Two
agents reached this independently from opposite directions — one by probing `queue`'s `forEach`,
one by reasoning about `&self`/`&mut self` aliasing in the T3 bridge. **Every class with a
callback-taking method is exposed**, and the fix is interior mutability at the boundary. Count
callback-taking methods before scoping any module; `static-disjoint-set` has zero and is immune.

</details>

**`default-weak-map` may be unportable.** `WeakMap` entries vanish when the key is garbage
collected; Rust cannot observe JS GC. Holding napi `Reference`s means entries never vanish — a
divergence with no faithful alternative. 60 test lines. Plan: documented divergence or honest
exclusion, not a heroic attempt.

**DIV-PROJ-36 — pointer-chasing cursors are missing machinery, not another module.** `trie`, `trie-map`
and `linked-list` walk by pointer, and `Sequence` requires position to be an ordinal. Second cursor
abstraction needed before any of T4's 1,036 lines are reachable.

**All-or-nothing units punish running out of time mid-cluster** (§1.1). `_utils` needs five modules
and 886 LOC are still missing; the `lru-cache` unit needs four. 4/5 through at freeze scores zero.

**`_utils` is further away than "five modules" suggests — `merge` drags in a heap.**
`utils/merge.js` requires `typed-arrays` (have), `iterables` (in flight), `binary-search`, **and
`fibonacci-heap`**. So the 389-line `_utils` unit transitively needs a T2 heap module that is not
in the current T2 batch (which is `heap` + `fixed-reverse-heap`). Chain in full:

    _utils.js → typed-arrays ✓ · iterables · binary-search · hash-tables · merge → fibonacci-heap

Treat `_utils` as one of the *last* things reachable, not a lull-filler. `binary-search` and
`hash-tables` are worth porting early anyway — zero dependencies each, and `binary-search` is also
required by `merge` and `vp-tree`.

**Dependency facts worth not re-deriving** (checked against the vendored sources):

| module | requires | status |
|---|---|---|
| `binary-search`, `hash-tables` | nothing | fully independent |
| `suffix-array` | nothing | fully independent |
| `bloom-filter` | `murmurhash3`, `foreach` | unblocked (foreach exists) |
| `bk-tree`, `symspell` | `foreach` only | unblocked |
| `vp-tree` | `iterables`, `typed-arrays`, `sort/quick`, `binary-search`, `heap` | blocked on four |
| `merge` | `typed-arrays`, `iterables`, `binary-search`, `fibonacci-heap` | blocked |

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
- **Run gate 10 as one long unattended pass while nobody is working**, not as a pause between
  batches. At ~10 min per module it is ~2h of machine time that would otherwise block porting, and
  it is the *only* work here that requires the machine to be doing nothing else. Porting continues
  right up until that window opens.
  Consequence, and it is fine: modules sit complete-except-gate-10 for longer, so `scope.txt` lags
  reality. `scripts/status.sh` shows them as `pend` with `bench --`, which is the honest picture.
- **One agent at a time — superseding the earlier parallelism guidance.** Ten merges of parallel
  work cost **ten split-block repairs** (a match arm or test function halved, because git picks
  conflict boundaries by line similarity, not syntax) and **four** cases of two agents building the
  same machinery independently. Every split was caught by the compiler, none by `git`. Since going
  sequential: zero conflicts, zero repairs, and agents reuse what exists because they can see it.
  Wall-clock is rarely what binds here; orchestrator tokens are, and merge repair spends them.
  Isolated worktrees and additive-only shared-file edits still apply.
- **Stop-and-review before a design is inherited.** Applied to the cursor, to `forEach`, and now to
  `JsKey`.
- **Route by task shape, not by default.** Nine of the first ten agents ran on the session default
  (Opus) because no `model` was passed — including work that was pure template-following. Measured
  cost: Opus batches ran **440k–570k** tokens each; the one Sonnet agent did a contained
  protocol-reconciliation in **133k** and found *three* defects where one was specified.
  **Sonnet for template-following ports against an existing reference, doc writing and registry
  plumbing. Opus for a genuinely new capability tier.** The expensive design work — cursor,
  `forEach`, `JsKey`, PORTBUG-1 — is done, so most remaining work is the former.
- **Tier-first ordering compounds.** Merging `foreach` + `iterables` + `fixed-stack` took eight
  modules from blocked to available in a single step (`vector`, `static-interval-tree`, `bi-map`,
  `fuzzy-map`, `lru-cache`, `bk-tree`, `symspell`, `passjoin-index` — 1,659 test lines, 21% of the
  repo). Unblocking is worth more than the module it ships with.
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

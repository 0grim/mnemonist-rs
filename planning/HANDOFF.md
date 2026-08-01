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

`git log` is dense and deliberately so; the merge commits carry the reasoning.

## Position as of 2026-08-01 ~18:00 UTC

**77% ported, 0% scoped.** 655 Rust tests, 651 upstream harness specs, all green on `main`.

The 0% is not a mistake and not a crisis — see "Why 0% scoped" below. It is the single most
important thing to fix, and it is fixed by running benchmarks, not by porting more.

## The plan, agreed with the user

**Ports run ONE AGENT AT A TIME, never in parallel.** This was a deliberate change. Parallel agents
produced ten split-block repairs across ten merges, because git picks conflict boundaries by line
similarity and repeatedly cut the *previous* match arm or test function in half. Sequential agents
branch from a `main` that already contains all prior work, so conflicts drop to zero — and each
agent can *see* the previous ones' machinery instead of rebuilding it, which had already happened
four times. Wall-clock is not the binding constraint; orchestrator tokens are.

Remaining batches, in priority order so that stopping early stops at the best available point:

| # | batch | lines | bug IDs | status |
|---|---|---|---|---|
| 1 | `fibonacci-heap` + close D-105 | 115 | B-220–239 | **in flight** |
| 2 | `default-weak-map`, `linked-list`, `inverted-index` | 325 | B-240–259 | not started |
| 3 | `critbit-tree-map`, `fixed-critbit-tree-map` | 294 | B-260–279 | not started |
| 4 | `vp-tree`, `kd-tree` | 344 | B-280–299 | not started |
| 5 | `passjoin-index`, `symspell`, `multi-array` | 639 | B-300–319 | not started |

Bug-ID ranges are allocated by the orchestrator and must not overlap; the current high-water mark in
`NOTES.md` is B-201. Two agents once claimed the same range and it had to be untangled by hand.

**Benchmarks run in TWO passes, not one.** Pass A covers everything landed by tonight; pass B covers
whatever lands after. The tempting plan — port everything, benchmark once at the end — is cheaper
and fragile: if anything goes wrong in the final hours we submit with `scope.txt` empty and our own
gates reporting 0% done. Two passes cost a few hours and guarantee a floor.

Then: README, `DECISIONS.md` assembled from the 200-odd candidates, demo script, `.port-mortem.toml`,
final `verify.sh`. `DESIGN.md` §12 has the specs. **Reserve budget for this** — the write-up is what
judges read, and rigor is 50% of the score against coverage's 40%.

## Why 0% scoped

`tests/scope.txt` is the done marker and a unit enters it only when **all ten gates** pass. Gate 10
is the benchmark, which has never run, so nearly everything sits at `pend` with `bench --` no matter
how complete it otherwise is. **The benchmark pass is the conversion step from ported to counted.**

Gate 10 cannot be pipelined: a benchmark on a busy machine is not slow, it is *wrong* — a contended
run inflated both sides 2–3× here, upstream's own p99 swung 32% between clean runs, and a timing
test flaked under agent contention during the last batch. Run it when nothing else is running.

## Open items that must not be forgotten

- **`sparse-set` is deliberately descoped**, reason inline in `tests/scope.txt`. B-31 is fixed, but
  the `RefCell` added a borrow-flag check to every access that nobody has measured — which is gate
  10. Re-scope after benchmarking.
- **D-105** — `_utils`'s k-way tie-break disagrees with `FibonacciHeap`'s ordering on 3+ way ties.
  Batch 1 is closing this. Until it does, `_utils`'s 1M-op campaign is green over a region that
  *excludes* the known disagreement. If batch 1 fails to close it, that caveat stands and must be
  stated in the module doc — do not narrow the grammar again to get green.
- **D-201** — `trie`'s cursor-versus-delete divergence. **Accepted, not a defect.** Upstream's
  iterator holds a live object reference; ours is path-based because it must resume across the FFI
  boundary. Its campaign is likewise green over a narrowed region. State it in DECISIONS.md as an
  architectural divergence rather than revisiting it.
- **Ten stale worktree branches.** Delete after merging; branch names never reach the final repo.

## When a background agent dies

It has happened three times: the Claude Code process exits and takes its agents with it. Their
transcripts survive on disk.

1. **Preserve first.** `git -C .claude/worktrees/agent-<id> status --porcelain`; if anything is
   loose, commit it labelled **UNVERIFIED** — it was snapshotted by the orchestrator, not by the
   agent that wrote it, so nothing in it has been compiled or run. Never treat it as evidence.
2. **Resume, do not relaunch.** `SendMessage` to the agent id picks up from its transcript with its
   context intact, which is far cheaper than a fresh brief.
3. Tell it what you snapshotted and that it must re-verify anything that snapshot touched.

Agents briefed to "commit early and often" left zero loose files; agents without that line left 39
and 8. Keep the line in every brief.

## Traps that have already cost this project time

- **`cargo build` does not compile `#[cfg(test)]` blocks.** Run `cargo test`. Bitten repeatedly.
- **A pipeline's exit status is its last command's** — `cargo clippy | tail` reports `tail`'s
  success and shipped a red commit. Never pipe verification.
- **Merging: appending at the end of a shared list is necessary and not sufficient.** Ten split
  blocks across ten merges. The compiler catches them; `git` does not. After resolving, run
  `cargo test`, not `cargo build` — three compiled fine and only failed under test.
- **rust-analyzer diagnostics go stale mid-merge.** They reported conflict markers in files that
  had none. `grep -rln '^<<<<<<< '` is authoritative; a compile that succeeds is authoritative.
- **Route by task shape.** Sonnet has done every port since the heap tier at roughly a quarter the
  cost of Opus, and has found defects beyond what its briefs specified.

## The one idea worth not losing

**Ten times** a confident green signal turned out to be verifying something other than what was
believed. The table is in `NOTES.md`. The sharpest: a fuzz spec whose oracle-side names never
matched, so every case panicked at construction and the campaign reported zero divergences
*truthfully* — zero disagreements out of zero comparisons.

The countermeasure that emerged is now standard in every brief: **make the fuzzer prove it reached
the interesting state.** Measured eviction rates for the LRU, a prefix pool where 5/8 entries are a
strict prefix of another for the trie, multi-value-bucket counts for the MultiMaps. And gate 6 has
now caught itself once — `_utils` named three sabotages, two stayed green, and it reported them as
findings rather than swapping in an easier target.

That is the strongest material the write-up has: **passing your own verification is not the same as
being correct.**

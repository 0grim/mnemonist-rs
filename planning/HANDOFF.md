# HANDOFF — read this first if you are a new session

A Rust port of the JS library `mnemonist`, entered in Port Mortem 2026. Kickoff was
2026-07-31 18:00 UTC; **code freeze 2026-08-03 18:00 UTC**.

## Orientation, in this order

1. `CLAUDE.md` (repo root) — auto-loads. Working rules and the traps that have already cost time.
2. `scripts/status.sh` — **live derived state**. Coverage, per-unit evidence, in-flight worktrees,
   health. Nothing in it is hand-maintained, so nothing in it is stale.
3. `planning/ROADMAP.md` — what to do next and why. The governing arithmetic is at the top.
4. `planning/DESIGN.md` §1.1 — the **Definition of Done**. Ten gates. Everything else is detail.
5. `planning/NOTES.md` — upstream bug candidates and the capture log for the write-up.

`git log` is dense and deliberately so; commit messages carry the reasoning.

## Operational state that no file derives

**Three worktree branches hold half-finished units.** Their agents' process exited before they
finished, so each stopped at the same place: **core ported and napi bridged, then nothing**. No
divergence doc, no fuzz campaign, no falsification — so **none is close to the Definition of Done**,
and none may be added to `tests/scope.txt` on the strength of its port alone.

| branch suffix | units | state |
|---|---|---|
| `…a945bf7525bd6b1ea` | `vector`, `static-interval-tree` | core+napi committed; fuzz specs written but **never compiled or run** |
| `…a63029988a59ec3ef` | `bi-map`, `fuzzy-map`, `bk-tree` | as above, plus a `bi-map` proptest regression file whose meaning was never recorded |
| `…a1bcceb13cf54bcb9` | `lru-cache` family (4 modules, one unit) | core+napi committed, clean tree, **no fuzz spec at all** |

The `wip(fuzz)` commit on the first two was made by the orchestrator to stop loose work being lost.
It is explicitly unverified — treat it as a draft someone else left on the desk, not as evidence.

These branches also predate the last two merges, so they lack `heap`, `fixed-reverse-heap`, and the
`fuzz/oracle.js` array-encoding change. **Merge main into the branch before finishing the unit**,
or the encoding conflict below will resurface as a mystery.

## Next actions, in priority order

1. **Finish the three units, one agent per branch.** Each needs: fuzz spec compiling and a logged
   campaign, `docs/modules/<unit>.md` with all six sections, a recorded falsification, then merge.
   Do not merge them half-done — a unit in `scope.txt` without real evidence is the one thing
   `tests/verify.sh` exists to prevent.
2. **Run the benchmark pass.** `bench/run.sh <module>` per pending module, serially, on an
   **idle machine**. This is the highest-value item left and has never run.
3. **Add benched units to `tests/scope.txt`**, and re-scope `sparse-set` (see below).
4. **README, demo script, submission** — `DESIGN.md` §12b/§12e. Watch for the submission form
   around Aug 2.

If time runs short, prefer **finishing fewer units completely** over merging more of them partially.
Coverage is 40% of the score and proportional; rigor is 50% and is what the ten gates buy.

## Why "0% done" is not a mistake

`scope.txt` is the done marker and a unit enters it only when **all ten gates** pass. Gate 10 is the
benchmark, which has never been run, so almost everything sits at `pend` with `bench --` no matter
how complete it otherwise is. **The benchmark pass is the conversion step from ported to counted.**

Gate 10 cannot be pipelined: a benchmark taken on a busy machine is not slow, it is *wrong* — a
contended run inflated both sides 2–3× here, and upstream's own p99 swung 32% between clean runs.
Run it when nothing else is.

**`sparse-set` is deliberately descoped**, with the reason inline in `tests/scope.txt`. B-31 is
fixed, but the `RefCell` added a borrow-flag check to every access that nobody has measured — which
is gate 10. Re-scope it after benchmarking, not before.

## Traps that have already cost this project time

- **`cargo build` does not compile `#[cfg(test)]` blocks.** Run `cargo test`. Bitten repeatedly.
- **A pipeline's exit status is its last command's** — `cargo clippy | tail` reports `tail`'s
  success and shipped a red commit. Never pipe verification.
- **Merging: appending at the end of a shared list is necessary and not sufficient.** Git splits
  conflict boundaries by line similarity, so it repeatedly cut the *previous* match arm or test
  function in half, leaving both sides sharing its tail. Seven such repairs. The compiler catches
  them; `git` does not.
- **Parallel agents solve the same sub-problem twice** — four instances so far, the worst being a
  fuzz op built with incompatible signatures whose duplicate handlers landed in one JS `switch`,
  where the first silently wins and a syntax check passes.
- **Route by task shape.** Sonnet handles template-following ports, doc work and reconciliation at
  roughly a quarter the cost and has found defects beyond what was specified. Reserve Opus for a
  genuinely new capability tier.

## The one idea worth not losing

Seven separate times, a confident green signal turned out to be verifying something other than what
was believed — a falsification that sabotaged a branch no test takes, RSS measuring pages that were
never resident, a fuzz campaign replaying two saved seeds, a bridge bug no fuzzer could reach
because the fuzzer never exercised the bridge. The table is in `NOTES.md`, and it is the strongest
material the write-up has: **passing your own verification is not the same as being correct.**

# Working rules for this repository

A Rust port of the JavaScript library `mnemonist`, entered in the Port Mortem 2026 hackathon.
These rules apply to **every** agent working here. They exist because each one below has already
cost this project time.

## Never touch `tests/original/`

Those files are the upstream test suite, hashed at kickoff as the submission's parity commitment.
`sha256sum -c tests/SHA256SUMS --quiet` must PASS at every commit. If it ever fails, stop and
report — do not "fix" it.

## Verify before every commit — run these exactly

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
sha256sum -c tests/SHA256SUMS --quiet
./tests/verify.sh          # the full Definition of Done
```

**Two traps that have already shipped broken commits here:**

- **`cargo build` does not compile `#[cfg(test)]` blocks.** A green build says nothing about
  whether test code even parses. Run `cargo test`.
- **A pipeline's exit status is its *last* command's.** `cargo clippy | tail -5` reports `tail`'s
  success and hides a failing lint — this shipped a red commit. Do not pipe verification commands.
  If you must, check `${PIPESTATUS[0]}`.

More generally: **before trusting a check, ask what it would look like if the thing it checks were
broken.** `planning/NOTES.md` has a table of five separate occasions where a confident green
signal turned out to be verifying something other than what was believed.

## The Definition of Done

`planning/DESIGN.md` §1.1. Ten gates. A unit is **the require-closure of one upstream test file**,
not a source module — a missing sibling makes the whole file fail with zero partial credit.

`tests/scope.txt` is the done marker and must never list a unit whose evidence is not real.
`tests/verify.sh` enforces that; `scripts/status.sh` reports current state (derived, never
hand-maintained).

**Gate 6 — falsification — must be capable of failing.** Name the assertion your sabotage should
break *before* running it, confirm red, then confirm green after revert. A falsification that
stays green is just a second green light.

**Gate 10 — benchmarks — needs an idle machine.** A contended run inflated both sides 2–3× here.
If other agents are working, do not benchmark; gate 10 is batched into a quiet serial pass
(§7.3).

## Porting rules

- **Reproduce upstream bug-for-bug.** A fuzz "divergence" where our port is *more correct* is a bug
  in the port. Document divergences in `docs/modules/<unit>.md` and `planning/DECISIONS-CANDIDATES.md`.
- `mnemonist-core` keeps `#![forbid(unsafe_code)]` and a **zero-dependency tree**. It must build
  and test with Node absent. JS-value handling belongs in `mnemonist-napi`.
- **Do not overclaim causation**, especially about performance. Check an explanation against a
  metric that would falsify it, and label the unconfirmed as unconfirmed.

## Git

- **Commit as a series, never a dump.** Judges inspect history for genuine incrementality; a single
  large commit or predated work risks disqualification.
- **Never amend, rebase, squash or force-push.** Fix forward and say so in the message.
- Conventional commits: `feat(core):`, `feat(napi):`, `test(fuzz):`, `perf(bench):`, `docs(module):`,
  `fix(harness):`, `chore:`.
- **Branches: commit to the branch your worktree already has. Do not create additional branches.**
  Naming is handled at merge time by the orchestrator. Do not push unless told to.
- Edits to shared files — `crates/*/src/lib.rs`, `structures/mod.rs`, `utils/mod.rs`, the difffuzz
  registry, `fuzz/log.txt` — must be **additive only**. Never reorder or reformat them: every extra
  change becomes another agent's merge conflict.

## Orientation

| | |
|---|---|
| `planning/ROADMAP.md` | what we do next and why |
| `planning/DESIGN.md` | how everything works (§1.1 DoD, §3.x semantics, §7.x schedule) |
| `planning/NOTES.md` | raw capture log + upstream bug candidates |
| `planning/DECISIONS-CANDIDATES.md` | divergences feeding the submission's DECISIONS.md |
| `scripts/status.sh` | live derived status |
| `~/upstream-mnemonist` | upstream source at the pinned commit — **port from the real file** |

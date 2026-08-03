//! Campaign driver: turns a [`ModuleSpec`] into a timed proptest run.
//!
//! `docs/METHODOLOGY.md`'s gate 9 is explicit that generation *and shrinking* are proptest's
//! job, not ours — shrinking is the expensive thing to write and the valuable
//! thing to have, since it is what turns a 4,000-op divergence into a repro
//! small enough to paste into an upstream issue. So this module builds a
//! [`TestRunner`] and gets out of the way.
//!
//! Two things it does add on top of the `proptest!` macro:
//!
//! 1. **An explicit, loggable seed.** The macro seeds itself from entropy and
//!    never tells you what it used, but gate 9 wants a seed in `fuzz/log.txt`.
//!    Driving `TestRunner` by hand with `TestRng::from_seed` gives a run that
//!    can be reproduced exactly by `--seed N --cases M`.
//! 2. **A wall-clock budget.** Gate 9 is specified in seconds, not cases, and
//!    how many cases fit in 60 seconds is not knowable up front.
//!
//! Failure persistence still points at `proptest-regressions/`, so a divergence
//! is recorded there by proptest itself exactly as it would be under the macro.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use proptest::collection::{self, SizeRange};
use proptest::strategy::{Just, Strategy};
use proptest::test_runner::{
    Config, FileFailurePersistence, RngAlgorithm, TestCaseError, TestError, TestRng, TestRunner,
};

use crate::spec::{check_program, CheckFailure, Divergence, ModuleSpec, Program};
use crate::{Oracle, OracleError};

/// How long and how hard to fuzz.
#[derive(Debug, Clone)]
pub struct Campaign {
    /// Seed for the whole run. Logged, and sufficient to reproduce it given
    /// the same case count.
    pub seed: u64,
    /// Stop after this many executed cases. `None` means "until the deadline".
    pub cases: Option<u64>,
    /// Stop after this much wall clock. `None` means "until the case budget".
    pub duration: Option<Duration>,
    /// Cases per `TestRunner::run` call. Only affects deadline granularity.
    pub batch: u32,
    /// Where proptest records a minimised failing seed.
    pub regressions: &'static str,
}

impl Campaign {
    /// A campaign bounded by cases alone — what `cargo test` uses.
    pub fn cases(seed: u64, cases: u64, regressions: &'static str) -> Self {
        Self {
            seed,
            cases: Some(cases),
            duration: None,
            batch: 32,
            regressions,
        }
    }
}

/// What a finished campaign is worth reporting.
#[derive(Debug, Clone)]
pub struct Report {
    pub module: &'static str,
    pub seed: u64,
    pub cases: u64,
    pub ops: u64,
    pub elapsed: Duration,
    /// The first divergence found, minimised by proptest. `None` is a pass.
    pub divergence: Option<Divergence>,
}

impl Report {
    pub fn divergences(&self) -> usize {
        usize::from(self.divergence.is_some())
    }

    /// One-line summary in the shape `fuzz/log.txt` records.
    pub fn log_line(&self) -> String {
        format!(
            "module={} seed={} cases={} ops={} wall={:.1}s divergences={}",
            self.module,
            self.seed,
            self.cases,
            self.ops,
            self.elapsed.as_secs_f64(),
            self.divergences(),
        )
    }
}

/// Run a campaign to completion.
///
/// `Err` means the apparatus failed (no `node`, dead pipe, malformed
/// response). A divergence is a successful measurement and comes back in
/// [`Report::divergence`] — conflating the two would let a broken oracle be
/// reported as "zero divergences".
pub fn run<S: ModuleSpec>(spec: &S, campaign: &Campaign) -> Result<Report, OracleError> {
    let mut oracle = Oracle::spawn(&Oracle::default_script())?;
    let report = run_with(spec, campaign, &mut oracle);
    let _ = oracle.shutdown();

    report
}

/// As [`run`], against an oracle the caller owns.
pub fn run_with<S: ModuleSpec>(
    spec: &S,
    campaign: &Campaign,
    oracle: &mut Oracle,
) -> Result<Report, OracleError> {
    // Two ways to build a campaign that quietly does nothing, both rejected
    // here rather than in the CLI, because `Campaign`'s fields are public and
    // a test harness can construct one directly.
    //
    // `batch == 0` is the nastier of the two: proptest's runner loops
    // `while successes < cases`, so a zero case count returns Ok without ever
    // calling the property. In duration mode that yields a 60-second run
    // reporting "zero divergences" over zero work; in cases mode the outer
    // loop spins forever, because `executed` can never reach the budget.
    if campaign.batch == 0 {
        return Err(OracleError::Protocol(
            "campaign batch is 0, which would run no cases at all".into(),
        ));
    }

    if campaign.cases.is_none() && campaign.duration.is_none() {
        return Err(OracleError::Protocol(
            "campaign has neither a case budget nor a deadline, so it would never stop".into(),
        ));
    }

    let strategy = program_strategy(spec);

    let config = Config {
        cases: campaign.batch,
        // proptest's default of 1024 shrink iterations is tuned for small
        // generated values. A program here is up to 600 ops, and shrinking a
        // 600-op sequence to the 3 ops that actually matter needs far more
        // steps than that — measured: the default stopped at a 29-op "minimal"
        // case that was still mostly noise. The time budget is the real bound.
        max_shrink_iters: 1 << 22,
        max_shrink_time: 120_000,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            campaign.regressions,
        ))),
        ..Config::default()
    };

    let started = Instant::now();
    let deadline = campaign.duration.map(|budget| started + budget);

    let oracle = RefCell::new(oracle);
    let broken = RefCell::new(None::<OracleError>);
    let executed = Cell::new(0u64);
    let ops = Cell::new(0u64);

    let mut divergence = None;
    let mut batch = 0u64;

    loop {
        // A FRESH runner per batch, and this is not tidiness.
        //
        // `TestRunner` counts successes for its whole lifetime and its run
        // loop is `while self.successes < self.config.cases`. So a *second*
        // `run` on the same runner executes **no new cases at all** — it
        // returns `Ok` immediately, and the loop below spins at 100% CPU until
        // the deadline.
        //
        // That is exactly what happened, and it is worse than a wasted run,
        // because the campaign still reports a case count: the only thing a
        // later batch still executes is the *persisted regression corpus*,
        // which proptest replays before the (empty) main loop. So a 120-second
        // campaign booked tens of thousands of "cases" that were two saved
        // seeds re-run tens of thousands of times, and reported it as coverage.
        // Measured: with the corpus file removed, a 120-second run drops to 32
        // cases — the first batch and nothing else.
        //
        // Seeding is `(campaign.seed, batch)` rather than `campaign.seed`
        // alone so a replay of `--seed N --cases M` is still exact while
        // successive batches explore genuinely new programs.
        let mut runner = TestRunner::new_with_rng(config.clone(), seeded_rng(campaign.seed, batch));

        batch += 1;

        let outcome = runner.run(&strategy, |program: Program| {
            // Past the budget: drain the rest of the batch without work, so
            // the deadline is honoured to within one case rather than one
            // batch.
            if broken.borrow().is_some() || over_budget(&executed, campaign, deadline) {
                return Ok(());
            }

            let mut held = oracle.borrow_mut();

            match check_program(spec, &mut held, &program) {
                Ok(count) => {
                    executed.set(executed.get() + 1);
                    ops.set(ops.get() + count);
                    Ok(())
                }
                Err(CheckFailure::Diverged(found)) => {
                    executed.set(executed.get() + 1);
                    Err(TestCaseError::fail(found.to_string()))
                }
                Err(CheckFailure::Oracle(error)) => {
                    // Not a divergence. Park it and let the batch unwind; the
                    // loop below turns it into an `Err` for the caller.
                    *broken.borrow_mut() = Some(error);
                    Ok(())
                }
            }
        });

        if let Some(error) = broken.borrow_mut().take() {
            return Err(error);
        }

        match outcome {
            Ok(()) => {}
            Err(TestError::Fail(_, program)) => {
                // `program` is proptest's minimised counterexample. Re-run it
                // to recover the structured divergence rather than parsing the
                // rendered message back out.
                let mut held = oracle.borrow_mut();

                divergence = match check_program(spec, &mut held, &program) {
                    Err(CheckFailure::Diverged(found)) => Some(*found),
                    Err(CheckFailure::Oracle(error)) => return Err(error),
                    // proptest has already told us this program failed; the
                    // re-run exists only to recover the structured value,
                    // because proptest hands back a rendered string.
                    //
                    // If the re-run passes, something is wrong that must not
                    // be rounded down to `None` -- that would turn a detected
                    // failure into a clean report, which is the exact failure
                    // mode this crate is built to avoid. Non-determinism in a
                    // deterministic structure would itself be the finding.
                    Ok(_) => {
                        return Err(OracleError::Protocol(format!(
                            "proptest reported a failing case that does not reproduce. \
                             The disagreement is not deterministic, which is itself a \
                             finding, and it is NOT being reported as clean. Program:\n{}",
                            program.render(spec.module(), spec.functions())
                        )));
                    }
                };
                drop(held);
                break;
            }
            Err(TestError::Abort(reason)) => {
                return Err(OracleError::Protocol(format!(
                    "proptest aborted the run: {reason}"
                )));
            }
        }

        if over_budget(&executed, campaign, deadline) {
            break;
        }
    }

    Ok(Report {
        module: spec.module(),
        seed: campaign.seed,
        cases: executed.get(),
        ops: ops.get(),
        elapsed: started.elapsed(),
        divergence,
    })
}

fn over_budget(executed: &Cell<u64>, campaign: &Campaign, deadline: Option<Instant>) -> bool {
    if let Some(limit) = campaign.cases {
        if executed.get() >= limit {
            return true;
        }
    }

    if let Some(deadline) = deadline {
        if Instant::now() >= deadline {
            return true;
        }
    }

    false
}

/// Expand `(seed, batch)` into the 32 bytes ChaCha wants, without pulling in a
/// hasher.
///
/// Only needs to be injective and stable across runs; it is a seed, not a
/// random number. `batch` is stirred in rather than added to `seed` so that
/// `--seed 42`'s second batch is not `--seed 43`'s first — overlapping streams
/// between campaigns would quietly re-test the same programs.
fn seeded_rng(seed: u64, batch: u64) -> TestRng {
    let mut bytes = [0u8; 32];

    for (chunk, block) in bytes.chunks_mut(8).enumerate() {
        // Splitmix-style stir so `--seed 1` and `--seed 2` do not produce
        // 31 identical bytes.
        let mixed = seed
            .wrapping_add(batch.wrapping_mul(0xD6E8_FEB8_6659_FD93))
            .wrapping_add((chunk as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
            .wrapping_mul(0xBF58_476D_1CE4_E5B9);

        block.copy_from_slice(&mixed.to_le_bytes());
    }

    TestRng::from_seed(RngAlgorithm::ChaCha, &bytes)
}

/// `ctor` first, then an op sequence generated against it.
///
/// The flat-map is load-bearing: `union(x, y)` cannot generate a valid `x`
/// until the set's size has been chosen, and proptest shrinks the size and the
/// ops together because of it.
fn program_strategy<'a, S: ModuleSpec>(spec: &'a S) -> impl Strategy<Value = Program> + 'a {
    let length: SizeRange = spec.program_len().into();

    spec.ctor_strategy()
        .prop_flat_map(move |ctor| {
            let ops = collection::vec(spec.op_strategy(&ctor), length.clone());

            (Just(ctor), ops)
        })
        .prop_map(|(ctor, ops)| Program { ctor, ops })
}

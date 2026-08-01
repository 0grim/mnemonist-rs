//! Campaign CLI.
//!
//! ```text
//! difffuzz --module static-disjoint-set --seed 42 --duration 60
//! difffuzz --module static-disjoint-set --seed 42 --cases 500     # exact replay
//! ```
//!
//! stdout is one machine-readable summary line, in the shape `fuzz/log.txt`
//! records; the human-facing narration and any divergence go to stderr. That
//! split is what lets `fuzz/run.sh` timestamp the line and append it without
//! parsing anything.
//!
//! Exit codes: `0` clean, `1` divergence found, `2` the harness broke. The
//! third is separate on purpose — "no node on PATH" must never be reportable as
//! a clean fuzz run.

use std::process::ExitCode;
use std::time::Duration;

use difffuzz::modules::bit_set::BitSetSpec;
use difffuzz::modules::bit_vector::BitVectorSpec;
use difffuzz::modules::hashed_array_tree::HashedArrayTreeSpec;
use difffuzz::modules::sparse_set::SparseSetSpec;
use difffuzz::modules::static_disjoint_set::StaticDisjointSetSpec;
use difffuzz::{Campaign, Report};

const USAGE: &str = "\
usage: difffuzz --module <name> [--seed N] [--duration SECONDS] [--cases N] [--batch N]

  --module    module to fuzz; currently: static-disjoint-set, sparse-set,
              hashed-array-tree, bit-set, bit-vector
  --seed      campaign seed (default 42); with --cases, reproduces exactly
  --duration  wall-clock budget in seconds (default 60)
  --cases     stop after N cases instead of after --duration
  --batch     cases per proptest invocation (default 32)
";

fn main() -> ExitCode {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("difffuzz: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let campaign = Campaign {
        seed: options.seed,
        cases: options.cases,
        // A case budget replaces the clock rather than capping it, so a replay
        // is not cut short on a slower machine.
        duration: options
            .cases
            .is_none()
            .then(|| Duration::from_secs(options.duration)),
        batch: options.batch,
        // Overwritten per module below: each keeps its own regression corpus,
        // so one module's minimised seed is never replayed against another's
        // grammar, where it would decode into a different program.
        regressions: "",
    };

    let report = match options.module.as_str() {
        "static-disjoint-set" => difffuzz::run(
            &StaticDisjointSetSpec,
            &Campaign {
                regressions: difffuzz::modules::static_disjoint_set::REGRESSIONS,
                ..campaign
            },
        ),
        "sparse-set" => difffuzz::run(
            &SparseSetSpec,
            &Campaign {
                regressions: difffuzz::modules::sparse_set::REGRESSIONS,
                ..campaign
            },
        ),
        "hashed-array-tree" => difffuzz::run(
            &HashedArrayTreeSpec,
            &Campaign {
                regressions: difffuzz::modules::hashed_array_tree::REGRESSIONS,
                ..campaign
            },
        ),
        "bit-set" => difffuzz::run(
            &BitSetSpec,
            &Campaign {
                regressions: difffuzz::modules::bit_set::REGRESSIONS,
                ..campaign
            },
        ),
        "bit-vector" => difffuzz::run(
            &BitVectorSpec,
            &Campaign {
                regressions: difffuzz::modules::bit_vector::REGRESSIONS,
                ..campaign
            },
        ),
        other => {
            eprintln!("difffuzz: unknown module `{other}`\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match report {
        Ok(report) => finish(&report),
        Err(error) => {
            eprintln!("difffuzz: harness failure, NOT a divergence: {error}");
            ExitCode::from(2)
        }
    }
}

fn finish(report: &Report) -> ExitCode {
    println!("{}", report.log_line());

    match &report.divergence {
        // A campaign that executed no operations is not clean, it is empty --
        // and "zero divergences" is exactly the wrong thing to print about it.
        // Reachable through a mistyped --cases, a grammar that generates
        // nothing, or a deadline shorter than one case.
        None if report.ops == 0 => {
            eprintln!(
                "difffuzz: campaign ran {} cases and ZERO operations, so it proved nothing. \
                 Not reporting this as clean.",
                report.cases
            );
            ExitCode::from(2)
        }
        None => {
            eprintln!(
                "clean: {} cases, {} ops, {:.1}s, zero divergences",
                report.cases,
                report.ops,
                report.elapsed.as_secs_f64()
            );
            ExitCode::SUCCESS
        }
        Some(divergence) => {
            eprintln!("DIVERGENCE (minimised by proptest)\n{divergence}");
            ExitCode::from(1)
        }
    }
}

struct Options {
    module: String,
    seed: u64,
    duration: u64,
    cases: Option<u64>,
    batch: u32,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut options = Self {
            module: String::new(),
            seed: 42,
            duration: 60,
            cases: None,
            batch: 32,
        };

        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--module" => options.module = value(&mut args, &flag)?,
                "--seed" => options.seed = number(&mut args, &flag)?,
                "--duration" => options.duration = number(&mut args, &flag)?,
                "--cases" => options.cases = Some(number(&mut args, &flag)?),
                "--batch" => options.batch = number(&mut args, &flag)?,
                "-h" | "--help" => return Err("help requested".into()),
                other => return Err(format!("unknown flag `{other}`")),
            }
        }

        if options.module.is_empty() {
            return Err("--module is required".into());
        }

        // Caught again in `run_with`, but rejecting it here gives the operator
        // the flag name rather than a protocol error.
        if options.batch == 0 {
            return Err("--batch 0 would run no cases at all".into());
        }

        if options.cases == Some(0) {
            return Err("--cases 0 would run no cases at all".into());
        }

        Ok(options)
    }
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("`{flag}` needs a value"))
}

fn number<T: std::str::FromStr>(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<T, String> {
    let raw = value(args, flag)?;

    raw.parse()
        .map_err(|_| format!("`{flag}` expects a number, got `{raw}`"))
}

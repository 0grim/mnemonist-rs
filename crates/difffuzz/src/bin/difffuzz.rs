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
use difffuzz::modules::default_map::DefaultMapSpec;
use difffuzz::modules::hashed_array_tree::HashedArrayTreeSpec;
use difffuzz::modules::queue::QueueSpec;
use difffuzz::modules::sparse_map::SparseMapSpec;
use difffuzz::modules::sparse_queue_set::SparseQueueSetSpec;
use difffuzz::modules::sparse_set::SparseSetSpec;
use difffuzz::modules::stack::StackSpec;
use difffuzz::modules::static_disjoint_set::StaticDisjointSetSpec;
// Appended at the end of the import run, never inserted.
use difffuzz::modules::circular_buffer::CircularBufferSpec;
use difffuzz::modules::fixed_deque::FixedDequeSpec;
use difffuzz::modules::fixed_stack::FixedStackSpec;
// Appended rather than filed alphabetically: this list is edited from several
// worktrees at once, and a conflict boundary that lands inside it has already
// broken three merges. New modules go on the end.
use difffuzz::modules::set::SetSpec;
use difffuzz::modules::sort::SortSpec;
// Appended, never interleaved (CLAUDE.md, Git).
use difffuzz::modules::bloom_filter::BloomFilterSpec;
use difffuzz::modules::suffix_array::{GeneralizedSuffixArraySpec, SuffixArraySpec};
use difffuzz::{Campaign, Report};
// Appended at the END of the import run, never inserted: this file is edited
// from several worktrees at once (CLAUDE.md, Git).
use difffuzz::modules::static_interval_tree::StaticIntervalTreeSpec;
use difffuzz::modules::vector::VectorSpec;

const USAGE: &str = "\
usage: difffuzz --module <name> [--seed N] [--duration SECONDS] [--cases N] [--batch N]

  --module    module to fuzz; currently: static-disjoint-set, sparse-set,
              sparse-map, sparse-queue-set, hashed-array-tree, bit-set,
              bit-vector, stack, queue, default-map, fixed-stack,
              fixed-deque, circular-buffer, sort, set, suffix-array,
              generalized-suffix-array, bloom-filter, vector,
              static-interval-tree
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
        "sparse-map" => difffuzz::run(
            &SparseMapSpec,
            &Campaign {
                regressions: difffuzz::modules::sparse_map::REGRESSIONS,
                ..campaign
            },
        ),
        "sparse-queue-set" => difffuzz::run(
            &SparseQueueSetSpec,
            &Campaign {
                regressions: difffuzz::modules::sparse_queue_set::REGRESSIONS,
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
        "stack" => difffuzz::run(
            &StackSpec,
            &Campaign {
                regressions: difffuzz::modules::stack::REGRESSIONS,
                ..campaign
            },
        ),
        "queue" => difffuzz::run(
            &QueueSpec,
            &Campaign {
                regressions: difffuzz::modules::queue::REGRESSIONS,
                ..campaign
            },
        ),
        "default-map" => difffuzz::run(
            &DefaultMapSpec,
            &Campaign {
                regressions: difffuzz::modules::default_map::REGRESSIONS,
                ..campaign
            },
        ),
        // Appended immediately before the fallback arm, never inserted into
        // the run above: a new arm at the end of the list cannot land inside
        // another agent's hunk.
        "fixed-stack" => difffuzz::run(
            &FixedStackSpec,
            &Campaign {
                regressions: difffuzz::modules::fixed_stack::REGRESSIONS,
                ..campaign
            },
        ),
        "fixed-deque" => difffuzz::run(
            &FixedDequeSpec,
            &Campaign {
                regressions: difffuzz::modules::fixed_deque::REGRESSIONS,
                ..campaign
            },
        ),
        "circular-buffer" => difffuzz::run(
            &CircularBufferSpec,
            &Campaign {
                regressions: difffuzz::modules::circular_buffer::REGRESSIONS,
                ..campaign
            },
        ),
        // New modules go on the end of this match, never in the middle: a
        // conflict boundary landing inside an arm has already broken three
        // merges.
        "sort" => difffuzz::run(
            &SortSpec,
            &Campaign {
                regressions: difffuzz::modules::sort::REGRESSIONS,
                ..campaign
            },
        ),
        "set" => difffuzz::run(
            &SetSpec,
            &Campaign {
                regressions: difffuzz::modules::set::REGRESSIONS,
                ..campaign
            },
        ),
        // Appended at the END of the module arms, never between them: a merge
        // conflict boundary inside a match arm has broken this tree before.
        "suffix-array" => difffuzz::run(
            &SuffixArraySpec,
            &Campaign {
                regressions: difffuzz::modules::suffix_array::REGRESSIONS,
                ..campaign
            },
        ),
        "generalized-suffix-array" => difffuzz::run(
            &GeneralizedSuffixArraySpec,
            &Campaign {
                regressions: difffuzz::modules::suffix_array::GENERALIZED_REGRESSIONS,
                ..campaign
            },
        ),
        "bloom-filter" => difffuzz::run(
            &BloomFilterSpec,
            &Campaign {
                regressions: difffuzz::modules::bloom_filter::REGRESSIONS,
                ..campaign
            },
        ),
        // Appended at the END of the match, never between arms: a conflict
        // boundary landing inside one has already broken three merges
        // (CLAUDE.md, Git).
        "vector" => difffuzz::run(
            &VectorSpec,
            &Campaign {
                regressions: difffuzz::modules::vector::REGRESSIONS,
                ..campaign
            },
        ),
        "static-interval-tree" => difffuzz::run(
            &StaticIntervalTreeSpec,
            &Campaign {
                regressions: difffuzz::modules::static_interval_tree::REGRESSIONS,
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

//! Rust half of the matched benchmark harness (DESIGN.md 5.1–5.2).
//!
//! Deliberately *not* criterion. §5.2 Problem 1: criterion has no Node
//! counterpart, so a criterion-vs-hand-rolled-loop table is two methodologies
//! in one grid and a judge who notices discounts every row. This binary and
//! `bench/node/run.js` are written the same way — same warmup count, same
//! measured count, same batch size, same monotonic clock semantics — and the
//! percentile maths happens once, in the shared driver, over samples from both.
//! Criterion remains the right tool for Rust-only regression tracking; it just
//! stays out of the comparison.
//!
//! Also per §5.1: this links `mnemonist-core` directly. Never through N-API —
//! bridge overhead would poison the comparison and misrepresent the port.
//!
//! Modes:
//!
//! ```text
//! bench-runner --module static-disjoint-set --warmup 3 --measured 1
//! bench-runner --module sparse-set --kind drain --passes 100
//! bench-runner --baseline        # no-op run, reports peak RSS only
//! bench-runner --noop            # startup floor, for hyperfine
//! bench-runner --dump-prng 1000  # matched-PRNG proof
//! ```
//!
//! `--kind` selects the loop within a module. `mixed` is the op-stream
//! workload every module has; `sparse-set` adds `drain`, which measures
//! iteration and is the only benchmark that puts the cursor machinery of
//! DESIGN.md 3.4 on the clock.

mod rss;
mod sparse_set;
mod static_disjoint_set;
mod workload;
mod xorshift;

use std::process::ExitCode;

use serde_json::json;

/// Ops per timed sample. See `workload.rs` for why batching is not optional.
const BATCH_K: usize = 1000;

const DEFAULT_SIZE: u32 = 1_000_000;
const DEFAULT_OPS: usize = 1_000_000;
const DEFAULT_SEED: u32 = 42;

const USAGE: &str = "\
usage: bench-runner [--module NAME] [--kind mixed|drain] [--warmup N] [--measured N]
                    [--size N] [--ops N] [--passes N] [--seed N]
       bench-runner --baseline | --noop | --dump-prng N

modules: static-disjoint-set (mixed), sparse-set (mixed, drain)
";

/// Modules this binary can benchmark, and which loops each offers.
const MODULES: &[(&str, &[&str])] = &[
    ("static-disjoint-set", &["mixed"]),
    ("sparse-set", &["mixed", "drain"]),
];

/// Drain passes, when `--passes` is not given. See `sparse_set::run_drain`:
/// one timed sample per pass, so this is also the sample count per measured
/// run, and 100 is the floor for a meaningful p99.
const DEFAULT_PASSES: usize = 100;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flags: Vec<&str> = args.iter().map(String::as_str).collect();

    // Startup floor for hyperfine: parse nothing, do nothing, exit. The Node
    // twin does the same, so the pair measures runtime boot and nothing else.
    if flags.contains(&"--noop") {
        return ExitCode::SUCCESS;
    }

    if let Some(count) = value(&flags, "--dump-prng") {
        let count: usize = match count.parse() {
            Ok(count) => count,
            Err(_) => return fail("--dump-prng expects a number"),
        };
        let mut rng = xorshift::XorShift32::new(DEFAULT_SEED);

        for _ in 0..count {
            println!("{}", rng.next());
        }

        return ExitCode::SUCCESS;
    }

    if flags.contains(&"--baseline") {
        // The RSS baseline (§5.2 Problem 3). Node carries ~40 MB of V8 before
        // any data structure exists, and reporting that as a data-structure
        // result is the memory equivalent of claiming process startup as a
        // throughput win.
        println!(
            "{}",
            json!({"side": "port", "mode": "baseline", "rss_kb": rss::peak_kb()})
        );

        return ExitCode::SUCCESS;
    }

    let module = value(&flags, "--module").unwrap_or("static-disjoint-set");
    let kind = value(&flags, "--kind").unwrap_or("mixed");

    match MODULES.iter().find(|(name, _)| *name == module) {
        None => return fail(&format!("unknown module `{module}`")),
        Some((_, kinds)) if !kinds.contains(&kind) => {
            return fail(&format!("module `{module}` has no `{kind}` workload"))
        }
        Some(_) => {}
    }

    if flags.contains(&"--structure") {
        // Isolates the structure's own footprint: build it, touch nothing
        // else, report peak RSS. The mixed workload's RSS delta is dominated
        // by the materialised op arrays (~9 MB, identical on both sides),
        // which hides the part of the memory story that is actually about the
        // port -- and on this module that part is a regression, so hiding it
        // would be exactly the thing DESIGN.md 5.1 warns against.
        let size = match size_flag(&flags) {
            Ok(size) => size,
            Err(message) => return fail(&message),
        };
        // Touch the structure so nothing can be deferred or elided, mirroring
        // the Node twin's `set.parents[size - 1]` / `set.dense[size - 1]`.
        match module {
            "sparse-set" => {
                let set = mnemonist_core::structures::sparse_set::SparseSet::new(size as usize)
                    .expect("benchmark sizes are well inside the pointer limit");

                std::hint::black_box(&set);
            }
            _ => {
                let set = mnemonist_core::structures::static_disjoint_set::StaticDisjointSet::new(
                    size as usize,
                )
                .expect("benchmark sizes are well inside the pointer limit");

                std::hint::black_box(&set);
            }
        }

        println!(
            "{}",
            json!({"side": "port", "mode": "structure", "size": size, "rss_kb": rss::peak_kb()})
        );

        return ExitCode::SUCCESS;
    }

    let warmup = number(&flags, "--warmup", 3);
    let measured = number(&flags, "--measured", 1);
    let ops = number(&flags, "--ops", DEFAULT_OPS);

    let (size, seed) = match (size_flag(&flags), u32_flag(&flags, "--seed", DEFAULT_SEED)) {
        (Ok(size), Ok(seed)) => (size, seed),
        (Err(message), _) | (_, Err(message)) => return fail(&message),
    };

    let passes = number(&flags, "--passes", DEFAULT_PASSES);

    if kind == "drain" {
        return drain(module, size, seed, passes, warmup, measured);
    }

    let generated = workload::generate(size, ops, seed);

    // Warmup is mandatory for the Node side (V8 JIT) and kept here purely for
    // symmetry: measuring a cold JS run against an optimised Rust one is a
    // dishonest win, and the fix is the same protocol on both sides, not a
    // protocol tuned per runtime.
    let mut checksum = 0;

    let run = |workload: &workload::Workload| match module {
        "sparse-set" => sparse_set::run_mixed(workload, BATCH_K),
        _ => static_disjoint_set::run_once(workload, BATCH_K),
    };

    for _ in 0..warmup {
        let (_, sum) = run(&generated);
        checksum = sum;
    }

    let mut batches = Vec::new();

    for _ in 0..measured {
        let (times, sum) = run(&generated);

        if sum != checksum && warmup > 0 {
            return fail("checksum changed between passes; the workload is not deterministic");
        }

        checksum = sum;
        batches.extend(times);
    }

    println!(
        "{}",
        json!({
            "side": "port",
            "module": module,
            "kind": kind,
            "size": size,
            "ops": ops,
            "seed": seed,
            "batch_k": BATCH_K,
            "warmup": warmup,
            "measured": measured,
            "checksum": checksum,
            "batch_ns": batches,
            "rss_kb": rss::peak_kb(),
        })
    );

    ExitCode::SUCCESS
}

/// The iteration workload. Reported through the same envelope as the mixed
/// one, with `batch_k` carrying members-per-pass instead of a fixed 1000, so
/// the driver's `ns / batch_k` still means nanoseconds per element.
fn drain(
    module: &str,
    size: u32,
    seed: u32,
    passes: usize,
    warmup: usize,
    measured: usize,
) -> ExitCode {
    if passes == 0 {
        return fail("`--passes` must be at least 1");
    }

    let mut checksum = 0;
    let mut per_pass = 0;

    for _ in 0..warmup {
        let (_, sum, size) = sparse_set::run_drain(size, seed, passes);
        checksum = sum;
        per_pass = size;
    }

    let mut batches = Vec::new();

    for _ in 0..measured {
        let (times, sum, members) = sparse_set::run_drain(size, seed, passes);

        if sum != checksum && warmup > 0 {
            return fail("checksum changed between passes; the workload is not deterministic");
        }

        checksum = sum;
        per_pass = members;
        batches.extend(times);
    }

    if per_pass == 0 {
        // A drain over an empty set would divide by zero in the driver and
        // report an infinite rate. There is no honest measurement here.
        return fail("the prefilled set is empty, so there is nothing to drain");
    }

    println!(
        "{}",
        json!({
            "side": "port",
            "module": module,
            "kind": "drain",
            "size": size,
            "ops": per_pass * passes * measured,
            "seed": seed,
            "batch_k": per_pass,
            "passes": passes,
            "warmup": warmup,
            "measured": measured,
            "checksum": checksum,
            "batch_ns": batches,
            "rss_kb": rss::peak_kb(),
        })
    );

    ExitCode::SUCCESS
}

fn value<'a>(flags: &[&'a str], name: &str) -> Option<&'a str> {
    flags
        .iter()
        .position(|flag| *flag == name)
        .and_then(|at| flags.get(at + 1))
        .copied()
}

fn number(flags: &[&str], name: &str, fallback: usize) -> usize {
    value(flags, name)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(fallback)
}

/// Parse into `u32` directly rather than into `usize` and casting.
///
/// `--size 4294967296` truncating to 0 would be reported in the results JSON
/// as a real measurement of a size nobody asked for. A benchmark that
/// silently measures the wrong thing is worse than one that refuses to run.
fn u32_flag(flags: &[&str], name: &str, fallback: u32) -> Result<u32, String> {
    match value(flags, name) {
        None => Ok(fallback),
        Some(raw) => raw
            .parse::<u32>()
            .map_err(|_| format!("`{name}` expects an integer in 1..=4294967295, got `{raw}`")),
    }
}

/// `--size`, with zero rejected.
///
/// Zero is the one value where the two runners genuinely disagree: the Rust
/// PRNG's `next() % 0` panics, while JS gives `NaN`, which coerces to 0 on the
/// way into a typed array and yields a plausible-looking all-zero workload. So
/// one side fails loudly and the other reports a meaningless success. Neither
/// is acceptable, and the only honest workload of size 0 is no workload.
fn size_flag(flags: &[&str]) -> Result<u32, String> {
    let size = u32_flag(flags, "--size", DEFAULT_SIZE)?;

    if size == 0 {
        return Err(String::from("`--size` must be at least 1"));
    }

    Ok(size)
}

fn fail(message: &str) -> ExitCode {
    eprintln!("bench-runner: {message}\n\n{USAGE}");

    ExitCode::from(2)
}

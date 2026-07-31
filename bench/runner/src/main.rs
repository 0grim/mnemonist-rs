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
//! bench-runner --baseline        # no-op run, reports peak RSS only
//! bench-runner --noop            # startup floor, for hyperfine
//! bench-runner --dump-prng 1000  # matched-PRNG proof
//! ```

mod rss;
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
usage: bench-runner [--module NAME] [--warmup N] [--measured N] [--size N] [--ops N] [--seed N]
       bench-runner --baseline | --noop | --dump-prng N
";

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

    if module != "static-disjoint-set" {
        return fail(&format!("unknown module `{module}`"));
    }

    if flags.contains(&"--structure") {
        // Isolates the structure's own footprint: build it, touch nothing
        // else, report peak RSS. The mixed workload's RSS delta is dominated
        // by the materialised op arrays (~9 MB, identical on both sides),
        // which hides the part of the memory story that is actually about the
        // port -- and on this module that part is a regression, so hiding it
        // would be exactly the thing DESIGN.md 5.1 warns against.
        let size = number(&flags, "--size", DEFAULT_SIZE as usize) as u32;
        let set =
            mnemonist_core::structures::static_disjoint_set::StaticDisjointSet::new(size as usize)
                .expect("benchmark sizes are well inside the pointer limit");

        std::hint::black_box(&set);

        println!(
            "{}",
            json!({"side": "port", "mode": "structure", "size": size, "rss_kb": rss::peak_kb()})
        );

        return ExitCode::SUCCESS;
    }

    let warmup = number(&flags, "--warmup", 3);
    let measured = number(&flags, "--measured", 1);
    let size = number(&flags, "--size", DEFAULT_SIZE as usize) as u32;
    let ops = number(&flags, "--ops", DEFAULT_OPS);
    let seed = number(&flags, "--seed", DEFAULT_SEED as usize) as u32;

    let generated = workload::generate(size, ops, seed);

    // Warmup is mandatory for the Node side (V8 JIT) and kept here purely for
    // symmetry: measuring a cold JS run against an optimised Rust one is a
    // dishonest win, and the fix is the same protocol on both sides, not a
    // protocol tuned per runtime.
    let mut checksum = 0;

    for _ in 0..warmup {
        let (_, sum) = workload::run_once(&generated, BATCH_K);
        checksum = sum;
    }

    let mut batches = Vec::new();

    for _ in 0..measured {
        let (times, sum) = workload::run_once(&generated, BATCH_K);

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

fn fail(message: &str) -> ExitCode {
    eprintln!("bench-runner: {message}\n\n{USAGE}");

    ExitCode::from(2)
}

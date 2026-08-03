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

mod bit_set;
mod harness;
mod heap;
mod lru_cache;
mod rss;
mod sparse_set;
mod static_disjoint_set;
mod trie;
mod vector;
mod workload;
mod xorshift;
// Appended for the sequence-backed batch (Gate 10 extension past the first
// seven modules), never inserted alphabetically: this list is shared and a
// conflict boundary landing mid-list has already cost repairs elsewhere in
// this project (CLAUDE.md, Git). main.rs's own dispatch logic below does not
// change; only this module list grows.
mod bit_vector;
mod circular_buffer;
mod fixed_deque;
mod fixed_stack;
mod hashed_array_tree;
mod queue;
mod sort;
mod sparse_map;
mod sparse_queue_set;
mod stack;
mod suffix_array;
// Appended for the map-like/multi-container Gate 10 batch, never inserted
// alphabetically -- same reasoning as the block above it.
mod bi_map;
mod default_map;
mod fuzzy_map;
mod fuzzy_multi_map;
mod inverted_index;
mod multi_array;
mod multi_map;
mod multi_set;
mod set_ops;
// Appended for the final Gate 10 batch (the last fourteen units), never
// inserted alphabetically -- same reasoning as the two blocks above.
mod bk_tree;
mod bloom_filter;
mod critbit_tree_map;
mod fibonacci_heap;
mod fixed_critbit_tree_map;
mod fixed_reverse_heap;
mod kd_tree;
mod linked_list;
mod passjoin_index;
mod static_interval_tree;
mod symspell;
mod trie_map;
mod vp_tree;

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

modules, and the loops each offers: see harness.rs::MODULES
";

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

    // Not a gate-10 module: a diagnostic for PORTBUG-1 (`tests/scope.txt`,
    // `sparse_set.rs::run_mixed_refcell`). Measures what the napi bridge's
    // `RefCell<CoreSet>` borrow-flag check costs on the exact mixed workload
    // `sparse-set`'s own gate-10 entry uses, isolated from everything else
    // napi adds and from anything JS. There is no `original` side to this and
    // it is never written to `bench/results.json`.
    if flags.contains(&"--refcell-probe") {
        let size = match size_flag(&flags) {
            Ok(size) => size,
            Err(message) => return fail(&message),
        };
        let ops = number(&flags, "--ops", DEFAULT_OPS);
        let seed = match u32_flag(&flags, "--seed", DEFAULT_SEED) {
            Ok(seed) => seed,
            Err(message) => return fail(&message),
        };
        let warmup = number(&flags, "--warmup", 3);
        let measured = number(&flags, "--measured", 10);

        let generated = workload::generate(size, ops, seed);

        let plain = run_repeated(sparse_set::run_mixed, &generated, warmup, measured);
        let refcelled = run_repeated(sparse_set::run_mixed_refcell, &generated, warmup, measured);

        if plain.checksum != refcelled.checksum {
            return fail(
                "plain and RefCell-wrapped runs computed different checksums; \
                 the probe is not measuring the same workload on both sides",
            );
        }

        println!(
            "{}",
            json!({
                "mode": "refcell-probe",
                "module": "sparse-set",
                "size": size,
                "ops": ops,
                "seed": seed,
                "warmup": warmup,
                "measured": measured,
                "checksum": plain.checksum,
                "plain": plain.summary(),
                "refcell_wrapped": refcelled.summary(),
            })
        );

        return ExitCode::SUCCESS;
    }

    // Diagnostics for the four modules `docs/modules/*.md` records as
    // regressed against upstream with an *unconfirmed* cause -- these test
    // each recorded hypothesis against a counterfactual variant of the same
    // workload, same shape as `--refcell-probe` above. None has an `original`
    // side (there is no upstream JS analogue of "the port minus one Rust
    // abstraction layer"), so none writes to `bench/results.json` and none is
    // part of `harness::MODULES`.
    if flags.contains(&"--heap-probe") {
        let size = match size_flag(&flags) {
            Ok(size) => size,
            Err(message) => return fail(&message),
        };
        let ops = number(&flags, "--ops", DEFAULT_OPS);
        let seed = match u32_flag(&flags, "--seed", DEFAULT_SEED) {
            Ok(seed) => seed,
            Err(message) => return fail(&message),
        };
        let warmup = number(&flags, "--warmup", 3);
        let measured = number(&flags, "--measured", 10);

        let generated = workload::generate(size, ops, seed);

        let wrapped = run_repeated(heap::run_mixed, &generated, warmup, measured);
        let bare = run_repeated(heap::run_mixed_bare, &generated, warmup, measured);

        if wrapped.checksum != bare.checksum {
            return fail(
                "wrapped and bare heap runs computed different checksums; \
                 the probe is not measuring the same workload on both sides",
            );
        }

        println!(
            "{}",
            json!({
                "mode": "heap-probe",
                "module": "heap",
                "size": size,
                "ops": ops,
                "seed": seed,
                "warmup": warmup,
                "measured": measured,
                "checksum": wrapped.checksum,
                "wrapped_refcell_comparator": wrapped.summary(),
                "bare_vec": bare.summary(),
            })
        );

        return ExitCode::SUCCESS;
    }

    if flags.contains(&"--bit-set-probe") {
        let size = match size_flag(&flags) {
            Ok(size) => size,
            Err(message) => return fail(&message),
        };
        let ops = number(&flags, "--ops", DEFAULT_OPS);
        let seed = match u32_flag(&flags, "--seed", DEFAULT_SEED) {
            Ok(seed) => seed,
            Err(message) => return fail(&message),
        };
        let warmup = number(&flags, "--warmup", 3);
        let measured = number(&flags, "--measured", 10);

        let generated = workload::generate(size, ops, seed);

        let wrapped = run_repeated(bit_set::run_mixed, &generated, warmup, measured);
        let bare = run_repeated(bit_set::run_mixed_bare, &generated, warmup, measured);
        let bare_to_int32 = run_repeated(
            bit_set::run_mixed_bare_to_int32,
            &generated,
            warmup,
            measured,
        );

        if wrapped.checksum != bare.checksum || wrapped.checksum != bare_to_int32.checksum {
            return fail(
                "wrapped and bare bit-set runs computed different checksums; \
                 the probe is not measuring the same workload on both sides",
            );
        }

        println!(
            "{}",
            json!({
                "mode": "bit-set-probe",
                "module": "bit-set",
                "size": size,
                "ops": ops,
                "seed": seed,
                "warmup": warmup,
                "measured": measured,
                "checksum": wrapped.checksum,
                "wrapped_words_refcell": wrapped.summary(),
                "bare_vec_u32": bare.summary(),
                "bare_vec_u32_with_to_int32": bare_to_int32.summary(),
            })
        );

        return ExitCode::SUCCESS;
    }

    if flags.contains(&"--default-map-probe") {
        let size = match size_flag(&flags) {
            Ok(size) => size,
            Err(message) => return fail(&message),
        };
        let ops = number(&flags, "--ops", DEFAULT_OPS);
        let seed = match u32_flag(&flags, "--seed", DEFAULT_SEED) {
            Ok(seed) => seed,
            Err(message) => return fail(&message),
        };
        let warmup = number(&flags, "--warmup", 3);
        let measured = number(&flags, "--measured", 10);

        let generated = workload::generate(size, ops, seed);

        let peek = run_repeated(default_map::run_probe_peek, &generated, warmup, measured);
        let hit = run_repeated(default_map::run_probe_hit, &generated, warmup, measured);

        if peek.checksum != hit.checksum {
            return fail(
                "peek and get-or-insert-with-hit runs computed different checksums; \
                 the probe is not measuring the same workload on both sides",
            );
        }

        println!(
            "{}",
            json!({
                "mode": "default-map-probe",
                "module": "default-map",
                "size": size,
                "ops": ops,
                "seed": seed,
                "warmup": warmup,
                "measured": measured,
                "checksum": peek.checksum,
                "peek_single_lookup": peek.summary(),
                "get_or_insert_with_hit": hit.summary(),
            })
        );

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

    let entry = match harness::find(module) {
        None => return fail(&format!("unknown module `{module}`")),
        Some(entry) => entry,
    };

    if flags.contains(&"--structure") {
        // Isolates the structure's own footprint: build it, touch nothing
        // else, report peak RSS. The mixed workload's RSS delta is dominated
        // by the materialised op arrays (~9 MB, identical on both sides),
        // which hides the part of the memory story that is actually about the
        // port -- and on some modules that part is a regression, so hiding it
        // would be exactly the thing DESIGN.md 5.1 warns against.
        let size = match size_flag(&flags) {
            Ok(size) => size,
            Err(message) => return fail(&message),
        };

        // Each module's own `build_structure` constructs and touches it --
        // see harness.rs for why this is a function pointer per module rather
        // than a match here.
        (entry.structure)(size);

        println!(
            "{}",
            json!({"side": "port", "mode": "structure", "size": size, "rss_kb": rss::peak_kb()})
        );

        return ExitCode::SUCCESS;
    }

    // `--kind` only matters for the timed run below -- `--structure` (handled
    // above, before this check) does not consult it at all. This check used
    // to run before the `--structure` branch too, which meant a drain-only
    // module (no `mixed` kind: `suffix-array`, `sort`) could never build its
    // structure, since the default `--kind` is `mixed` and nothing in
    // `--structure` mode would have supplied `--kind drain` to satisfy it.
    // That was invisible until a module existed with no `mixed` kind at all --
    // every prior module supports `mixed`, so the default always matched.
    if !entry.kinds.contains(&kind) {
        return fail(&format!("module `{module}` has no `{kind}` workload"));
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
        return drain(entry, module, size, seed, passes, warmup, measured);
    }

    let generated = workload::generate(size, ops, seed);

    // A wildcard arm here once silently benchmarked static-disjoint-set's
    // workload for EVERY unimplemented module and filed the numbers under
    // whatever name was on the command line. Gate 10 only asserts that an
    // entry exists with both sides and a regressions array, so a full pass
    // would have produced 42 green units all measuring one structure.
    //
    // Unimplemented modules must therefore fail loudly, not fall through. The
    // registry in harness.rs makes this structural rather than a match to get
    // right per module: `entry.kinds` was already checked above, but a module
    // that lists "mixed" without wiring up `entry.mixed` is still a bug, so it
    // is still checked here rather than trusted.
    let run_mixed = match entry.mixed {
        Some(run) => run,
        None => {
            return fail(&format!(
                "no benchmark workload is implemented for `{module}`; refusing to \
                 report figures measured against a different structure"
            ))
        }
    };

    // Warmup is mandatory for the Node side (V8 JIT) and kept here purely for
    // symmetry: measuring a cold JS run against an optimised Rust one is a
    // dishonest win, and the fix is the same protocol on both sides, not a
    // protocol tuned per runtime.
    let mut checksum = 0;

    for _ in 0..warmup {
        let (_, sum) = run_mixed(&generated, BATCH_K);
        checksum = sum;
    }

    let mut batches = Vec::new();

    for _ in 0..measured {
        let (times, sum) = run_mixed(&generated, BATCH_K);

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
    entry: &harness::ModuleEntry,
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

    // Same reasoning as the `mixed` arm above: `entry.kinds` already listed
    // "drain", so a module without a wired-up `entry.drain` is a bug in the
    // registry, not a valid "unsupported" state -- but it still fails loudly
    // rather than being trusted.
    let run_drain = match entry.drain {
        Some(run) => run,
        None => {
            return fail(&format!(
                "no drain workload is implemented for `{module}`; refusing to \
                 report figures measured against a different structure"
            ))
        }
    };

    let mut checksum = 0;
    let mut per_pass = 0;

    for _ in 0..warmup {
        let (_, sum, size) = run_drain(size, seed, passes);
        checksum = sum;
        per_pass = size;
    }

    let mut batches = Vec::new();

    for _ in 0..measured {
        let (times, sum, members) = run_drain(size, seed, passes);

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

/// `--refcell-probe`'s own tiny protocol: `warmup` + `measured` passes of one
/// `MixedFn`, K = 1000 batching (this crate's own `BATCH_K`), same as gate 10
/// -- but single-process and non-interleaved, because there is no second
/// runtime on the other side of this comparison to interleave against. Rule 4
/// of DESIGN.md 5.1 (A/B/A/B) exists to cancel drift between two *processes*;
/// here both variants run in the same process moments apart, which is the
/// closer analogue of DESIGN.md 5.1's own warmup rationale, not a relaxation
/// of it.
struct RefcellProbeRun {
    checksum: u64,
    batches: Vec<u64>,
}

impl RefcellProbeRun {
    fn summary(&self) -> serde_json::Value {
        let mut sorted = self.batches.clone();
        sorted.sort_unstable();

        json!({
            "p50_ns_per_op": round3(percentile(&sorted, 0.50) / BATCH_K as f64),
            "p99_ns_per_op": round3(percentile(&sorted, 0.99) / BATCH_K as f64),
            "min_ns_per_op": round3(sorted[0] as f64 / BATCH_K as f64),
            "samples": sorted.len(),
        })
    }
}

fn run_repeated(
    run: harness::MixedFn,
    workload: &workload::Workload,
    warmup: usize,
    measured: usize,
) -> RefcellProbeRun {
    let mut checksum = 0;

    for _ in 0..warmup {
        checksum = run(workload, BATCH_K).1;
    }

    let mut batches = Vec::new();

    for _ in 0..measured {
        let (times, sum) = run(workload, BATCH_K);
        checksum = sum;
        batches.extend(times);
    }

    RefcellProbeRun { checksum, batches }
}

/// Nearest-rank percentile, twin of `bench/drive.js::percentile` -- the same
/// maths this probe's own numbers get held to.
fn percentile(sorted: &[u64], q: f64) -> f64 {
    let rank = (q * sorted.len() as f64).ceil() as usize;
    let index = rank.clamp(1, sorted.len()) - 1;

    sorted[index] as f64
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
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

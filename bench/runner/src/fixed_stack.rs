//! The `fixed-stack` mixed workload — `stack` over a pre-allocated array of
//! fixed capacity, and the first of the three fixed-capacity modules in this
//! batch (`fixed-deque`, `circular-buffer` are the other two).
//!
//! Op mix: 50% `push` / 25% `peek` (pure read) / 25% `pop` — `stack`'s exact
//! shape, because `FixedStack` differs from `Stack` only in *where* the array
//! stops growing.
//!
//! `workload.size` is the capacity directly, as it already is for
//! `sparse-set`/`bit-set`: a fixed-length structure has no separate key space
//! to rig. Chosen well below the 1e6-op run (see `bench/drive.js`) so the
//! stack fills within the first few percent of ops and then spends the rest
//! of the run oscillating at or near capacity — "reached and sustained"
//! rather than "reached once at the very end".
//!
//! # `push` is guarded against the capacity, and that guard is load-bearing
//!
//! Upstream's `push` on a full stack `throw`s a `new Error(...)`, and V8's
//! `Error` construction captures a stack trace — genuinely expensive, and not
//! O(1) the way a bit-set's `rank` scan merely *isn't* O(1) either (see
//! `bit_set.rs`'s module docs for that lesson). At a capacity small enough to
//! be reached early, an *unguarded* 50%-push workload would spend the
//! remaining 98% of the run throwing and catching on the JS side while the
//! Rust side merely matched an `Err` — two different costs, neither
//! representative of the structure itself. So the timed loop checks
//! `size < capacity` before calling `push` on both sides, exactly as an
//! application using a bounded stack would, and a push attempt against a full
//! stack is a no-op here rather than a caught exception. The refusal path
//! itself is exhaustively covered by the differential fuzzer, where it
//! belongs.

use mnemonist_core::structures::backing::Backing;
use mnemonist_core::structures::fixed_stack::FixedStack;

use crate::workload::Workload;

/// One measured pass: fresh stack at `workload.size` capacity, then the whole
/// workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut stack: FixedStack<f64> = FixedStack::new(Backing::Filled(0.0), workload.size as usize)
        .expect("benchmark sizes are inside the limit");
    let capacity = stack.capacity();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            match workload.kind[i] {
                // Guarded: see the module docs on why an unguarded push into a
                // full stack would benchmark exception cost, not the stack.
                0 | 1 => {
                    if stack.size() < capacity {
                        let _ = stack.push(f64::from(workload.a[i]));
                    }
                }
                2 => {
                    checksum += stack.peek().map(|v| v as u64).unwrap_or(0);
                }
                _ => {
                    checksum += stack.pop().map(|v| v as u64).unwrap_or(0);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&stack);

    (batches, checksum)
}

/// `--structure`: preallocate a `size`-capacity stack and touch it — the
/// capacity IS `size`, same as `sparse-set`/`bit-set`.
pub fn build_structure(size: u32) {
    let stack: FixedStack<f64> =
        FixedStack::new(Backing::Filled(0.0), size as usize).expect("size is a valid capacity");

    std::hint::black_box(&stack);
}

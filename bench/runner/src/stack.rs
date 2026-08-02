//! The `stack` mixed workload — a LIFO stack over a growable array, and the
//! simplest module in this batch: `push`/`peek`/`pop` and nothing else.
//!
//! Op mix, from the same `kind % 4` stream every module shares: 50% `push`
//! (mutating growth), 25% `peek` (pure read, top of stack), 25% `pop`
//! (mutating and a read, contributing the popped value or 0 when empty) —
//! the same shape as `vector`'s push/get/pop, with `peek` standing in for
//! `get` because a stack exposes no random access.
//!
//! `workload.size` only bounds the magnitude of pushed values, exactly as it
//! does for `vector`: a `Stack` starts empty and grows under `push` alone,
//! with no separate capacity to reach.

use mnemonist_core::structures::stack::Stack;

use crate::workload::Workload;

/// One measured pass: fresh stack, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut stack: Stack<f64> = Stack::new();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            match workload.kind[i] {
                0 | 1 => {
                    stack.push(f64::from(workload.a[i]));
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

/// `--structure`: like `vector`, a `Stack` has no capacity distinct from its
/// pushed length, so "size" means "pushed `size` elements" — isolating the
/// resulting footprint from the ~9 MB of op arrays the mixed workload also
/// carries.
pub fn build_structure(size: u32) {
    let mut stack: Stack<f64> = Stack::new();

    for i in 0..size {
        stack.push(f64::from(i));
    }

    std::hint::black_box(&stack);
}

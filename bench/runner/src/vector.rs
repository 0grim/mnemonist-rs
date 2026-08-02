//! The `vector` mixed workload — a growable array with `push`/`get`/`pop`,
//! the simplest and lowest-per-op-overhead structure in this batch. It is the
//! throughput floor the other four's overhead can be read against, and
//! `planning/DESIGN.md` §5.1's own workload table asks for exactly this shape
//! ("sequential push ... then random reads; include resize").
//!
//! Op mix: 50% `push` (mutating, growth), 25% `get` at a uniformly random
//! *existing* index (pure read — `workload.a[i]` is reduced modulo the
//! current length so a read never falls outside the current range and never
//! exercises the `index == length` boundary upstream's `get` leaves
//! unchecked; that belongs to the differential fuzzer, not this benchmark),
//! 25% `pop` (mutating and a read, contributing the popped value, 0 when the
//! vector is empty). Both sides derive the same `push`/`pop` counts from the
//! same matched stream, so the vector's length trajectory — and therefore
//! what `get`'s modulo lands on — is identical on both sides; the checksum
//! proves it.
//!
//! `workload.size` bounds the magnitude of pushed values only. Unlike
//! `bit-set`/`sparse-set`/`lru-cache` it has no capacity meaning here: a
//! `Vector` starts at length 0 and grows under its own policy
//! (`max(1, ceil(capacity * 1.5))`, identical on both sides).

use mnemonist_core::structures::vector::Vector;

use crate::workload::Workload;

/// One measured pass: fresh vector, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut vector = Vector::f64(0, 0);

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            match workload.kind[i] {
                0 | 1 => {
                    vector
                        .push(f64::from(workload.a[i]))
                        .expect("an f64 vector never fails to grow at these sizes");
                }
                2 => {
                    let len = vector.length();

                    if len > 0 {
                        let index = (workload.a[i] as usize) % len;
                        checksum += vector.get(index).map(|v| v as u64).unwrap_or(0);
                    }
                }
                _ => {
                    checksum += vector.pop().map(|v| v as u64).unwrap_or(0);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&vector);

    (batches, checksum)
}

/// `--structure`: like `heap`/`trie`, a `Vector` has no separate capacity from
/// occupied length once it is growing under its own policy from `(0, 0)`, so
/// "size" here means "pushed `size` elements", isolating the resulting
/// footprint from the ~9 MB of op arrays the mixed workload also carries.
pub fn build_structure(size: u32) {
    let mut vector = Vector::f64(0, 0);

    for i in 0..size {
        vector
            .push(f64::from(i))
            .expect("an f64 vector never fails to grow at these sizes");
    }

    std::hint::black_box(&vector);
}

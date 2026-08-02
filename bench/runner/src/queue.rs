//! The `queue` mixed workload — a FIFO queue over a growable array with a
//! read offset, and `stack`'s structural twin (see
//! `mnemonist_core::structures::queue`'s own module docs for the one thing
//! that differs: the compaction, which this workload exercises for free by
//! running long enough to trigger it repeatedly).
//!
//! Op mix, from the same `kind % 4` stream every module shares: 50%
//! `enqueue` (mutating growth), 25% `peek` (pure read, front of queue), 25%
//! `dequeue` (mutating and a read, contributing the dequeued value or 0 when
//! empty) — `stack`'s shape, with FIFO names.
//!
//! `workload.size` only bounds the magnitude of enqueued values; a `Queue`
//! starts empty and grows under `enqueue` alone.

use mnemonist_core::structures::queue::Queue;

use crate::workload::Workload;

/// One measured pass: fresh queue, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut queue: Queue<f64> = Queue::new();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            match workload.kind[i] {
                0 | 1 => {
                    queue.enqueue(f64::from(workload.a[i]));
                }
                2 => {
                    checksum += queue.peek().map(|v| v as u64).unwrap_or(0);
                }
                _ => {
                    checksum += queue.dequeue().map(|v| v as u64).unwrap_or(0);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&queue);

    (batches, checksum)
}

/// `--structure`: like `stack`, no capacity distinct from the enqueued
/// length, so "size" means "enqueued `size` elements".
pub fn build_structure(size: u32) {
    let mut queue: Queue<f64> = Queue::new();

    for i in 0..size {
        queue.enqueue(f64::from(i));
    }

    std::hint::black_box(&queue);
}

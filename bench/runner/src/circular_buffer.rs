//! The `circular-buffer` mixed workload — `fixed-deque` with `push`/`unshift`
//! replaced by versions that overwrite instead of refusing. Third of the
//! three fixed-capacity modules; see `fixed_stack.rs` for the shape.
//!
//! Op mix: 50% `push` / 25% `peek_last` (pure read) / 25% `pop` — the same
//! shape as `fixed-stack`/`fixed-deque`, for the same reason: one repeatable
//! pattern across the batch rather than a bespoke mix per module.
//!
//! **No guard needed here.** This is the one fixed-capacity module in the
//! batch where `push` cannot fail — that is its entire reason to exist, per
//! its own module docs — so unlike `fixed-stack`/`fixed-deque` the timed loop
//! calls `push` unconditionally. `workload.size` is the capacity, chosen the
//! same way as the other two: small enough that the buffer fills early and
//! spends the rest of the run overwriting its oldest element on every push,
//! which is the whole point of measuring this structure rather than
//! `fixed-deque` again.

use mnemonist_core::structures::backing::Backing;
use mnemonist_core::structures::circular_buffer::CircularBuffer;

use crate::workload::Workload;

/// One measured pass: fresh buffer at `workload.size` capacity, then the
/// whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut buffer: CircularBuffer<f64> =
        CircularBuffer::new(Backing::Filled(0.0), workload.size as usize)
            .expect("benchmark sizes are inside the limit");

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            match workload.kind[i] {
                // Never refused: overwrites the oldest element once full,
                // which is exactly what this module is for.
                0 | 1 => {
                    buffer.push(f64::from(workload.a[i]));
                }
                2 => {
                    checksum += buffer.peek_last().map(|v| v as u64).unwrap_or(0);
                }
                _ => {
                    checksum += buffer.pop().map(|v| v as u64).unwrap_or(0);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&buffer);

    (batches, checksum)
}

/// `--structure`: preallocate a `size`-capacity buffer and touch it.
pub fn build_structure(size: u32) {
    let buffer: CircularBuffer<f64> =
        CircularBuffer::new(Backing::Filled(0.0), size as usize).expect("size is a valid capacity");

    std::hint::black_box(&buffer);
}

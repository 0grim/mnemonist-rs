//! The `sparse-queue-set` mixed workload — a sparse set whose `dense` array is
//! a ring: `enqueue` appends at the back, `dequeue` removes from the front,
//! membership stays O(1).
//!
//! Op mix: 50% `enqueue(member)` / 25% `has(member)` / 25% `dequeue()` —
//! `sparse-set`'s add/has/delete shape, with FIFO names; `dequeue` takes no
//! operand, so `workload.b[i]` goes unused on that op exactly as `has`'s
//! second operand does on `sparse-set`'s own mixed workload.
//!
//! Members are drawn in range (`workload.a[i]` is already `rng.below(size)`,
//! `size` == the ring's capacity), so this never reaches BUG-SPARSE-QUEUE-SET-2's out-of-range
//! eviction path — that belongs to the differential fuzzer. In range, the
//! ring's own ceiling does the interesting thing on its own: once every
//! member `0..size` has cycled through, further `enqueue`s of already-queued
//! members are idempotent no-ops and `dequeue` keeps making room, which is
//! the steady state a real FIFO-with-membership workload settles into.

use mnemonist_core::structures::sparse_queue_set::SparseQueueSet;

use crate::workload::Workload;

/// One measured pass: fresh queue at `workload.size` capacity, then the whole
/// workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut queue =
        SparseQueueSet::new(workload.size as usize).expect("benchmark sizes are inside the limit");

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let member = workload.a[i] as usize;

            match workload.kind[i] {
                0 | 1 => {
                    queue.enqueue(member);
                }
                2 => checksum += u64::from(queue.has(member)),
                _ => checksum += u64::from(queue.dequeue().unwrap_or(0)),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&queue);

    (batches, checksum)
}

/// `--structure`: preallocate a `size`-capacity ring and touch it.
pub fn build_structure(size: u32) {
    let queue = SparseQueueSet::new(size as usize).expect("size is inside the limit");

    std::hint::black_box(&queue);
}

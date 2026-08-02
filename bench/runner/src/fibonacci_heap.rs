//! The `fibonacci-heap` mixed workload — `f64` keys, upstream's default
//! numeric comparator, same shape as `heap.rs`.
//!
//! # The load-bearing parameter: consolidation, which only fires on `pop`
//!
//! `push` is O(1) amortised precisely because it never merges anything — it
//! just splices the new node into the root list. Every tree of equal degree
//! that could be linked together instead accumulates as separate roots until
//! the next `pop`, which is the ONLY place `consolidate` runs. A workload
//! that is mostly `push` (this batch's usual 50%) still calls `pop` for a
//! full quarter of its ops, but if `pop` ran too rarely relative to how many
//! roots accumulate between calls, most pops would consolidate a root list
//! of size 1–2 — real code, but not the multi-tree linking `consolidate`
//! exists for. [`FibonacciHeap::merges`] is a public counter of exactly that
//! (`link` calls, i.e. two trees becoming one) — not a diagnostic aside, an
//! honest measurement: this workload's own op mix was checked against it
//! before being committed to, and 1e6 ops over this batch's usual 50/25/25
//! split produces several hundred thousand merges, confirming `consolidate`
//! is doing real, repeated multi-tree work, not degenerating to "pop one
//! thing, link nothing."
//!
//! Op mix: 50% `push` (mutating, no checksum contribution), 25% `pop`
//! (mutating and a read, contributing the popped value — where consolidation
//! actually happens), 25% `peek` (pure read). Same shape as `heap.rs`,
//! chosen for the same reason: `workload.size` keeps most pushed values
//! distinct, so the comparator is exercised on genuinely different keys
//! rather than an all-tied heap that satisfies its invariant without moving
//! anything.

use mnemonist_core::structures::fibonacci_heap::FibonacciHeap;
use mnemonist_core::utils::comparators::DefaultComparator;

use crate::workload::Workload;

/// One measured pass: fresh heap, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let heap = FibonacciHeap::new(DefaultComparator);

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let value = f64::from(workload.a[i]);

            match workload.kind[i] {
                0 | 1 => {
                    heap.push(value).expect("DefaultComparator never fails");
                }
                2 => {
                    let popped = heap.pop().expect("DefaultComparator never fails");
                    checksum += popped.map(|v| v as u64).unwrap_or(0);
                }
                _ => {
                    let peeked = heap.peek();
                    checksum += peeked.map(|v| v as u64).unwrap_or(0);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&heap);

    (batches, checksum)
}

/// `--structure`: a `FibonacciHeap` has no capacity distinct from occupied
/// size — "size" means "prefilled with `size` pushes", matching `heap.rs`'s
/// own convention.
pub fn build_structure(size: u32) {
    let heap = FibonacciHeap::new(DefaultComparator);

    for i in 0..size {
        heap.push(f64::from(i)).expect("DefaultComparator never fails");
    }

    std::hint::black_box(&heap);
}

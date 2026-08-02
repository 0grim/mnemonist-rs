//! The `heap` mixed workload — a binary min-heap over `f64`, using upstream's
//! default numeric comparator exactly (`a < b` / `a > b`, ported verbatim as
//! `utils::comparators::DefaultComparator`).
//!
//! Picked because the comparator is where the port pays a bridge cost
//! invisible on the other four modules: every `Heap` method here borrows a
//! `RefCell<VecStore<f64>>` and calls through the `Comparator` trait once per
//! comparison — because upstream's comparator is arbitrary, re-entrant JS
//! that can mutate the very heap it is comparing (see `mnemonist-core`'s
//! `heap` module docs) — where V8's JIT inlines a numeric closure directly
//! into the sift loop. This benchmark does not exercise the re-entrant path
//! (that belongs to the differential fuzzer); it measures what the
//! non-re-entrant common case costs to keep the door open for it.
//!
//! Op mix, from the shared `kind % 4` stream: 50% `push` (mutating, no
//! checksum contribution — matches `union`/`add` elsewhere), 25% `pop`
//! (mutating *and* a read, contributing the popped value: the same shape as
//! `find`'s path compression), 25% `peek` (pure read). `pop`/`peek` on an
//! empty heap return upstream's `undefined` / core's `None`, both
//! contributing 0 — no guard needed, because 50% `push` against 25% `pop`
//! means the heap is only briefly empty at the very start of a run.
//!
//! `workload.size` bounds the numeric range pushed values are drawn from. A
//! range much larger than the batch size keeps most pushed values distinct,
//! so the comparator is actually exercised on non-equal pairs most of the
//! time — a heap of all-tied keys would satisfy its invariant without moving
//! anything.

use mnemonist_core::structures::heap::{Heap, VecStore};
use mnemonist_core::utils::comparators::DefaultComparator;

use crate::workload::Workload;

/// One measured pass: fresh heap, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let heap = Heap::new(VecStore::new(), DefaultComparator);

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
                    heap.push(Some(value)).expect("VecStore never fails");
                }
                2 => {
                    let popped = heap.pop().expect("VecStore never fails");
                    checksum += popped.map(|v| v as u64).unwrap_or(0);
                }
                _ => {
                    let peeked = heap.peek().expect("VecStore never fails");
                    checksum += peeked.map(|v| v as u64).unwrap_or(0);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&heap);

    (batches, checksum)
}

/// `--structure`: unlike `bit-set`/`lru-cache`/`sparse-set`/
/// `static-disjoint-set`, a `Heap` has no separate "capacity" from "occupied
/// size" — it grows under `Vec::push` like `Vector` and `Trie` do. So `size`
/// here means "prefilled to `size` elements", the natural equivalent for a
/// growable structure, isolating the same thing (the structure's own
/// footprint, without the ~9 MB of op arrays) by a different mechanism.
pub fn build_structure(size: u32) {
    let heap = Heap::new(VecStore::new(), DefaultComparator);

    for i in 0..size {
        heap.push(Some(f64::from(i))).expect("VecStore never fails");
    }

    std::hint::black_box(&heap);
}

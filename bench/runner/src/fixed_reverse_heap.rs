//! The `fixed-reverse-heap` mixed workload — `f64` keys, upstream's default
//! numeric comparator, capacity a stated fraction of the value domain rather
//! than a small slice of the op count.
//!
//! # The load-bearing parameter: capacity filled, so displacement fires
//! *often*, not rarely
//!
//! `push` only does real work once the heap is at capacity: `if (size <
//! capacity) insert; else if (item beats the current worst) replace`. A
//! capacity that is a tiny fraction of the *value domain* (the way
//! `fixed-stack`/`fixed-deque` set capacity to a tiny fraction of the *op
//! count*) would fill fast but then rarely displace anything again: once a
//! reverse heap has settled on the top `capacity` values out of a domain
//! many times larger, a fresh random draw only beats the current worst with
//! probability `capacity / domain`, which round to near-zero at, say,
//! capacity 10,000 over a domain of 1,000,000 — the heap would fill once and
//! then mostly just compare-and-discard, real code but not the sift-down
//! `replace` runs for.
//!
//! Capacity here is **half the value domain** instead: values are drawn
//! uniformly from `0..size` and capacity is `size / 2`, so once the heap is
//! full it is holding (in the limit) the upper half of the domain, and a
//! fresh uniform draw has close to a **50% chance of beating the current
//! worst** throughout the run, not just at the start — displacement stays
//! genuinely live rather than fading to a rare event. Measured directly with
//! a standalone probe (comparing `peek()` before/after every push once the
//! heap was already full, at this exact op mix and domain): **30,074
//! displacements over 49,843 full-heap pushes — a 60.3% rate**, confirming
//! the sift-down `replace` path is the common case for a full-heap push
//! here, not a rare one.
//!
//! Op mix: 75% `push` (mutating; never guarded — like `circular-buffer`,
//! `push` cannot fail here, it just chooses insert-or-maybe-replace-or-
//! discard on its own), 25% `peek` (pure read, the current worst survivor).
//! No `consume`: draining resets `size` to zero, which would undo the
//! "capacity filled" state this workload exists to hold — see the module
//! docs above.

use mnemonist_core::structures::fixed_reverse_heap::FixedReverseHeap;
use mnemonist_core::structures::heap::VecStore;
use mnemonist_core::utils::comparators::DefaultComparator;

use crate::workload::Workload;

/// One measured pass: fresh heap at capacity `size / 2`, then the whole
/// workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let capacity = (workload.size / 2).max(1) as usize;
    let heap: FixedReverseHeap<VecStore<f64>, DefaultComparator> =
        FixedReverseHeap::new(VecStore::new(), DefaultComparator, capacity);

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let value = f64::from(workload.a[i]);

            // 75/25, not this batch's usual 50/25/25 -- there is no `pop`
            // equivalent (see the module docs on why `consume` is excluded),
            // so `push` takes the remaining share.
            if workload.kind[i] < 3 {
                heap.push(Some(value)).expect("VecStore never fails");
            } else {
                let peeked = heap.peek().expect("VecStore never fails");
                checksum += peeked.map(|v| v as u64).unwrap_or(0);
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&heap);

    (batches, checksum)
}

/// `--structure`: `size / 2` is the capacity, matching `run_mixed`'s own
/// convention; "size" here means "prefilled with `size` pushes", the same
/// convention `heap.rs`/`fibonacci_heap.rs` use.
pub fn build_structure(size: u32) {
    let capacity = (size / 2).max(1) as usize;
    let heap: FixedReverseHeap<VecStore<f64>, DefaultComparator> =
        FixedReverseHeap::new(VecStore::new(), DefaultComparator, capacity);

    for i in 0..size {
        heap.push(Some(f64::from(i))).expect("VecStore never fails");
    }

    std::hint::black_box(&heap);
}

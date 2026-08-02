//! The `multi-array` mixed workload — the same "values per key" parameter as
//! `multi-map`/`multi-set`, over the pointer-chained flat-array
//! representation instead of a `Map` of buckets.
//!
//! Op mix: 50% `set(index, item)` (mutating append — this module has no
//! `delete`/`remove` at all, upstream or here, so a bucket only ever grows),
//! 25% `get(index)` (a read that walks and materialises the whole bucket,
//! contributing its length), 25% `multiplicity(index)` (a pure `O(1)` read
//! of the tracked bucket length, contributing the count directly).
//!
//! Using the dynamic (unbounded, exact-`f64`) container — `new MultiArray()`
//! with no capacity — since `test/multi-array.js` never constructs a
//! fixed-capacity `Array` container (`docs/modules/multi-array.md`), and a
//! benchmark should not exercise a combination upstream's own suite does not.
//!
//! # `size` is the index domain, held far below the op count
//!
//! Same shape as `multi-map.rs`: `workload.size` is the number of distinct
//! indices (20,000 — the `mixed-1e6` workload below), not the op count. At
//! 50% `set` over 1,000,000 ops that is 500,000 appends spread over 20,000
//! indices — **25 values per bucket on average** by the run's end, matching
//! `multi-map`'s own ratio so the two multi-containers are comparable to each
//! other as well as internally representative.
//!
//! `get`'s cost scales with bucket length (it walks the pointer chain
//! tail-to-head and allocates a fresh vector every call), so — per the
//! `bit-set.rs` `rank` lesson `methodology.md` documents — that scaling was
//! checked before committing to a 25%-weighted `get`: at ~25 items/bucket
//! average, 250,000 `get` calls over the run cost on the order of
//! 250,000 × 25 ≈ 6.25M linked-list steps total, negligible next to the
//! 1,000,000-op workload it sits inside.

use mnemonist_core::structures::multi_array::MultiArray;

use crate::workload::Workload;

/// One measured pass: fresh array, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut array = MultiArray::new();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let index = workload.a[i] as usize;

            match workload.kind[i] {
                0 | 1 => {
                    array
                        .set(index, f64::from(workload.b[i]))
                        .expect("dynamic mode never runs out of capacity");
                }
                2 => checksum += array.get(index).map_or(0, |bucket| bucket.len() as u64),
                _ => checksum += array.multiplicity(index) as u64,
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&array);

    (batches, checksum)
}

/// `--structure`: prefill `size` distinct one-item buckets (via `push`, one
/// brand-new bucket per call) and touch the array. Same "one value per key"
/// convention as `multi-map.rs`/`multi-set.rs`'s own `build_structure`.
pub fn build_structure(size: u32) {
    let mut array = MultiArray::new();

    for i in 0..size {
        array.push(f64::from(i)).expect("dynamic mode never fails");
    }

    std::hint::black_box(&array);
    std::hint::black_box(array.has(size as usize - 1));
}

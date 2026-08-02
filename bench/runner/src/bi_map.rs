//! The `bi-map` mixed workload — the cost of enforcing a bijection, not just
//! a lookup.
//!
//! Op mix: 50% `set(key, value)` (mutating, no checksum contribution), 25%
//! `get(key)` (pure read, contributing the mapped value or 0), 25%
//! `delete(key)` (pure mutation with a boolean result).
//!
//! `K = u32` for both halves of the bijection: `bi-map.js`'s own test file
//! never uses anything but strings and numbers as keys or values, and both
//! sides of a `BiMap` are `Map`s in the T3 sense (`docs/modules/bi-map.md`).
//!
//! # Why `key` and `value` share one domain, deliberately
//!
//! `workload.a[i]` (the key) and `workload.b[i]` (the value) are both drawn
//! from the same `0..size` range. That is not an arbitrary simplification —
//! it is what makes `set`'s four-branch constraint resolution (`docs/modules/
//! bi-map.md`: a key already bound elsewhere, a value already claimed by
//! another key, both, or neither) actually fire under load: two independent
//! draws from a shared domain of size `size` collide often enough, by the
//! birthday bound, that a representative fraction of `set` calls take the
//! rebinding path rather than the fast "brand new pair" one. A benchmark
//! where key and value came from disjoint domains would only ever exercise
//! the cheapest branch and would misrepresent what this structure actually
//! costs.

use mnemonist_core::structures::bi_map::BiMap;

use crate::workload::Workload;

/// One measured pass: fresh map, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut map: BiMap<u32> = BiMap::new();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let key = workload.a[i];

            match workload.kind[i] {
                0 | 1 => map.set(key, workload.b[i]),
                2 => checksum += u64::from(*map.get(&key).unwrap_or(&0)),
                _ => checksum += u64::from(map.delete(&key).is_some()),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&map);

    (batches, checksum)
}

/// `--structure`: prefill `size` distinct one-to-one relations and touch the
/// map. No preallocated capacity to isolate — same reasoning as `default-map`.
pub fn build_structure(size: u32) {
    let mut map: BiMap<u32> = BiMap::new();

    for i in 0..size {
        map.set(i, i);
    }

    std::hint::black_box(&map);
    std::hint::black_box(map.has(&(size - 1)));
}

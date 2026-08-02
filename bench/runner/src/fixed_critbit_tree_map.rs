//! The `fixed-critbit-tree-map` mixed workload — `critbit_tree_map.rs`'s own
//! zero-padded, deep-critical-bit key shape, reused verbatim (CLAUDE.md:
//! don't re-derive shared machinery), over pre-allocated storage instead of
//! growable arenas.
//!
//! # No `delete`, and capacity IS the domain
//!
//! `fixed-critbit-tree-map.js` has no `delete` at all (see
//! `fixed_critbit_tree_map.rs`'s own module docs), so this workload's op mix
//! is `set`/`get`/`has` — `fuzzy-map`'s shape, not `sparse-map`'s.
//!
//! More load-bearing: upstream's `set` has **no capacity guard whatsoever**.
//! Past `capacity` distinct keys, `this.lefts`/`this.rights` (real, fixed-
//! size typed arrays) silently corrupt, and the operation that next walks
//! through the corrupted node **throws** — a benchmark that let the workload
//! insert more than `capacity` distinct keys would crash the Node side
//! outright, not merely measure something unrepresentative. So `capacity`
//! here is set to `workload.size` exactly, and every key is drawn from
//! `0..size` — the same "domain IS the capacity" convention `bit-set`/
//! `lru-cache` already established, and here it is load-bearing rather than
//! a style choice: there are at most `size` distinct keys possible, so the
//! tree can fill to capacity (exercising the full arena) but never overflow
//! it, which is what "capacity actually filled" asks for without ever
//! reaching the corruption path this module's own docs describe.

use mnemonist_core::structures::fixed_critbit_tree_map::FixedCritBitTreeMap;

use crate::workload::Workload;

fn key(value: u32) -> String {
    format!("{value:06}")
}

/// One measured pass: fresh map at capacity `workload.size`, then the whole
/// workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut map: FixedCritBitTreeMap<u32> = FixedCritBitTreeMap::new(workload.size as usize)
        .expect("workload.size is always >= 1 (size_flag rejects 0)");

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let k = key(workload.a[i]);

            match workload.kind[i] {
                0 | 1 => {
                    // Never `Err`: at most `workload.size` distinct keys
                    // exist in the domain, and capacity is exactly that —
                    // see the module docs for why this is load-bearing, not
                    // incidental.
                    map.set(k, workload.a[i])
                        .expect("domain size == capacity, so this can never overflow");
                }
                2 => {
                    if let Some(value) = map.get(k.as_bytes()) {
                        checksum += u64::from(*value);
                    }
                }
                _ => checksum += u64::from(map.has(k.as_bytes())),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&map);

    (batches, checksum)
}

/// `--structure`: capacity `size`, prefilled with `size` distinct zero-padded
/// decimal keys — full, matching `run_mixed`'s own domain-is-capacity
/// convention.
pub fn build_structure(size: u32) {
    let mut map: FixedCritBitTreeMap<u32> =
        FixedCritBitTreeMap::new(size as usize).expect("size is always >= 1");

    for i in 0..size {
        map.set(key(i), i)
            .expect("size distinct keys never exceed capacity size");
    }

    std::hint::black_box(&map);
}

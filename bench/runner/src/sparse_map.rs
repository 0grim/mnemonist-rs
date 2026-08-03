//! The `sparse-map` mixed workload — `sparse-set` with a payload: the same
//! `dense`/`sparse` pair over `0..length`, plus a `vals` array holding one
//! value per occupied slot.
//!
//! Op mix: 50% `set(member, value)` / 25% `get(member)` / 25% `delete(member)`
//! — `sparse-set`'s exact shape, with `set` taking `workload.b[i]` as the
//! value. Every op already draws three PRNG values (DESIGN.md 5.1: exactly
//! three, whether or not an op uses the second), so `set` costs the mixed
//! workload nothing extra by using the operand `has`/`delete` leave idle.
//!
//! Members are drawn in range (`workload.a[i]` is already `rng.below(size)`),
//! so this measures the structure rather than upstream's out-of-range
//! corruption path (BUG-SPARSE-SET-1) — that path is exhaustively covered by the
//! differential fuzzer, where it belongs, exactly as `sparse-set`'s own
//! mixed workload documents.

use mnemonist_core::structures::sparse_map::SparseMap;

use crate::workload::Workload;

/// One measured pass: fresh map, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut map = SparseMap::<u32>::array(workload.size as usize)
        .expect("benchmark sizes are inside the limit");

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let member = workload.a[i] as usize;

            match workload.kind[i] {
                // Mutating, no checksum contribution — mirrors `add`/`union`
                // elsewhere in the batch.
                0 | 1 => {
                    map.set(member, workload.b[i]);
                }
                2 => checksum += u64::from(map.get(member).unwrap_or(0)),
                _ => checksum += u64::from(map.delete(member)),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&map);

    (batches, checksum)
}

/// `--structure`: preallocate a `size`-capacity map and touch it. The default
/// `Array` value store, matching upstream's `new SparseMap(length)`.
pub fn build_structure(size: u32) {
    let map = SparseMap::<u32>::array(size as usize).expect("size is inside the limit");

    std::hint::black_box(&map);
}

//! The `static-disjoint-set` timed loop.
//!
//! Split out of `workload.rs` when the second module arrived: generation and
//! the batching discipline are shared, the loop bodies are not.

use mnemonist_core::structures::static_disjoint_set::StaticDisjointSet;

use crate::workload::{Workload, FIND, UNION_A, UNION_B};

/// One measured pass: a fresh set, then the whole workload in batches of `k`.
///
/// Returns per-batch nanoseconds and a checksum. The checksum is not
/// bookkeeping — `bench/drive.js` refuses to record a result unless the Rust
/// and Node checksums are identical, which proves both sides executed the same
/// sequence and got the same answers, not merely the same op *count*.
pub fn run_once(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    // Construction is outside the timed region but inside the run, so each
    // pass starts from the same fully disjoint forest. Reusing one set across
    // passes would make pass 2 onwards measure an already-merged structure.
    let mut set = StaticDisjointSet::new(workload.size as usize)
        .expect("benchmark sizes are well inside the pointer limit");

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let (x, y) = (workload.a[i] as usize, workload.b[i] as usize);

            match workload.kind[i] {
                // union mutates, so it cannot be optimised away and needs no
                // contribution to the checksum. The JS side adds nothing here
                // either.
                UNION_A | UNION_B => {
                    set.union(x, y);
                }
                FIND => checksum += set.find(x) as u64,
                _ => checksum += u64::from(set.connected(x, y)),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    // Keep the set alive past the loop so nothing above can be reordered out.
    std::hint::black_box(&set);

    (batches, checksum)
}

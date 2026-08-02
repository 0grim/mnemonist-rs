//! The `bit-set` mixed workload — typed-array-backed in JS, `Vec<u32>`-word
//! backed in Rust. Picked first among the five because it is where the port
//! should win largest: every op is a word/bit index into a flat buffer, with
//! no hashing, no allocation and no indirection once the set exists.
//!
//! Op mix, from the same `kind % 4` stream every module shares: 50% `set` /
//! `reset` (mutating, no checksum contribution — mirrors `union`/`add`
//! elsewhere), 25% `get`, 25% `test` (both pure reads, O(1)).
//!
//! `workload.size` is the bit-set's capacity directly — no derived parameter
//! needed, exactly as it already is for `sparse-set`/`static-disjoint-set`.
//!
//! # `rank` was tried and pulled — a real finding, not a rejected idea
//!
//! The first draft used `rank` as the fourth op, on the theory that it is the
//! closest thing this module has to `static-disjoint-set`'s `find`. It is not
//! a fair comparison: `rank(i)` has **no rank/select index** behind it in
//! upstream (`bench/upstream/bit-set.js`) or in the port
//! (`mnemonist-core::structures::bits::rank`) — both sum popcounts word by
//! word from the start, so a single call costs O(i / 32) words. At this
//! module's 1e6 domain, a 25%-weighted mix put ~250,000 calls averaging
//! ~15,000 word-scans each into *every measured pass*, and the harness was
//! still running after ten minutes and six of ten reps. That is not a
//! representative bit-set workload; it is a benchmark of the domain-size
//! parameter. `rank`'s real cost is worth measuring, but as its own dedicated
//! single-op workload with a size sweep — not mixed uniformly into a bulk
//! op-stream where 1/4 of the ops are each individually as expensive as the
//! other 3/4 combined.

use mnemonist_core::structures::bit_set::BitSet;

use crate::workload::Workload;

/// One measured pass: fresh set, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut set = BitSet::new(workload.size as usize);

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let index = i64::from(workload.a[i]);

            match workload.kind[i] {
                0 => set.set(index),
                1 => set.reset(index),
                2 => checksum += u64::from(set.get(index)),
                _ => checksum += u64::from(set.test(index)),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&set);

    (batches, checksum)
}

/// `--structure`: preallocate a `size`-bit set and touch it. `BitSet::new`
/// allocates its whole word array up front, so this isolates exactly that
/// allocation from the ~9 MB of op arrays the mixed workload also carries —
/// same rationale as the two existing modules' `--structure` mode.
pub fn build_structure(size: u32) {
    let set = BitSet::new(size as usize);

    std::hint::black_box(&set);
    std::hint::black_box(set.get(i64::from(size) - 1));
}

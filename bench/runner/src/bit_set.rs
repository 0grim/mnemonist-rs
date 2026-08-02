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

/// A bare `Vec<u32>` word array, `usize` indices, `set_bit`/`get_bit` inlined
/// directly at the call site — the counterfactual this module's own bench doc
/// names as unconfirmed: does the extra call frame through
/// `Words::set_bit`/`get_bit` (`crates/mnemonist-core/src/structures/bits.rs`)
/// cost anything once the `Rc<RefCell<Vec<u32>>>` borrow-flag check and the
/// `i64`-with-real-`ToInt32` split both go away too? This workload never
/// produces a negative or out-of-range index (`workload.a[i]` is drawn
/// `below(size)`), so the `usize` shortcut here is faithful to what actually
/// runs, not a narrower test than the real one.
struct BareBitSet {
    words: Vec<u32>,
}

impl BareBitSet {
    fn new(length: usize) -> Self {
        Self {
            words: vec![0u32; length.div_ceil(32)],
        }
    }

    fn set(&mut self, index: usize) {
        self.words[index / 32] |= 1u32 << (index % 32);
    }

    fn reset(&mut self, index: usize) {
        self.words[index / 32] &= !(1u32 << (index % 32));
    }

    fn get(&self, index: usize) -> u32 {
        (self.words[index / 32] >> (index % 32)) & 1
    }

    fn test(&self, index: usize) -> bool {
        self.get(index) != 0
    }
}

/// [`BareBitSet`], but with `index` run through the exact `f64`-based
/// `ToInt32` conversion `Words::split` uses
/// (`crates/mnemonist-core/src/structures/bits.rs`,
/// `crate::utils::bitwise::to_int32`) instead of a plain `usize` cast. Still
/// no `RefCell`, still `Vec<u32>` rather than `Words`. Exists to split
/// [`BareBitSet`] vs `BitSet`'s gap into its two candidate causes: the
/// `RefCell` borrow-flag check this module's doc names, and the
/// `to_int32`/`rem_euclid` float conversion `split` does on every call, which
/// the doc does not mention at all.
struct BareBitSetToInt32 {
    words: Vec<u32>,
}

impl BareBitSetToInt32 {
    fn new(length: usize) -> Self {
        Self {
            words: vec![0u32; length.div_ceil(32)],
        }
    }

    fn split(index: i64) -> (usize, u32) {
        let index = mnemonist_core::utils::bitwise::to_int32(index as f64);
        ((index >> 5) as usize, (index & 0x1f) as u32)
    }

    fn set(&mut self, index: i64) {
        let (word, pos) = Self::split(index);
        self.words[word] |= 1u32 << pos;
    }

    fn reset(&mut self, index: i64) {
        let (word, pos) = Self::split(index);
        self.words[word] &= !(1u32 << pos);
    }

    fn get(&self, index: i64) -> u32 {
        let (word, pos) = Self::split(index);
        (self.words[word] >> pos) & 1
    }

    fn test(&self, index: i64) -> bool {
        self.get(index) != 0
    }
}

/// The [`BareBitSetToInt32`] counterpart to [`run_mixed`]/[`run_mixed_bare`].
/// Not part of `harness::MODULES`, same reasoning as the other two.
pub fn run_mixed_bare_to_int32(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut set = BareBitSetToInt32::new(workload.size as usize);

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

/// The bare counterpart to [`run_mixed`], same op mix and same op stream —
/// see `main.rs`'s `--bit-set-probe`. Not part of `harness::MODULES`, same
/// reasoning as `sparse_set.rs::run_mixed_refcell` and `heap.rs::run_mixed_bare`.
pub fn run_mixed_bare(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut set = BareBitSet::new(workload.size as usize);

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let index = workload.a[i] as usize;

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

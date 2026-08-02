//! The `hashed-array-tree` mixed workload — a growable array built from fixed-
//! size blocks rather than one reallocated buffer, so growth appends a block
//! and never copies. `vector`'s structural sibling: same op shape, different
//! growth policy underneath.
//!
//! Op mix: 50% `push` (mutating, growth) / 25% `get` at a uniformly random
//! *existing* index (pure read, modulo the current length so it never lands
//! past it — the growth-boundary reads `get(length)` admits, per the module's
//! own docs, belong to the differential fuzzer, not this benchmark) / 25%
//! `pop` (mutating and a read; upstream's wrong-block defect makes the
//! *value* unreliable across a block boundary, but it is still a real memory
//! read costing the same either way, and the checksum only needs both sides
//! to agree with each other — which they do, bug-for-bug).
//!
//! `workload.size` only bounds the magnitude of pushed values, as it does for
//! `vector`: a fresh tree starts at length 0, capacity 0, and grows one block
//! at a time under `push` alone.

use mnemonist_core::structures::hashed_array_tree::{HashedArrayTree, Options};
use mnemonist_core::utils::typed_arrays::PointerWidth;

use crate::workload::Workload;

/// One measured pass: fresh tree, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut tree = HashedArrayTree::new(PointerWidth::U32, Options::default())
        .expect("the default block size is a power of two");

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            match workload.kind[i] {
                0 | 1 => {
                    tree.push(workload.a[i]);
                }
                2 => {
                    let len = tree.length();

                    if len > 0 {
                        let index = (workload.a[i] as usize) % len;
                        checksum += u64::from(tree.get(index).ok().flatten().unwrap_or(0));
                    }
                }
                _ => {
                    checksum += u64::from(tree.pop().unwrap_or(0));
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&tree);

    (batches, checksum)
}

/// `--structure`: like `vector`, no capacity distinct from the pushed length
/// once growing under its own policy, so "size" means "pushed `size`
/// elements".
pub fn build_structure(size: u32) {
    let mut tree = HashedArrayTree::new(PointerWidth::U32, Options::default())
        .expect("the default block size is a power of two");

    for i in 0..size {
        tree.push(i);
    }

    std::hint::black_box(&tree);
}

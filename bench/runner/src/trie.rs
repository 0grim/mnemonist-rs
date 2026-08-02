//! The `trie` mixed workload — string keys over a shared `Trie<char>`, chosen
//! because it is allocation-heavy and its per-op cost (a node walk
//! proportional to key length, fanning out through a hash map per node) has a
//! shape nothing else in this batch has.
//!
//! # Keys are hex, not a bespoke string generator
//!
//! Every op needs a string and the shared workload only hands out `u32`s.
//! Minting a second matched string generator would be new shared machinery,
//! and `CLAUDE.md` is explicit that two agents solving the same sub-problem
//! twice has already cost real time here — so instead each key is
//! `format!("{value:x}")` on the Rust side and `value.toString(16)` in JS:
//! both lowercase hex with no leading zeros, therefore byte-identical for the
//! same `u32`, with zero risk of the two sides' generators drifting apart.
//! This also gives the pool genuine prefix-sharing for free — every value
//! under `0x1000` shares its leading digits with thousands of others — without
//! a second RNG or a dictionary file.
//!
//! Op mix, the same shape as `sparse-set`: 50% `add` (mutating, no checksum
//! contribution), 25% `has` (pure read), 25% `delete` (mutating read,
//! contributing the boolean).

use mnemonist_core::structures::trie::Trie;

use crate::workload::Workload;

fn key(value: u32) -> String {
    format!("{value:x}")
}

/// One measured pass: fresh trie, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut trie: Trie<char> = Trie::new();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let word = key(workload.a[i]);

            match workload.kind[i] {
                0 | 1 => {
                    trie.add(word.chars());
                }
                2 => checksum += u64::from(trie.has(word.chars())),
                _ => checksum += u64::from(trie.delete(word.chars())),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&trie);

    (batches, checksum)
}

/// `--structure`: like `heap`/`vector`, a `Trie` has no separate capacity
/// from occupied size — it grows node by node. "size" here means "prefilled
/// with `size` distinct hex keys", isolating the structure's own allocation
/// footprint from the ~9 MB of op arrays the mixed workload also carries.
pub fn build_structure(size: u32) {
    let mut trie: Trie<char> = Trie::new();

    for i in 0..size {
        trie.add(key(i).chars());
    }

    std::hint::black_box(&trie);
}

//! The `trie-map` mixed workload — same hex-key, prefix-sharing shape as
//! `trie.rs`, reused rather than re-derived (CLAUDE.md: two agents solving
//! the same sub-problem twice has already cost real time here). The only
//! difference from `trie.rs` is that every stored word carries a value, so
//! `set`/`get` replace `add`/`has` and `delete` returns the displaced value
//! rather than a boolean.
//!
//! # Keys are hex, values are the same `u32`
//!
//! `format!("{value:x}")` on the Rust side, `value.toString(16)` in JS —
//! byte-identical for the same `u32`, and genuinely prefix-sharing (every
//! value under `0x1000` shares its leading digits with thousands of others)
//! without a second RNG or a dictionary file. See `trie.rs`'s own docs for
//! the full account of why this generator was reused rather than invented
//! twice.
//!
//! Op mix, `trie`'s own shape: 50% `set` (mutating, no checksum
//! contribution), 25% `get` (pure read, contributing the stored `u32`),
//! 25% `delete` (mutating read, contributing a boolean).
//!
//! `delete` contributes `is_some()` as a boolean, not the removed value
//! itself: upstream's `TrieMap#.delete` returns a plain boolean (`bench/
//! upstream/trie-map.js`), while core's `TrieMap::delete` returns
//! `Option<V>` so callers who need the displaced value (the N-API bridge, to
//! release a JS reference) can have it. Checksumming the `Option`'s payload
//! here would make the two sides agree on the *count* of successful deletes
//! but not necessarily the *value* summed, since a JS boolean and a Rust
//! `u32` are not the same quantity — matching upstream's own return shape is
//! what keeps the checksum a proof of "same ops, same answers" rather than an
//! artifact of which side happens to expose more information.

use mnemonist_core::structures::trie_map::TrieMap;

use crate::workload::Workload;

fn key(value: u32) -> String {
    format!("{value:x}")
}

/// One measured pass: fresh trie, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut trie: TrieMap<char, u32> = TrieMap::new();

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
                    trie.set(word.chars(), workload.a[i]);
                }
                2 => {
                    if let Some(value) = trie.get(word.chars()) {
                        checksum += u64::from(*value);
                    }
                }
                _ => checksum += u64::from(trie.delete(word.chars()).is_some()),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&trie);

    (batches, checksum)
}

/// `--structure`: same convention as `trie.rs` — "size" means "prefilled with
/// `size` distinct hex keys", isolating the structure's own footprint from
/// the ~9 MB of op arrays the mixed workload also carries.
pub fn build_structure(size: u32) {
    let mut trie: TrieMap<char, u32> = TrieMap::new();

    for i in 0..size {
        trie.set(key(i).chars(), i);
    }

    std::hint::black_box(&trie);
}

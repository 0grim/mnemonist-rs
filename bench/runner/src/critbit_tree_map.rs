//! The `critbit-tree-map` mixed workload — string keys over a shared
//! `CritBitTreeMap<u32>`, chosen so the critical-bit machinery
//! (`msb8`/`mask_for`/`get_direction`) actually has to walk several internal
//! nodes per operation rather than branching on the very first byte.
//!
//! # Keys are zero-padded decimal, not bare `to_string()`
//!
//! A crit-bit tree branches on the position of the **first** bit two keys
//! disagree on. Bare `value.to_string()` keys of varying length (`"7"` vs
//! `"1234"`) would make most pairs diverge at byte 0 — the tail-vs-implicit-0
//! branch (`find_critical_bit`'s doc comment) firing on almost every
//! comparison, which exercises the *shortest* path through the tree rather
//! than a representative one. Zero-padding every key to the same width
//! (`format!("{value:06}")`, six decimal digits — this batch's 200,000-key
//! domain never exceeds six) forces every pair through genuine byte-by-byte
//! comparison: the leading digits repeat across most of the domain (every
//! key under 100,000 shares its leading `0`), so the bit where two keys
//! actually diverge sits in the **low-order** digits, deep into the key
//! rather than at the front — see `bench/methodology.md`'s own account of
//! why a workload has to be checked against the shape of the algorithm it
//! is meant to exercise, not just against its op names.
//!
//! Op mix, `sparse-map`/`trie-map`'s shape: 50% `set` (mutating), 25% `get`
//! (pure read), 25% `delete` (mutating, contributing upstream's own plain
//! boolean — `CritBitTreeMap::delete` returns `Option<V>`, matching
//! `trie_map.rs`'s reasoning for the same divergence).

use mnemonist_core::structures::critbit_tree_map::CritBitTreeMap;

use crate::workload::Workload;

fn key(value: u32) -> String {
    format!("{value:06}")
}

/// One measured pass: fresh map, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut map: CritBitTreeMap<u32> = CritBitTreeMap::new();

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
                    map.set(k, workload.a[i]);
                }
                2 => {
                    if let Some(value) = map.get(k.as_bytes()) {
                        checksum += u64::from(*value);
                    }
                }
                _ => checksum += u64::from(map.delete(k.as_bytes()).is_some()),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&map);

    (batches, checksum)
}

/// `--structure`: "size" means "prefilled with `size` distinct zero-padded
/// decimal keys", isolating the structure's own footprint from the ~9 MB of
/// op arrays the mixed workload also carries.
pub fn build_structure(size: u32) {
    let mut map: CritBitTreeMap<u32> = CritBitTreeMap::new();

    for i in 0..size {
        map.set(key(i), i);
    }

    std::hint::black_box(&map);
}

//! The `default-map` mixed workload — `sparse-map`'s shape over a real
//! `HashMap`-backed structure instead of a fixed-domain array.
//!
//! Op mix: 50% `set(key, value)` (mutating, no checksum contribution), 25%
//! `get_or_insert_with(key, factory)` (mutating **and** a read — the same
//! "mutates and reads" shape as `lru-cache`'s `get`/`static-disjoint-set`'s
//! `find`, since a miss both manufactures a value and stores it), 25%
//! `delete(key)` (pure mutation with a boolean result, contributing 1/0).
//!
//! `IK = K = V = u32`, matching `sparse-map`/`bit-set`: only strings and
//! numbers ever reach this family's index upstream, and a bare integer is the
//! simplest faithful instance (see `lru_cache.rs`'s own module docs for the
//! same choice).
//!
//! # Domain — the full key space, not a fraction of it
//!
//! Unlike `lru-cache`, `DefaultMap` has no eviction and no separate notion of
//! "capacity" distinct from how many keys have been written — it is a plain
//! hash map that grows with what is stored in it. So `workload.size` is read
//! directly as the key domain, exactly as `sparse-map`/`bit-set` do: there is
//! no separate index to rig here, because there is only one.
//!
//! The factory always returns `Some`, so B-40's `size` drift (a stored
//! `undefined` making the counter diverge from the live entry count) is never
//! triggered by this workload — that divergence is exhaustively covered by
//! the differential fuzzer and the unit's own native tests, where it belongs;
//! a benchmark whose checksum depended on a drifting counter would not be
//! reproducible run to run.

use mnemonist_core::structures::default_map::DefaultMap;

use crate::workload::Workload;

/// One measured pass: fresh map, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut map: DefaultMap<u32, u32> = DefaultMap::new();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let key = workload.a[i];

            match workload.kind[i] {
                0 | 1 => {
                    map.set(key, Some(workload.b[i]));
                }
                2 => {
                    checksum += u64::from(
                        *map.get_or_insert_with(key, |k, _size| Some(*k))
                            .unwrap_or(&0),
                    );
                }
                _ => checksum += u64::from(map.delete(&key).is_some()),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&map);

    (batches, checksum)
}

/// `--structure`: prefill `size` distinct keys and touch the map. `DefaultMap`
/// has no preallocated capacity to isolate (unlike `sparse-map`/`bit-set`),
/// so — like `heap`/`trie`/`vector` — "size" means "prefilled with `size`
/// elements" here.
pub fn build_structure(size: u32) {
    let mut map: DefaultMap<u32, u32> = DefaultMap::new();

    for i in 0..size {
        map.set(i, Some(i));
    }

    std::hint::black_box(&map);
    std::hint::black_box(map.has(&(size - 1)));
}

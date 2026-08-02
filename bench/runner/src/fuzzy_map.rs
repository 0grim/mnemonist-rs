//! The `fuzzy-map` mixed workload — `default-map`'s shape, minus the factory
//! (this module has no `get`-time auto-insert, see `docs/modules/
//! fuzzy-map.md`), with every key passed through a hash function before it
//! ever reaches the backing map.
//!
//! # The hash function has to do the identical work on both sides
//!
//! Core never hashes — hashing is a JavaScript callback applied by the
//! bridge, exactly as `default-map`'s factory is (`fuzzy_map.rs`'s own module
//! docs). Benchmarking this module directly against `mnemonist-core` (never
//! through N-API, per `methodology.md`) means the hash has to be reproduced
//! as a **plain Rust closure** on this side and a **plain JS function** on
//! `bench/node/run.js`'s side — and if the two ever computed different
//! things, this benchmark would measure the hash function, not the
//! structure, exactly the trap the brief calls out by name.
//!
//! So the hash here is deliberately the simplest possible operation that
//! still produces real collisions: `hash(x) = x >> 4`, an arithmetic right
//! shift. Rust's `u32 >> 4` and JavaScript's `x >>> 4` compute the identical
//! bit pattern for every non-negative 32-bit value with no floating-point
//! rounding to keep in sync (unlike, say, a modulo-based hash, which would
//! still match but adds nothing this shift does not already prove) — the
//! same "no floating point, exact bitwise match" property `xorshift.rs`
//! itself already depends on for the matched PRNG. It collapses 16 raw keys
//! onto one hashed slot, which is what makes `set`/`get`/`has` actually
//! exercise `FuzzyMap`'s reason to exist (several distinct queries resolving
//! to the same stored item) rather than behaving like a plain `Map` that
//! happens to relabel its keys 1:1.
//!
//! Op mix: 50% `set(hash(key), value)` (mutating, no checksum contribution),
//! 25% `get(hash(key))` (pure read, contributing the value or 0), 25%
//! `has(hash(key))` (pure read, contributing the boolean) — `has` stands in
//! for `sparse-map`'s `delete` slot, since this module has no delete at all
//! (`docs/modules/fuzzy-map.md`: "clear is the only way to shrink a
//! FuzzyMap").
//!
//! `workload.size` is the **raw** key domain (1,000,000, matching the other
//! 1e6 workloads in this batch), not the post-hash one — the hash's own 16:1
//! collapse is what produces the collision-heavy access pattern, so no
//! separate small domain needs to be chosen by hand the way the multi-
//! container modules in this batch choose one explicitly.

use mnemonist_core::structures::fuzzy_map::FuzzyMap;

use crate::workload::Workload;

/// `hash(x) = x >> 4`. See the module docs for why this exact function and
/// not something floating-point-based.
fn hash(x: u32) -> u32 {
    x >> 4
}

/// One measured pass: fresh map, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut map: FuzzyMap<u32, u32> = FuzzyMap::new();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let key = hash(workload.a[i]);

            match workload.kind[i] {
                0 | 1 => {
                    map.set(key, Some(workload.b[i]));
                }
                2 => checksum += u64::from(*map.get(&key).unwrap_or(&0)),
                _ => checksum += u64::from(map.has(&key)),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&map);

    (batches, checksum)
}

/// `--structure`: prefill `size` distinct HASHED keys and touch the map.
/// Fills through the same `hash` function the mixed workload uses, so the
/// structure reflects the post-collapse key count, not the raw domain.
pub fn build_structure(size: u32) {
    let mut map: FuzzyMap<u32, u32> = FuzzyMap::new();

    for i in 0..size {
        map.set(hash(i), Some(i));
    }

    std::hint::black_box(&map);
    std::hint::black_box(map.has(&hash(size - 1)));
}

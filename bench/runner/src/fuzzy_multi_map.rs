//! The `fuzzy-multi-map` mixed workload — `fuzzy-map`'s hashed-key shape,
//! wrapping a `MultiMap` instead of a `Map`, so this is also a
//! multi-container: the load-bearing "values per key" parameter applies here
//! too, and it is the hash's own collapse that produces it (see below).
//!
//! Same hash as `fuzzy_map.rs`, for the same reason (has to do the identical
//! work on both sides — see that file's module docs): `hash(x) = x >> 4`.
//!
//! Op mix: 50% `set(hash(key), value)` (mutating append — upstream's own
//! `set` returns `this`; this module has no `delete`/`remove` at all, so a
//! bucket only ever grows, the same shape as `multi-array`), 25%
//! `get(hash(key))` (pure read, contributing the bucket's length), 25%
//! `has(hash(key))` (pure read, contributing the boolean).
//!
//! `ContainerKind::List`, matching upstream's default (`new
//! FuzzyMultiMap(descriptor)` with no `Container` argument resolves to
//! `Array`) — same choice `multi_map.rs` makes and for the identical reason.
//!
//! # Where the "values per key" comes from here
//!
//! Unlike `multi-map`/`multi-array`, this module does not need a hand-picked
//! small key domain — the hash's 16:1 collapse (`x >> 4`) already produces
//! one. `workload.size` is 200,000 raw keys (smaller than `fuzzy-map`'s
//! 1,000,000: chosen so the post-hash domain, 200,000 / 16 = 12,500, keeps
//! bucket depth in a representative range without the raw domain being large
//! enough to make every hashed slot's bucket implausibly deep). At 50% `set`
//! over 1,000,000 ops, that is 500,000 appends over ~12,500 distinct hashed
//! keys: **~40 values per bucket on average** by the run's end — a real
//! multi-valued structure, not a map with a collapsed key space and buckets
//! of length 1.

use mnemonist_core::structures::fuzzy_multi_map::FuzzyMultiMap;
use mnemonist_core::structures::multi_map::ContainerKind;

use crate::workload::Workload;

/// `hash(x) = x >> 4`. Identical function to `fuzzy_map.rs::hash` — see that
/// file's module docs for why this exact shape.
fn hash(x: u32) -> u32 {
    x >> 4
}

/// One measured pass: fresh map, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut map: FuzzyMultiMap<u32, u32> = FuzzyMultiMap::new(ContainerKind::List);

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let key = hash(workload.a[i]);
            let value = workload.b[i];

            match workload.kind[i] {
                0 | 1 => {
                    let outcome: Result<Option<u32>, std::convert::Infallible> =
                        map.set_with(key, value, |a, b| Ok(a == b));

                    outcome.expect("List-kind buckets never invoke the equality closure");
                }
                2 => checksum += map.get(&key).map_or(0, |bucket| bucket.len() as u64),
                _ => checksum += u64::from(map.has(&key)),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&map);

    (batches, checksum)
}

/// `--structure`: prefill `size` distinct HASHED keys, one value each, and
/// touch the map. Same "one value per key" convention as
/// `multi-map.rs::build_structure`, through the same `hash` the mixed
/// workload uses.
pub fn build_structure(size: u32) {
    let mut map: FuzzyMultiMap<u32, u32> = FuzzyMultiMap::new(ContainerKind::List);

    for i in 0..size {
        let outcome: Result<Option<u32>, std::convert::Infallible> =
            map.set_with(hash(i), i, |a, b| Ok(a == b));

        outcome.expect("List-kind buckets never invoke the equality closure");
    }

    std::hint::black_box(&map);
    std::hint::black_box(map.has(&hash(size - 1)));
}

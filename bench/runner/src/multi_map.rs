//! The `multi-map` mixed workload — the first genuine multi-container in this
//! batch, where the load-bearing parameter is how many values sit under one
//! key, not just how many keys exist.
//!
//! Op mix: 50% `set(key, value)` (mutating append, no checksum contribution
//! — matches upstream, whose `set` returns `this`), 25% `get(key)` (pure
//! read, contributing the bucket's length — 0 for a key never set), 25%
//! `remove(key, value)` (mutating, contributing the boolean result).
//!
//! `ContainerKind::List`, matching upstream's default (`new MultiMap()`
//! resolves `this.Container` to `Array`) — the `Set`-kind path is a linear
//! scan behind a supplied equality (`docs/modules/multi-map.md`) and is not
//! what this default construction exercises.
//!
//! # `size` is the key domain, deliberately far smaller than the op count
//!
//! Every other module in the first two Gate 10 batches either has a fixed
//! domain equal to its capacity (`bit-set`, `sparse-map`) or grows without a
//! separate key space to rig (`heap`, `vector`). `MultiMap` is different: the
//! key domain and "how many values live under each key" are two independent
//! knobs, and only the second one is what makes this a multi-container
//! benchmark rather than a map-with-extra-indirection one — a workload where
//! every key holds exactly one value would produce a clean, meaningless
//! number, per the brief.
//!
//! So `workload.size` here is read as the **key domain** (20,000 — the
//! `mixed-1e6` workload below), not the op count. With 1,000,000 ops at
//! 50% `set`, that is 500,000 appends spread over 20,000 keys: **25 values
//! per key on average** by the run's end, a bucket depth this benchmark
//! actually reaches rather than merely claims. `remove`'s linear scan (of
//! whichever bucket its key hits) is therefore genuinely exercised at a
//! representative depth, not against buckets that are all length 1 — and its
//! cost scaling with bucket depth is a property `multi-map.js`'s own
//! `Array.indexOf` shares, not a port-only pathology (the `bit-set.rs` `rank`
//! trap `methodology.md` warns about is an op whose cost the *other* side
//! does not pay at all; here upstream pays the identical linear scan).

use mnemonist_core::structures::multi_map::{ContainerKind, MultiMap};

use crate::workload::Workload;

/// One measured pass: fresh map, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut map: MultiMap<u32, u32> = MultiMap::new(ContainerKind::List);

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let key = workload.a[i];
            let value = workload.b[i];

            match workload.kind[i] {
                0 | 1 => map.set(key, value),
                2 => checksum += map.get(&key).map_or(0, |bucket| bucket.len() as u64),
                _ => checksum += u64::from(map.remove(key, &value)),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&map);

    (batches, checksum)
}

/// `--structure`: prefill `size` distinct one-value buckets and touch the
/// map. Deliberately one value per key here — the extra values per key the
/// mixed workload reaches are an emergent property of running the op stream,
/// not a fixed per-key overhead this isolation needs to reproduce (the same
/// call `lru-cache.rs::build_structure` makes for its own derived capacity).
pub fn build_structure(size: u32) {
    let mut map: MultiMap<u32, u32> = MultiMap::new(ContainerKind::List);

    for i in 0..size {
        map.set(i, i);
    }

    std::hint::black_box(&map);
    std::hint::black_box(map.has(&(size - 1)));
}

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
//! The factory always returns `Some`, so BUG-DEFAULT-MAP-1's `size` drift (a stored
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

/// Isolates `get_or_insert_with`'s **hit path** — `OrderedMap::slot_of` then
/// `OrderedMap::entry_at` (`DefaultMap::try_get_or_insert_with`) — against a
/// single-hash-lookup baseline, to test this module's own bench doc's
/// "unconfirmed" hypothesis that the hit path costs two hash lookups rather
/// than one. Every key is prefilled before the timed region and every op here
/// reads one back, so the factory never runs and no insert ever happens: both
/// [`run_probe_peek`] and [`run_probe_hit`] below measure a pure read.
///
/// Reading `OrderedMap::entry_at` (`crates/mnemonist-core/src/map/mod.rs`)
/// shows it is `self.slots.get(slot)` — a plain `Vec` index, not a second
/// `HashMap::get` — so the two hash lookups the doc hypothesises should not
/// exist by construction; [`peek`](DefaultMap::peek), which is exactly one
/// `OrderedMap::get` (one hash lookup, one slot read), is the baseline this
/// probe checks that reading against. Not part of `harness::MODULES` — no
/// upstream JS analogue of "call peek instead of get-or-insert" to publish a
/// `regressions` array against. Call with `--default-map-probe`.
pub fn run_probe_peek(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut map: DefaultMap<u32, u32> = DefaultMap::new();

    for i in 0..workload.size {
        map.set(i, Some(i));
    }

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            // `workload.a[i]` is already drawn `below(workload.size)`, so
            // every key here was prefilled above -- this is a hit, always.
            let key = workload.a[i];
            checksum += u64::from(*map.peek(&key).expect("every key was prefilled"));
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&map);

    (batches, checksum)
}

/// The `get_or_insert_with` half of the same comparison — see
/// [`run_probe_peek`]. The factory is `unreachable!()`: every key was
/// prefilled above, so this exercises only the hit path
/// (`slot_of` + `entry_at`), never the miss path (factory + `set`).
pub fn run_probe_hit(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut map: DefaultMap<u32, u32> = DefaultMap::new();

    for i in 0..workload.size {
        map.set(i, Some(i));
    }

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let key = workload.a[i];
            let value = map.get_or_insert_with(key, |_, _| {
                unreachable!("every key was prefilled; the probe's hit path never inserts")
            });
            checksum += u64::from(*value.expect("every key was prefilled"));
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&map);

    (batches, checksum)
}

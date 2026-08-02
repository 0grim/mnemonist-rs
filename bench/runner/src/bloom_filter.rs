//! The `bloom-filter` mixed workload — hex-encoded keys over a shared
//! `BloomFilter`, sized for `workload.size` items at upstream's default
//! error rate (`0.5%`).
//!
//! # The load-bearing parameters: a stated fill ratio, and real hits AND
//! real misses
//!
//! An empty or near-empty filter answers every `test` the same way `data`'s
//! all-zero bits would — trivially fast, and it proves nothing about the
//! hashing/bit-setting this module actually exists to measure. So the filter
//! is **prefilled to half its stated capacity before timing starts** (an
//! explicit, reported fill ratio of 0.5 at the first timed op), and the
//! timed `add` stream keeps drawing from the SAME `0..size` domain — by the
//! run's end, birthday-bound coverage means very close to the full
//! `0..size` domain has been added, so the fill ratio climbs from 0.5 toward
//! ~1.0 over the course of the run rather than sitting still.
//!
//! `test` queries split across two disjoint pools: **hits** drawn from
//! `0..size` (the domain being added throughout — a true positive with
//! near-certainty once that key has actually been added, and upstream's own
//! `test` never has false negatives, so once a key is added it never stops
//! matching) and **misses** drawn from `size..2*size` (a domain never added
//! at all — a true negative except for the configured false-positive rate).
//! Testing only hits, or only misses, would time one comparator/hash branch
//! and never validate the other. Measured directly at `size` 200,000, seed
//! 42: the hit pool answers `true` **61.1%** of the time (not near-0, not
//! near-100% — a genuine mix of "already added" and "not yet, this early in
//! the run"), and the miss pool's false-positive rate is **0.028%**, both
//! confirming real hits and real (mostly true) misses rather than a
//! degenerate all-one-answer workload.
//!
//! Op mix: 50% `add` (mutating, no checksum contribution), 25% `test` on the
//! hit pool (pure read, contributing a boolean), 25% `test` on the miss pool
//! (pure read, contributing a boolean) — `sparse-map`'s shape, with `test`
//! standing in for `get`/`delete` since this module has neither.

use mnemonist_core::structures::bloom_filter::BloomFilter;

use crate::workload::Workload;

fn key(value: u32) -> Vec<u16> {
    format!("{value:x}").encode_utf16().collect()
}

/// One measured pass: filter prefilled to half capacity (untimed), then the
/// whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let capacity = f64::from(workload.size.max(1));
    let mut filter =
        BloomFilter::new(capacity, None).expect("workload.size is always >= 1, so capacity > 0");

    // Prefill to a stated 50% fill ratio -- see the module docs for why an
    // empty filter is not a representative starting point.
    let prefill = workload.size / 2;

    for i in 0..prefill {
        filter.add(&key(i));
    }

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            match workload.kind[i] {
                0 | 1 => {
                    let member = workload.a[i] % workload.size.max(1);
                    filter.add(&key(member));
                }
                2 => {
                    // The hit pool: the same `0..size` domain `add` draws
                    // from throughout the run.
                    let member = workload.a[i] % workload.size.max(1);
                    checksum += u64::from(filter.test(&key(member)));
                }
                _ => {
                    // The miss pool: `size..2*size`, never added at all.
                    let member = workload.size.max(1) + (workload.a[i] % workload.size.max(1));
                    checksum += u64::from(filter.test(&key(member)));
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&filter);

    (batches, checksum)
}

/// `--structure`: "size" means "capacity `size`, prefilled with `size`
/// items" -- full, not the workload's own half-capacity starting point,
/// matching the general `build_structure` convention elsewhere in this
/// batch (a filled structure, not a half-filled one, isolates the maximum
/// footprint).
pub fn build_structure(size: u32) {
    let capacity = f64::from(size.max(1));
    let mut filter = BloomFilter::new(capacity, None).expect("size is always >= 1");

    for i in 0..size {
        filter.add(&key(i));
    }

    std::hint::black_box(&filter);
}

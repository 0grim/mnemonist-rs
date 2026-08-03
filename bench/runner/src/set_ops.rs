//! The `set` benchmark — `set.js`'s fourteen free functions over
//! `OrderedSet`s (`crate::structures::set` in core; `bench/upstream/set.js`
//! upstream) have no instance and no per-element op stream, so — like
//! `sort`/`suffix-array` — this reuses the `drain` shape: one measured
//! sample per **call**, not per batch of 1000.
//!
//! # Why `union`, out of fourteen candidates
//!
//! `union` is the representative choice: it is the operation every one of
//! the other thirteen (`intersection`, `difference`, `jaccard`, …) is
//! defined in terms of touching *every* member of *both* inputs (unlike
//! `intersection`, which only ever walks the smaller one), so it is the one
//! whose cost scales most directly and predictably with the two input
//! sizes — a property worth measuring plainly rather than picking
//! `disjunct`, this module's most intricate function (`crate::structures::
//! set`'s own docs single it out for its three-phase write order), whose
//! extra subtlety is a correctness question already covered by the unit's
//! own tests and fuzzing, not a distinct performance question.
//!
//! # Why two sets drawn from the same domain, not two disjoint ones
//!
//! Both `A` and `B` are `size` elements each, drawn from the SAME `0..size`
//! range. Two independent draws that large from that domain guarantee real
//! overlap (by the birthday bound) and real internal duplicates within each
//! set — so `union`'s own `OrderedSet::add` dedup path is genuinely exercised
//! on both a duplicate-within-A/B and a duplicate-across-A/B basis, rather
//! than benchmarking a `union` of two sets that never overlap at all (which
//! `union` would still handle, just not representatively).
//!
//! # The checksum is position-weighted, not a sum
//!
//! Same reasoning as `sort.rs`: a plain sum of the result's members would be
//! insensitive to *order*, and `union`'s order is exactly what
//! `crate::structures::set`'s own docs pin down (argument order, then
//! insertion order) — see `test/set.js`'s own `Array.from(result)` assertions.
//! Weighting each member by its final position makes the checksum sensitive
//! to that order, so agreement between the two sides is evidence they
//! produced the same *sequence*, not merely the same *membership*.
//!
//! `size = 20,000`, `passes = 50` — `size * passes` at the same ~1e6 order of
//! magnitude as this batch's other workloads, matching `sort.rs`/
//! `suffix_array.rs`'s own reasoning for picking their own `size * passes`.

use mnemonist_core::structures::set::{union, OrderedSet};

use crate::xorshift::XorShift32;

/// One measured sample per `union` call. `size` elements per input set, drawn
/// from the shared `0..size` domain; `passes` fresh pairs unioned. Returns
/// per-pass batch nanoseconds, the position-weighted checksum, and `2 * size`
/// (the number of source elements `union` iterates over each call, constant
/// across passes) so the driver's `ns / batch_k` means nanoseconds per
/// element visited.
pub fn run_drain(size: u32, seed: u32, passes: usize) -> (Vec<u64>, u64, usize) {
    let size = size as usize;
    let mut rng = XorShift32::new(seed);

    // Materialised before any timing -- one draw per element, the same "op
    // array" discipline `bench/methodology.md` asks for, applied to this module's
    // input rather than to an op-kind stream. `2 * size * passes` values:
    // the first half of each pass's slice seeds `A`, the second half `B`.
    let domain = size as u32;
    let buffer: Vec<u32> = (0..2 * size * passes).map(|_| rng.below(domain)).collect();

    let mut batches = Vec::with_capacity(passes);
    let mut checksum: u64 = 0;

    for pass in 0..passes {
        let base = pass * 2 * size;
        let a = OrderedSet::from_members(buffer[base..base + size].iter().copied());
        let b = OrderedSet::from_members(buffer[base + size..base + 2 * size].iter().copied());

        let clock = std::time::Instant::now();
        let result = union(&[&a, &b]).expect("two sets is always at least two arguments");
        batches.push(clock.elapsed().as_nanos() as u64);

        // Outside the timed region: a verification read, not part of what
        // `union` itself costs.
        for (index, member) in result.iter().enumerate() {
            checksum += (index as u64 + 1) * u64::from(*member);
        }
    }

    std::hint::black_box(&buffer);

    (batches, checksum, 2 * size)
}

/// `--structure`: `set.js` has no persistent structure of its own (see the
/// module docs) -- this measures the transient footprint of building one
/// `size`-element `OrderedSet`, the same "nothing left to hold after the
/// call returns" convention `sort.rs::build_structure` documents for its own
/// module.
pub fn build_structure(size: u32) {
    let set = OrderedSet::from_members(0..size);

    std::hint::black_box(&set);
    std::hint::black_box(set.has(&(size - 1)));
}

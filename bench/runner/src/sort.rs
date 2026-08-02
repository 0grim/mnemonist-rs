//! The `sort` benchmark — `inplace_quick_sort`, the only ported *function*
//! rather than a structure in this batch (see `sort/mod.rs`'s own module
//! docs). There is no instance to hold state between calls and therefore no
//! `mixed` op-stream to run: an "op" here is "sort one freshly-generated
//! array", which is why this reuses the `drain` shape — one measured sample
//! per sort, the same convention `sparse-set`'s iteration walk and
//! `suffix-array`'s construction both use, rather than forcing a batch-of-1000
//! shape onto something that is not a stream of cheap per-element calls.
//!
//! # Why quicksort, not insertion sort
//!
//! `sort/mod.rs` ports both. Quicksort is the general-purpose default upstream
//! ships (`inplaceInsertionSort` exists for small windows, e.g. as a
//! sub-routine of a hybrid sort elsewhere in the library) — see
//! `docs/modules/sort.md`. Benchmarking the general-purpose sort is the
//! representative choice; insertion sort's O(n²) behaviour would need a much
//! smaller `size` to stay in budget and would not be measuring what most
//! callers of this module actually use.
//!
//! # Every pass sorts *fresh* random data — this is not a detail
//!
//! Upstream's quicksort picks its pivot from a fixed position (see
//! `quick.rs`'s own docs on why this is transcribed statement by statement).
//! A naive fixed-pivot quicksort's worst case is already-sorted or
//! reverse-sorted input, and re-sorting the *previous pass's output* would
//! feed exactly that worst case into every pass after the first — silently
//! turning an O(n log n) benchmark into an O(n²) one, the same shape of
//! mistake `bit_set.rs`'s `rank` was. So the whole `size * passes` buffer of
//! random values is materialised **once, before any timing**, and each pass
//! sorts its own disjoint, never-before-sorted slice.
//!
//! # The checksum is position-weighted, not a sum
//!
//! Summing the sorted values would be true regardless of whether the sort
//! did anything at all, since sorting cannot change a multiset's sum. Upstream's
//! quicksort is not stable, so which of two equal elements ends up on the left
//! is a property of the algorithm's exact partitioning, not of "sorted"-ness —
//! see `quick.rs`'s own docs: "the permutation is observable, so the algorithm
//! is the contract." Weighting each value by its final index makes the
//! checksum sensitive to that permutation, so an agreement between the two
//! sides is evidence they ran the same statement-by-statement algorithm, not
//! merely that both produced *a* sorted array.
//!
//! `size` is chosen large enough that comparisons dominate the fixed per-call
//! cost, and `size * passes` is kept at the same 1e6-element order of
//! magnitude as this batch's other workloads for comparable generation cost.

use mnemonist_core::sort::quick::inplace_quick_sort;

use crate::xorshift::XorShift32;

/// Values are drawn from `0..VALUE_BOUND`, a domain large enough that
/// duplicates are rare (not the near-certain-duplicate small alphabet
/// `suffix-array` deliberately picks) — this benchmark is about comparison
/// cost, not about exercising the tie-breaking behaviour the checksum above
/// already accounts for.
const VALUE_BOUND: u32 = 1_000_000;

/// One measured sample per sort. `size` elements per pass, `passes` fresh
/// arrays sorted; returns per-pass batch nanoseconds, the position-weighted
/// checksum, and `size` itself (so the driver's `ns / batch_k` means
/// nanoseconds per element, matching every other drain-shaped workload).
pub fn run_drain(size: u32, seed: u32, passes: usize) -> (Vec<u64>, u64, usize) {
    let size = size as usize;
    let mut rng = XorShift32::new(seed);

    // Materialised before any timing, one draw per element -- exactly the
    // "op array" discipline DESIGN.md 5.1 asks for, applied to sort's input
    // rather than to an op-kind stream.
    let mut buffer: Vec<u32> = (0..size * passes).map(|_| rng.below(VALUE_BOUND)).collect();

    let mut batches = Vec::with_capacity(passes);
    let mut checksum: u64 = 0;

    for pass in 0..passes {
        let chunk = &mut buffer[pass * size..(pass + 1) * size];

        let clock = std::time::Instant::now();
        inplace_quick_sort(chunk, 0, chunk.len());
        batches.push(clock.elapsed().as_nanos() as u64);

        // Outside the timed region: this is a verification read, not part of
        // what the sort itself costs.
        for (index, value) in chunk.iter().enumerate() {
            checksum += (index as u64 + 1) * u64::from(*value);
        }
    }

    std::hint::black_box(&buffer);

    (batches, checksum, size)
}

/// `--structure`: sort has no persistent structure -- see the module docs.
/// This measures the transient footprint of allocating and sorting one
/// `size`-element array, which is a different kind of number from the other
/// eleven units' `structure_rss_delta_mb` (there is nothing left to hold
/// after the call returns): it isolates sort's own allocation (the input
/// buffer; the 64-slot partition stack in `quick.rs` is stack-allocated and
/// therefore invisible to RSS) from the ~9 MB of unrelated op arrays the
/// mixed-workload RSS figures carry for every other module. Reported for
/// harness uniformity; read it as "memory to hold and sort `size` elements",
/// not as "size of the sort structure".
pub fn build_structure(size: u32) {
    let mut rng = XorShift32::new(42);
    let mut buffer: Vec<u32> = (0..size).map(|_| rng.below(VALUE_BOUND)).collect();

    let length = buffer.len();
    inplace_quick_sort(&mut buffer, 0, length);

    std::hint::black_box(&buffer);
}

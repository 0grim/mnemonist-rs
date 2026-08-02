//! The `suffix-array` benchmark — the DC3 (Kärkkäinen–Sanders) construction,
//! `new SuffixArray(string)` upstream. There is no incremental API: a suffix
//! array is built once from a whole sequence and then only read (`array()`),
//! so — like `sort` — this has no per-element `mixed` op-stream and instead
//! reuses the `drain` shape: one measured sample per **construction**, the
//! same convention `sparse-set`'s iteration walk and `sort`'s sorting both
//! use for an operation that is not a stream of cheap calls.
//!
//! # A small alphabet, on purpose
//!
//! Real suffix-array workloads skew heavily toward small alphabets — genomic
//! text (4 symbols) is the textbook case upstream's own docs cite. A large,
//! nearly-collision-free alphabet would make every suffix comparison resolve
//! in one step, which is the easy case and not the one the algorithm exists
//! for. Four symbols is used here for the same reason, generated fresh per
//! pass so the recursive case (repeated triples) is exercised rather than
//! avoided — see the module's own docs on B-91, which only fires when the
//! recursion actually runs.
//!
//! # Fresh text every pass, materialised before any timing
//!
//! The whole `size * passes` character buffer is drawn from the matched
//! xorshift stream **once, before the timed region**, exactly as `sort`'s
//! input is. Each pass then copies its own `size`-character slice into an
//! owned sequence and builds a `SuffixArray` from it, timing the copy and the
//! construction together — both sides pay an equivalent copy (a JS string is
//! immutable, so producing a substring to build from is not optional there
//! either), so this stays a fair, symmetric measurement of "build one from
//! this text" rather than crediting one side with skipping a step upstream's
//! own API does not let it skip.
//!
//! # The checksum is position-weighted
//!
//! `array()`'s positions are a permutation of `0..size`, so a plain sum is
//! always the same regardless of whether the sort order is right — sorting
//! (or reproducing B-90/B-91's wrong order) cannot change which positions
//! exist, only where they land. Weighting each position by its rank makes the
//! checksum sensitive to the actual order, which is what proves the port
//! reproduces upstream's construction bug-for-bug rather than merely
//! producing *a* permutation of the same numbers.

use mnemonist_core::structures::suffix_array::{Sequence, SuffixArray};

use crate::xorshift::XorShift32;

/// Four symbols, `A`..`D` by code point -- large enough that the string is
/// not the single-character degenerate case (already pinned by upstream's own
/// `'aaaaaaa'` B-91 test), small enough that repeated triples -- and
/// therefore the DC3 recursion -- are common rather than incidental.
const ALPHABET: u16 = 4;
const ALPHABET_BASE: u16 = 65;

/// One measured sample per suffix-array build. `size` characters per pass,
/// `passes` fresh strings indexed; returns per-pass batch nanoseconds, the
/// position-weighted checksum, and `size` itself, so the driver's
/// `ns / batch_k` means nanoseconds per character indexed.
pub fn run_drain(size: u32, seed: u32, passes: usize) -> (Vec<u64>, u64, usize) {
    let size = size as usize;
    let mut rng = XorShift32::new(seed);

    let text: Vec<u16> = (0..size * passes)
        .map(|_| ALPHABET_BASE + rng.below(u32::from(ALPHABET)) as u16)
        .collect();

    let mut batches = Vec::with_capacity(passes);
    let mut checksum: u64 = 0;

    for pass in 0..passes {
        let clock = std::time::Instant::now();

        let slice = text[pass * size..(pass + 1) * size].to_vec();
        let array = SuffixArray::new(Sequence::Text(slice));

        batches.push(clock.elapsed().as_nanos() as u64);

        // Outside the timed region: a verification read, not part of what
        // construction itself costs.
        for (index, position) in array.array().iter().enumerate() {
            checksum += (index as u64 + 1) * (*position as u64);
        }
    }

    std::hint::black_box(&text);

    (batches, checksum, size)
}

/// `--structure`: build one suffix array over a `size`-character random text
/// and touch it -- "size" means "built from `size` characters", the same
/// convention `vector`/`trie` use for a structure with no separate capacity.
pub fn build_structure(size: u32) {
    let mut rng = XorShift32::new(42);
    let text: Vec<u16> = (0..size)
        .map(|_| ALPHABET_BASE + rng.below(u32::from(ALPHABET)) as u16)
        .collect();

    let array = SuffixArray::new(Sequence::Text(text));

    std::hint::black_box(&array);
}

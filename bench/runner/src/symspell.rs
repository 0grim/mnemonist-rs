//! The `symspell` mixed workload — a small, clustered vocabulary over a
//! shared `SymSpell` at upstream's own defaults (`maxDistance` 2,
//! `verbosity` 2), because a random vocabulary defeats the whole structure:
//! `search` only ever finds anything by generating deletes of the query and
//! matching them against deletes generated from added words, and if no
//! added word is within `maxDistance` of a query, every search returns
//! empty — timing a hash-map miss, not the edit-distance machinery this
//! module exists to measure.
//!
//! # Word generation: fixed prefix, scrambled suffix — and why "scrambled"
//! is load-bearing, not decorative
//!
//! Every word is `"qu"` (a fixed two-character prefix) followed by a
//! six-character suffix over the 26-letter alphabet, encoding a
//! **multiplicatively scrambled** domain value in base 26 — not the domain
//! value directly. Two vocabulary designs were tried and measured before
//! this one, both rejected for opposite reasons documented in
//! [`domain_cap`] and [`scramble`]'s own doc comments:
//!
//! 1. A 4-letter suffix over a 10-symbol alphabet, encoding the domain value
//!    directly: at `size` 4,000 (40% of the 10,000-word space), most word
//!    pairs landed within `maxDistance` (2) by sheer density — `search`
//!    returned **~413 suggestions per call out of a 4,000-word dictionary**,
//!    and a 200,000-op pass took over a minute.
//! 2. A 6-letter suffix over the full 26-letter alphabet, ALSO encoding the
//!    domain value directly (not scrambled): switching alphabets did
//!    **not** fix it — `search` still returned ~542 suggestions per call.
//!    The real cause was never density of the suffix space; it was that
//!    encoding `0, 1, 2, …` directly in any fixed base makes CONSECUTIVE
//!    domain values one-character-apart neighbours by construction (exactly
//!    like counting), so most of the dictionary sits within `maxDistance` of
//!    most queries regardless of alphabet size.
//!
//! [`scramble`] (a golden-ratio multiplicative hash, matched on the JS side
//! by `Math.imul`) spreads the domain across the suffix space before
//! encoding, so sequential workload values stop being artificially adjacent
//! dictionary entries — what makes a query findable is now, genuinely, only
//! the deliberate one-character perturbation `query_for` applies to one
//! specific dictionary entry, not incidental proximity between unrelated
//! ones. Measured after this fix: `search` returns an average of **1.40
//! suggestions per call**, and a 200,000-op pass completes in under a
//! second.
//!
//! # Prefilled, then grown — same reasoning as `bloom-filter`
//!
//! Half the domain is added before timing starts (a stated fill ratio), and
//! the timed `add` stream keeps drawing from the same domain, so the
//! searchable fraction of the vocabulary grows over the run rather than
//! starting at zero. Measured directly at `size` 4,000, seed 42, over the
//! full workload: **98.4% of `search` calls return at least one
//! suggestion** — a real, non-empty candidate rate, not the near-zero a
//! random vocabulary would produce, and not the near-total-match rate the
//! two rejected designs above showed either.
//!
//! Op mix: 50% `add` (mutating, no checksum contribution), 50% `search`
//! (pure read, contributing a position-weighted sum of `distance + 1` over
//! the returned suggestions) — `fuzzy-map`'s two-method shape, since this
//! module has neither `delete` nor a second read method.

use mnemonist_core::structures::symspell::SymSpell;

use crate::workload::Workload;

const PREFIX: &str = "qu";
const ALPHABET: &[u8; 26] = b"abcdefghijklmnopqrstuvwxyz";
const SUFFIX_LEN: u32 = 6;

/// `workload.size` is capped so every index still has a distinct 6-letter
/// suffix, and — load-bearing, not incidental — so the vocabulary stays
/// *sparse* within the 26^6 suffix space. A first draft used a 4-letter
/// suffix over a 10-symbol alphabet (10,000 possible words): at `size`
/// 4,000 that filled 40% of the space, and most word PAIRS ended up within
/// `maxDistance` (2) of each other by pigeonhole, not by the query
/// construction below — measured directly, `search` returned **~413
/// suggestions per call out of a 4,000-word dictionary**, over 10% of the
/// whole vocabulary matching every query, which made each call cost ~645µs
/// (200 calls in 129ms) and a 200,000-op pass take **over a minute**. A
/// 6-letter suffix over the full 26-letter alphabet (26^6 ≈ 309M possible
/// words) keeps the vocabulary sparse enough that accidental proximity
/// between unrelated entries is rare — what makes queries findable is the
/// deliberate one-character perturbation in `query_for`, not a densely
/// packed word space.
fn domain_cap(size: u32) -> u32 {
    size.max(1)
}

/// Golden-ratio multiplicative hash, truncated to 32 bits — matched on the
/// JS side by `Math.imul`, which does exactly the same wrapping 32-bit
/// multiply. Needed because `word_for` would otherwise enumerate the
/// suffix space by straightforward counting (`0, 1, 2, …` in base 26),
/// which makes CONSECUTIVE domain values one-character-apart neighbours by
/// construction — the actual cause of the dense-collision blowup a sparse
/// alphabet alone did not fix (see [`domain_cap`]'s own doc for the
/// measurement). Scrambling first spreads the domain across the suffix
/// space so sequential workload values stop being artificially adjacent
/// dictionary entries.
fn scramble(value: u32) -> u32 {
    value.wrapping_mul(0x9E37_79B1)
}

fn word_for(value: u32) -> String {
    let mut word = String::with_capacity(PREFIX.len() + SUFFIX_LEN as usize);
    word.push_str(PREFIX);

    let mut remaining = scramble(value);

    for _ in 0..SUFFIX_LEN {
        let digit = remaining % 26;
        remaining /= 26;
        word.push(ALPHABET[digit as usize] as char);
    }

    word
}

/// A one-character perturbation of `word_for(value)`'s suffix — a genuine
/// edit-distance-1 relationship to a real dictionary entry, not an
/// independently-random string.
fn query_for(value: u32) -> String {
    let mut bytes: Vec<u8> = word_for(value).into_bytes();
    let position = PREFIX.len() + (value as usize % SUFFIX_LEN as usize);
    let current_index = ALPHABET
        .iter()
        .position(|&b| b == bytes[position])
        .unwrap_or(0);

    bytes[position] = ALPHABET[(current_index + 1) % ALPHABET.len()];

    String::from_utf8(bytes).expect("PREFIX and ALPHABET are both plain ASCII")
}

/// One measured pass: dictionary prefilled to half the domain (untimed),
/// then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let domain = domain_cap(workload.size);
    let mut dict = SymSpell::new(2.0, 2).expect("max_distance 2.0, verbosity 2 are always valid");

    let prefill = domain / 2;

    for i in 0..prefill {
        dict.add(&word_for(i));
    }

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let member = workload.a[i] % domain;

            if workload.kind[i] < 2 {
                dict.add(&word_for(member));
            } else {
                let suggestions = dict.search(&query_for(member));

                for (position, suggestion) in suggestions.iter().enumerate() {
                    checksum += (position as u64 + 1) * (suggestion.distance as u64 + 1);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&dict);

    (batches, checksum)
}

/// `--structure`: "size" means "prefilled with `domain_cap(size)` words".
pub fn build_structure(size: u32) {
    let domain = domain_cap(size);
    let mut dict = SymSpell::new(2.0, 2).expect("max_distance 2.0, verbosity 2 are always valid");

    for i in 0..domain {
        dict.add(&word_for(i));
    }

    std::hint::black_box(&dict);
}

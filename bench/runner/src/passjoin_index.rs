//! The `passjoin-index` mixed workload — a scrambled vocabulary over a
//! shared `PassjoinIndex`, at `k = 2`.
//!
//! # Same vocabulary trap `symspell.rs` already found and fixed
//!
//! `PassjoinIndex::search` only ever finds anything by partitioning the
//! query into segments and matching them against segments generated from
//! added strings — same shape as `symspell`'s delete-based matching, same
//! failure mode for a random vocabulary (nothing within `k`, every search
//! empty) and same failure mode for a *naively* clustered one. `symspell.rs`
//! already measured both traps directly: encoding the domain value straight
//! into a fixed alphabet — density aside — makes consecutive domain values
//! one-character-apart neighbours by construction, so most of the
//! vocabulary lands within `k` of most queries. [`word_for`] reuses that
//! file's fix rather than rediscovering it: a golden-ratio multiplicative
//! scramble (`Math.imul`-matched on the JS side) before encoding, so only
//! the deliberate one-character perturbation [`query_for`] applies makes a
//! query findable.
//!
//! # `levenshtein` is a plain textbook DP, not a matched primitive
//!
//! Unlike the xorshift32 PRNG, Levenshtein distance is a well-defined
//! mathematical function with exactly one correct answer for any pair of
//! strings — both sides only need to compute it *correctly*, not
//! byte-for-byte identically, for the checksum to agree. `levenshtein`
//! here is the standard single-row dynamic-programming edit distance;
//! `bench/node/run.js`'s own twin is the same algorithm, not a shared
//! implementation.
//!
//! # Prefilled, then grown — same reasoning as `bloom-filter`/`symspell`
//!
//! Half the domain is added before timing starts, and the timed `add`
//! stream keeps drawing from the same domain, so the searchable fraction
//! grows over the run.
//!
//! Op mix: 50% `add` (mutating, no checksum contribution), 50% `search`
//! (pure read, contributing a position-weighted sum over the returned
//! strings' lengths) — the same two-method shape `symspell.rs` uses, for
//! the same reason (neither module has a `delete` or a second read method).
//!
//! # `size`/`ops` are 2,000/5,000 — much smaller than `symspell`'s, and for
//! a different reason than any tree module's domain reduction
//!
//! `add` never deduplicates — upstream's own `#.add` unconditionally pushes
//! a new `stringIndex` onto every segment's candidate list even for a word
//! already present, a genuine property this port reproduces exactly. This
//! batch's usual `kind % 4` stream draws `add` targets *with replacement*
//! from a domain far smaller than the op count, so the SAME word gets
//! re-added many times over a long run, and each re-add makes every
//! matching segment's candidate list longer — which is exactly what
//! `search` has to walk (and Levenshtein-verify) for every future query
//! touching that segment. Measured directly: at `size` 2,000, `ops` 20,000,
//! a single pass took **7.5 seconds** and the index (should have stayed
//! near 2,000 entries) grew past 3,500 from duplicate re-adds alone. At
//! `size` 2,000, `ops` 5,000 the same shape stays honest but fast: the
//! index grows from a 1,000-entry prefill to 3,551 entries by the run's
//! end, `search` still returns a match **74.9%** of the time (avg 0.88
//! matches/call), and the whole pass completes in well under a second.

use mnemonist_core::structures::passjoin_index::PassjoinIndex;

use crate::workload::Workload;

const PREFIX: &str = "qu";
const ALPHABET: &[u8; 26] = b"abcdefghijklmnopqrstuvwxyz";
const SUFFIX_LEN: u32 = 6;
const K: i64 = 2;

fn domain_cap(size: u32) -> u32 {
    size.max(1)
}

/// Golden-ratio multiplicative hash, matched on the JS side by `Math.imul`
/// — see the module docs for why this is load-bearing, not decorative
/// (`symspell.rs`'s own measurement of the sequential-adjacency trap).
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
/// edit-distance-1 relationship to one specific added string.
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

/// Standard single-row DP Levenshtein distance — see the module docs for
/// why this need not match `bench/node/run.js`'s own implementation
/// byte-for-byte, only agree with it on the answer.
fn levenshtein(a: &str, b: &str) -> i64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<i64> = (0..=b.len() as i64).collect();

    for (i, &ca) in a.iter().enumerate() {
        let mut prev_diagonal = row[0];
        row[0] = i as i64 + 1;

        for (j, &cb) in b.iter().enumerate() {
            let temp = row[j + 1];
            row[j + 1] = if ca == cb {
                prev_diagonal
            } else {
                1 + prev_diagonal.min(row[j]).min(row[j + 1])
            };
            prev_diagonal = temp;
        }
    }

    row[b.len()]
}

/// One measured pass: index prefilled to half the domain (untimed), then
/// the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let domain = domain_cap(workload.size);
    let mut index = PassjoinIndex::new(K).expect("K is always >= 1");

    let prefill = domain / 2;

    for i in 0..prefill {
        index.add(&word_for(i));
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
                index.add(&word_for(member));
            } else {
                let matches = index.search(&query_for(member), levenshtein);

                for (position, term) in matches.iter().enumerate() {
                    checksum += (position as u64 + 1) * (term.len() as u64 + 1);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&index);

    (batches, checksum)
}

/// `--structure`: "size" means "prefilled with `domain_cap(size)` words".
pub fn build_structure(size: u32) {
    let domain = domain_cap(size);
    let mut index = PassjoinIndex::new(K).expect("K is always >= 1");

    for i in 0..domain {
        index.add(&word_for(i));
    }

    std::hint::black_box(&index);
}

//! The `inverted-index` mixed workload — a multi-container whose "values per
//! key" is a posting list: how many documents sit under one token.
//!
//! Tokenizing is the bridge's job upstream (`docs/modules/inverted-index.md`)
//! and core's `add`/`get` both take already-tokenized input, so this bench
//! does the (trivial, deterministic) tokenizing itself, on both sides, before
//! calling in — the same division `fuzzy_map.rs` draws for hashing.
//! `Doc = Tok = u32`: a document is identified by its own insertion counter,
//! and a token is a bare integer rather than a formatted string (`trie.rs`
//! needs hex text because a real trie only makes sense over strings; an
//! inverted index's token equality does not care what the token *looks*
//! like, only that two occurrences of "the same token" compare equal, so a
//! `u32` is the simplest faithful instance and avoids paying string
//! formatting on every op on either side).
//!
//! Op mix: 50% `add` (mutating, two tokens per document, no checksum
//! contribution), 25% `get` with a single-token query (a pure read,
//! contributing the number of matching documents), 25% `get` with a
//! two-token query (a pure read that additionally exercises the AND
//! intersection over two posting lists, contributing the same).
//!
//! # `size` is the token vocabulary, deliberately far smaller than the doc count
//!
//! `workload.size` bounds `workload.a`/`workload.b` (`Workload::generate`),
//! so it is read here as the **token vocabulary** (1,000 — the `mixed-2e5`
//! workload below) rather than the doc count: every `add`'s two tokens and
//! every query's tokens are drawn straight from `0..size`, with no modulo
//! needed on top. `ops` is 200,000 here rather than this batch's usual 1e6 —
//! smaller on purpose, and checked before committing to it (the `bit-set.rs`
//! `rank` lesson `methodology.md` documents): at 50% `add`, that is 100,000
//! documents over a 1,000-word vocabulary, ~200,000 token insertions, **~200
//! documents per posting list on average**. A two-token `get` (25% of
//! 200,000 = 50,000 calls) intersects two ~200-length posting lists each —
//! on the order of 50,000 × 200 ≈ 10M comparisons total, negligible next to
//! the workload's own size. Reusing this batch's usual 1e6/1e6 shape here
//! (500,000 docs, ~1,000 postings/token, 250,000 two-token queries) would
//! have pushed that up roughly 25×, into the same "op whose cost scales with
//! a workload parameter" territory `bit-set`'s `rank` fell into — sanity-checked
//! and pulled back before committing to a size, not discovered by a run that
//! would not finish.

use mnemonist_core::structures::inverted_index::InvertedIndex;

use crate::workload::Workload;

/// Docs per vocabulary word for `--structure`'s prefill, matching the mixed
/// workload's own ~100,000-docs-over-1,000-words ratio (100:1). See
/// `lru_cache.rs::capacity_for` for the same "derive a comparable footprint
/// from `size`" idea applied to a different pair of quantities.
const DOCS_PER_WORD: u32 = 100;

/// One measured pass: fresh index, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut index: InvertedIndex<u32, u32> = InvertedIndex::new();
    let mut next_doc: u32 = 0;

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let t1 = workload.a[i];
            let t2 = workload.b[i];

            match workload.kind[i] {
                0 | 1 => {
                    index.add(next_doc, vec![t1, t2]);
                    next_doc += 1;
                }
                2 => checksum += index.get(&[t1]).len() as u64,
                _ => checksum += index.get(&[t1, t2]).len() as u64,
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&index);

    (batches, checksum)
}

/// `--structure`: prefill `size * DOCS_PER_WORD` documents, two deterministic
/// tokens each (`i % size`, `(i / size) % size`, needing no PRNG), and touch
/// the index. `size` here is the vocabulary, matching the mixed workload's
/// own meaning of it.
pub fn build_structure(size: u32) {
    let mut index: InvertedIndex<u32, u32> = InvertedIndex::new();
    let doc_count = size.saturating_mul(DOCS_PER_WORD).max(1);

    for i in 0..doc_count {
        index.add(i, vec![i % size, (i / size) % size]);
    }

    std::hint::black_box(&index);
    std::hint::black_box(index.get(&[0]).len());
}

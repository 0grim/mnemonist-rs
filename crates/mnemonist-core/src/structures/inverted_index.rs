//! Port of upstream `inverted-index.js` (v0.40.4).
//!
//! A document store plus a token → posting-list index: `add(doc)` tokenizes
//! `doc`, and for every distinct token records `doc`'s position; `get(query)`
//! tokenizes the query and intersects the posting lists of its tokens,
//! returning every document that contains **all** of them (a boolean AND).
//!
//! # Tokenizing is the bridge's job, not core's
//!
//! Upstream stores two JS functions, `documentTokenizer`/`queryTokenizer`,
//! called on every `add`/`get`. Core never holds a callback of unknown type —
//! same reasoning as `default_map`'s factory and `default_weak_map`'s: a
//! stored `F` would put a JS callback inside a crate that must not know
//! JavaScript exists, and the constructor's `typeof … !== 'function'` check
//! (plus the `Array.isArray(descriptor) ? […] : …` shape-sniffing before it)
//! is a JavaScript question start to finish. So [`InvertedIndex::add`] and
//! [`InvertedIndex::get`] both take an **already-tokenized** `Vec<Tok>` —
//! the bridge runs the JS tokenizer, checks `Array.isArray` on its result
//! (upstream's own guard, verbatim, at
//! `crates/mnemonist-napi/src/inverted_index.rs`), and hands the result in.
//!
//! # `mapping` is `crate::map::OrderedMap`, reused rather than reinvented
//!
//! `this.mapping = new Map()` is exactly `default_map`'s `Map` problem again
//! — insertion order matters (`tokens()` must yield tokens in the order they
//! were first seen; the upstream test pins all seven, in order) — so this
//! module reuses [`crate::map::OrderedMap`] and [`crate::map::MapCursor`]
//! rather than a second copy of the same machinery. Nothing here ever
//! deletes a token, so the tombstone/compaction half of `OrderedMap` is
//! simply never exercised — harmless, and cheaper than building a
//! delete-free map that upstream itself does not have either (`mapping` is a
//! bare `new Map()`, the general-purpose one).
//!
//! # `documents()` is the OTHER cursor shape this project has, `values()` over a frozen length
//!
//! ```js
//! InvertedIndex.prototype.documents = function() {
//!   var documents = this.items, l = documents.length, i = 0;
//!   return new Iterator(function() {
//!     if (i >= l) return {done: true};
//!     var value = documents[i++];
//!     return {value, done: false};
//!   });
//! };
//! ```
//!
//! This is [`crate::cursor::Sequence`]'s exact shape — a length frozen at
//! creation, elements read live — except for one simplification that is
//! sound *because* of what this module's public API actually allows:
//! `this.items` only ever grows (there is no `delete`/`remove` anywhere in
//! `inverted-index.js`), so an index below the frozen length can **never**
//! fail to resolve. [`crate::cursor::Step::Gap`] cannot happen here, so
//! [`DocumentsCursor`] is a small dedicated cursor rather than a
//! `Sequence` impl that would have to invent a `Gap` branch nothing can ever
//! reach — the same judgement call `default_map.rs`'s module docs make for
//! `MapCursor` not being `Sequence`, applied in the other direction: this one
//! *would* fit `Sequence`, but doing so would force `Doc: Clone` (`Sequence::
//! Item` is returned by value) for a structure that never needs to hand back
//! anything but a borrow.
//!
//! # B-240 — `forEach` never calls its callback, regardless of how many documents are stored
//!
//! See [`InvertedIndex::for_each`]'s own docs for the mechanism —
//! `this.documents.length` reads a **method's** arity, not an array's
//! length — and NOTES.md B-240 for the confirmed repro against Node
//! 24.18.1. Reproduced here as a cursor frozen at length zero, so a walk
//! against it is a no-op by construction rather than a hand-written empty
//! callback list standing in for the same effect through a different route.

use std::collections::HashSet;
use std::hash::Hash;

use crate::map::{MapCursor, OrderedMap};
use crate::utils::merge::intersection_unique_two;

/// Upstream's `InvertedIndex`.
///
/// `Doc` is the stored document type, opaque to this module. `Tok` is the
/// token type tokenizing a document or a query produces; `Hash + Eq + Clone`
/// because tokens are `OrderedMap` keys and because a document's own tokens
/// are deduplicated against each other on `add` (upstream's `new Set()`).
pub struct InvertedIndex<Doc, Tok> {
    items: Vec<Doc>,
    mapping: OrderedMap<Tok, Vec<usize>>,
    size: usize,
}

impl<Doc, Tok> Default for InvertedIndex<Doc, Tok> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Doc, Tok> InvertedIndex<Doc, Tok> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            mapping: OrderedMap::new(),
            size: 0,
        }
    }

    /// Upstream's `clear`.
    pub fn clear(&mut self) {
        self.items.clear();
        self.mapping.clear();
        self.size = 0;
    }

    /// Upstream's `size` property: the document count. A real, non-drifting
    /// counter — every `add` increments it exactly once, and nothing removes
    /// a document, so there is no B-40-shaped divergence to reproduce here.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Upstream's `dimension` property: the number of distinct tokens seen.
    ///
    /// Upstream assigns `this.dimension = this.mapping.size;` at the end of
    /// every `add` — a re-derivation, not an independent increment — so a
    /// read straight off `mapping.len()` is exactly equivalent and cannot
    /// drift the way `DefaultMap.size` does (B-40).
    pub fn dimension(&self) -> usize {
        self.mapping.len()
    }

    /// The stored documents, in insertion order — upstream's `this.items`.
    pub fn items(&self) -> &[Doc] {
        &self.items
    }

    /// The token → posting-list index — upstream's `this.mapping`.
    pub fn mapping(&self) -> &OrderedMap<Tok, Vec<usize>> {
        &self.mapping
    }

    /// A cursor over the stored documents — upstream's `documents()`. See
    /// the module docs for why this is [`DocumentsCursor`] rather than
    /// [`crate::cursor::Sequence`].
    pub fn documents(&self) -> DocumentsCursor {
        DocumentsCursor::open(self.items.len())
    }

    /// Upstream's `forEach`.
    ///
    /// # B-240 — always a no-op
    ///
    /// ```js
    /// InvertedIndex.prototype.forEach = function(callback, scope) {
    ///   scope = arguments.length > 1 ? scope : this;
    ///   for (var i = 0, l = this.documents.length; i < l; i++)
    ///     callback.call(scope, this.documents[i], i, this);
    /// };
    /// ```
    ///
    /// `this.documents` is the **method**
    /// `InvertedIndex.prototype.documents`, defined a few lines above — not
    /// `this.items`, the property that actually holds the document array. A
    /// JS function's `.length` is its declared parameter count, and
    /// `documents` takes none, so `this.documents.length` is `0`: not
    /// "usually 0", not "0 until some threshold" — the literal, permanent
    /// arity of a zero-argument function. The loop condition `i < l` is
    /// therefore `0 < 0`, false on the very first check, for every call,
    /// regardless of `this.items.length`. `callback` is **never** invoked,
    /// on an index with one document or with a thousand.
    ///
    /// Verified against Node 24.18.1
    /// (`~/upstream-mnemonist/inverted-index.js`):
    /// `InvertedIndex.from(['a b', 'b c'], s => s.split(' ')).forEach(cb)`
    /// calls `cb` zero times. Recorded as **B-240** in NOTES.md.
    ///
    /// The original suite's own `forEach` block (`test/inverted-index.js`,
    /// `'should be possible to iterate using #.forEach'`) asserts properties
    /// of each invocation but never counts how many happened — so it passes
    /// identically whether the callback runs 0 times or *n* times, and gate
    /// 4 cannot catch this on its own. This port's own tests and the
    /// differential fuzzer's `$forEach` op are what pin the count at exactly
    /// zero — see `crates/difffuzz/src/modules/inverted_index.rs`.
    ///
    /// Reproduced rather than "fixed": a walk that visited every document
    /// would be the *correct*, useful behaviour, and is exactly what a
    /// careful porter would write without reading this file line by line —
    /// which is precisely why it would be a defect (CLAUDE.md's bug-for-bug
    /// mandate). [`InvertedIndex::for_each`] hands back a [`DocumentsCursor`]
    /// frozen at length **zero** unconditionally, so stepping it always
    /// yields `None` immediately: the loop bound really is zero here, not a
    /// value merely rendered as if it were.
    pub fn for_each(&self) -> DocumentsCursor {
        DocumentsCursor::open(0)
    }
}

impl<Doc, Tok: Hash + Eq + Clone> InvertedIndex<Doc, Tok> {
    /// A cursor over the distinct tokens seen, in the order they were first
    /// added — upstream's `tokens()`, `this.mapping.keys()`. Step against
    /// [`InvertedIndex::mapping`] with [`MapCursor::step`] and project out
    /// the key half.
    pub fn tokens(&self) -> MapCursor {
        MapCursor::open()
    }

    /// Upstream's `add`.
    ///
    /// `tokens` is what the bridge's `documentTokenizer(doc)` already
    /// returned — the `Array.isArray` check on that value is a JS question
    /// and happens before this is called; see the module docs.
    ///
    /// Per-document dedup (`done` upstream, a `Set`) is a fresh `HashSet`
    /// scoped to this one call: it decides whether *this document* has
    /// already recorded this token, which is a different question from
    /// whether the token has ever been seen by the index before (that is
    /// `mapping`'s own job, one line down).
    pub fn add(&mut self, doc: Doc, tokens: Vec<Tok>) {
        self.size += 1;

        let key = self.items.len();
        self.items.push(doc);

        let mut done: HashSet<Tok> = HashSet::new();

        for token in tokens {
            if !done.insert(token.clone()) {
                continue;
            }

            match self.mapping.get_mut(&token) {
                Some(postings) => postings.push(key),
                None => {
                    self.mapping.set(token, vec![key]);
                }
            }
        }
    }
}

impl<Doc, Tok: Hash + Eq + Clone> InvertedIndex<Doc, Tok> {
    /// Upstream's `get`: an AND query over the tokenized `query`.
    ///
    /// `query_tokens` is what the bridge's `queryTokenizer(query)` already
    /// returned, same division of labour as [`InvertedIndex::add`].
    ///
    /// Each token's posting list is naturally sorted ascending and
    /// duplicate-free — a document is recorded under a token at most once
    /// (the per-`add` dedup above), and documents are recorded in the order
    /// they were added — which is exactly [`intersection_unique_two`]'s
    /// precondition, so upstream's own repeated
    /// `helpers.intersectionUniqueArrays(results, c)` fold is [`
    /// intersection_unique_two`] called in the identical left-to-right
    /// order, not the k-way form `mnemonist-core::utils::merge` also
    /// provides (upstream never uses that one here either).
    pub fn get(&self, query_tokens: &[Tok]) -> Vec<&Doc> {
        if self.size == 0 || query_tokens.is_empty() {
            return Vec::new();
        }

        let first = match self.mapping.get(&query_tokens[0]) {
            Some(postings) if !postings.is_empty() => postings.clone(),
            _ => return Vec::new(),
        };

        let mut results = first;

        for token in &query_tokens[1..] {
            match self.mapping.get(token) {
                Some(postings) if !postings.is_empty() => {
                    results = intersection_unique_two(&results, postings);
                }
                _ => return Vec::new(),
            }
        }

        results.iter().map(|&index| &self.items[index]).collect()
    }
}

impl<Doc, Tok> FromIterator<(Doc, Vec<Tok>)> for InvertedIndex<Doc, Tok>
where
    Tok: Hash + Eq + Clone,
{
    /// Upstream's static `.from(iterable, descriptor)`, minus the
    /// tokenizing (already applied per item here) and the JS-iterable
    /// enumeration question — both the bridge's job, exactly as for
    /// [`InvertedIndex::add`].
    fn from_iter<I: IntoIterator<Item = (Doc, Vec<Tok>)>>(iter: I) -> Self {
        let mut index = Self::new();

        for (doc, tokens) in iter {
            index.add(doc, tokens);
        }

        index
    }
}

/// A stateful, non-restartable walk over `0..len`, freezing `len` once at
/// [`DocumentsCursor::open`] and never re-reading it.
///
/// See the module docs for why this is a dedicated cursor rather than
/// [`crate::cursor::Sequence`]: this walk can never open a
/// [`crate::cursor::Step::Gap`] (nothing before `len` can ever stop
/// resolving, because nothing here ever removes a document), so there is no
/// third state to model, and no need to return owned elements just to fit
/// the general trait's shape.
#[derive(Debug, Clone, Copy)]
pub struct DocumentsCursor {
    ordinal: usize,
    len: usize,
}

impl DocumentsCursor {
    fn open(len: usize) -> Self {
        Self { ordinal: 0, len }
    }

    /// Advance one step, reading live against `items` — upstream's
    /// `documents[i++]`.
    pub fn step<'a, Doc>(&mut self, items: &'a [Doc]) -> Option<&'a Doc> {
        if self.ordinal >= self.len {
            return None;
        }

        let item = &items[self.ordinal];
        self.ordinal += 1;

        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    fn index_from(docs: &[&str]) -> InvertedIndex<String, String> {
        let mut index = InvertedIndex::new();

        for doc in docs {
            index.add((*doc).to_owned(), tokenize(doc));
        }

        index
    }

    fn drain_documents(index: &InvertedIndex<String, String>) -> Vec<&String> {
        let mut cursor = index.documents();
        let mut out = Vec::new();

        while let Some(doc) = cursor.step(index.items()) {
            out.push(doc);
        }

        out
    }

    fn drain_tokens(index: &InvertedIndex<String, String>) -> Vec<String> {
        let mut cursor = index.tokens();
        let mut out = Vec::new();

        while let Some((token, _)) = cursor.step(index.mapping()) {
            out.push(token.clone());
        }

        out
    }

    // ---- 1:1 port of the upstream suite (shape), as a baseline -----------

    #[test]
    fn adding_documents_updates_size_and_dimension() {
        let docs = [
            "the cat eats the mouse",
            "the mouse likes cheese",
            "cheese is something mouse really like to eat",
        ];
        let index = index_from(&docs);

        assert_eq!(index.size(), 3);
        // Distinct tokens across all three (whitespace tokenizer, no
        // stemming/stopwords here -- the fuzz-friendly shape; see the fuzz
        // spec's own docs for why the differential grammar uses a small
        // fixed pool instead of a real tokenizer).
        let expected_dimension = {
            let mut all: HashSet<&str> = HashSet::new();
            for doc in &docs {
                for token in doc.split_whitespace() {
                    all.insert(token);
                }
            }
            all.len()
        };
        assert_eq!(index.dimension(), expected_dimension);
    }

    #[test]
    fn querying_returns_the_and_intersection_of_matching_documents() {
        let docs = ["a b c", "b c d", "c d e"];
        let index = index_from(&docs);

        assert_eq!(
            index.get(&tokenize("c")),
            vec![
                &docs[0].to_owned(),
                &docs[1].to_owned(),
                &docs[2].to_owned()
            ]
        );
        assert_eq!(
            index.get(&tokenize("b c")),
            vec![&docs[0].to_owned(), &docs[1].to_owned()]
        );
        assert_eq!(index.get(&tokenize("a b")), vec![&docs[0].to_owned()]);
        assert_eq!(index.get(&tokenize("a d")), Vec::<&String>::new());
        assert_eq!(index.get(&tokenize("e")), vec![&docs[2].to_owned()]);
    }

    #[test]
    fn from_iter_builds_the_same_index_as_repeated_add() {
        let docs = ["a b", "b c"];
        let via_from: InvertedIndex<String, String> = docs
            .iter()
            .map(|doc| ((*doc).to_owned(), tokenize(doc)))
            .collect();

        assert_eq!(via_from.size(), 2);
        assert_eq!(
            via_from.get(&tokenize("b")),
            vec![&docs[0].to_owned(), &docs[1].to_owned()]
        );
    }

    #[test]
    fn documents_iterates_in_insertion_order() {
        let docs = ["a", "b", "c"];
        let index = index_from(&docs);

        assert_eq!(
            drain_documents(&index),
            vec![&"a".to_owned(), &"b".to_owned(), &"c".to_owned()]
        );
    }

    #[test]
    fn tokens_iterates_in_first_seen_order() {
        let docs = ["cat eats mouse", "mouse likes cheese"];
        let index = index_from(&docs);

        assert_eq!(
            drain_tokens(&index),
            vec!["cat", "eats", "mouse", "likes", "cheese"]
        );
    }

    // ---- B-240 -------------------------------------------------------

    #[test]
    fn b_240_for_each_never_visits_a_single_document() {
        let index = index_from(&["a b", "b c", "c d"]);
        assert_eq!(index.size(), 3, "documents really are stored");

        let mut cursor = index.for_each();
        assert_eq!(
            cursor.step(index.items()),
            None,
            "the loop bound is zero, unconditionally"
        );
    }

    #[test]
    fn b_240_holds_on_an_empty_index_too() {
        let index: InvertedIndex<String, String> = InvertedIndex::new();
        let mut cursor = index.for_each();
        assert_eq!(cursor.step(index.items()), None);
    }

    // ---- Everything else --------------------------------------------

    #[test]
    fn a_query_before_any_document_is_added_returns_nothing() {
        let index: InvertedIndex<String, String> = InvertedIndex::new();
        assert_eq!(index.get(&[String::from("a")]), Vec::<&String>::new());
    }

    #[test]
    fn an_empty_query_returns_nothing() {
        let index = index_from(&["a b"]);
        assert_eq!(index.get(&[]), Vec::<&String>::new());
    }

    #[test]
    fn a_repeated_token_within_one_document_is_recorded_once() {
        let mut index: InvertedIndex<String, String> = InvertedIndex::new();
        index.add(
            "aaa".to_owned(),
            vec!["a".to_owned(), "a".to_owned(), "a".to_owned()],
        );

        assert_eq!(index.dimension(), 1);
        assert_eq!(index.mapping().get(&"a".to_owned()), Some(&vec![0]));
    }

    #[test]
    fn clear_resets_everything() {
        let mut index = index_from(&["a b", "b c"]);
        index.clear();

        assert_eq!(index.size(), 0);
        assert_eq!(index.dimension(), 0);
        assert_eq!(index.get(&[String::from("a")]), Vec::<&String>::new());
        assert_eq!(drain_documents(&index), Vec::<&String>::new());
    }

    #[test]
    fn documents_cursor_is_not_restartable_and_does_not_grow_after_it_reports_done() {
        let mut index = index_from(&["a"]);
        let mut cursor = index.documents();

        assert!(cursor.step(index.items()).is_some());
        assert_eq!(cursor.step(index.items()), None);

        index.add("b".to_owned(), tokenize("b"));
        assert_eq!(
            cursor.step(index.items()),
            None,
            "a cursor that reported done stays done even though items grew"
        );
    }

    #[test]
    fn a_document_added_after_a_cursor_opens_is_not_visible_because_the_length_is_frozen() {
        let mut index = index_from(&["a", "b"]);
        let mut cursor = index.documents();

        assert_eq!(cursor.step(index.items()), Some(&"a".to_owned()));
        index.add("c".to_owned(), tokenize("c"));
        // The frozen length was 2 at `documents()`'s call time, so the
        // append is NOT visible -- this is the frozen-length half of the
        // `Sequence` shape, faithfully reproduced without `Sequence` itself.
        assert_eq!(cursor.step(index.items()), Some(&"b".to_owned()));
        assert_eq!(cursor.step(index.items()), None);
    }

    #[test]
    fn get_only_matches_documents_containing_every_query_token() {
        let index = index_from(&["red car", "red bike", "blue car"]);

        assert_eq!(index.get(&tokenize("red car")), vec![&"red car".to_owned()]);
    }
}

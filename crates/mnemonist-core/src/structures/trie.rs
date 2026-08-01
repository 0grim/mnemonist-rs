//! Port of upstream `trie.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! Upstream's own architecture, quoted from its header comment: *"the Trie is
//! based upon the TrieMap since the underlying machine is the very same. The
//! Trie just does not let you set values and only considers the existence of
//! the given prefixes."* Concretely, `trie.js` copies every method off
//! `TrieMap.prototype` onto `Trie.prototype`, then deletes four of them
//! (`set`, `get`, `values`, `entries`) and defines its own `add` and `find`.
//! Everything else — `clear`, `has`, `delete`, `update`, `prefixes`/`keys` — is
//! **the exact same function**, running against a sentinel value of `true`.
//!
//! [`Trie`] mirrors this by composition rather than by copy-and-delete, which
//! Rust has no equivalent of: it wraps a
//! [`TrieMap<T, bool>`](crate::structures::trie_map::TrieMap) and forwards to
//! it. `add`/`update`/`delete`/`has`/`clear` are the forwarded methods; `find`
//! re-shapes `TrieMap::find`'s `(suffix, &bool)` pairs down to bare suffixes,
//! matching upstream's own override, which drops the value half `TrieMap`'s
//! `find` keeps.
//!
//! # `update` is inherited upstream, and reproduced here too
//!
//! `trie.js`'s delete list is `set`, `get`, `values`, `entries` — **not**
//! `update`. Confirmed against real Node 24.18.1: `new Trie().update` is a
//! real, callable function, running `TrieMap.prototype.update` against the
//! boolean sentinel. No upstream test calls it and `trie.d.ts` does not
//! declare it, but it is genuinely reachable, so [`Trie::update`] exists here
//! too rather than being silently dropped.

use crate::structures::trie_map::TrieMap;

/// Upstream's `Trie`: a set of token sequences, with no attached value beyond
/// membership.
#[derive(Debug, Clone)]
pub struct Trie<T> {
    inner: TrieMap<T, bool>,
}

impl<T> Default for Trie<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Trie<T> {
    pub fn new() -> Self {
        Self {
            inner: TrieMap::new(),
        }
    }

    /// Upstream's `size` property.
    pub fn size(&self) -> usize {
        self.inner.size()
    }

    /// Upstream's `clear`.
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

impl<T: Clone + PartialEq> Trie<T> {
    /// Upstream's `add`.
    ///
    /// Upstream sets the sentinel to `true` unconditionally and returns
    /// `this` for chaining; this returns whether the sequence was newly
    /// added, which the bridge drops to match the JS surface (same choice
    /// `SparseSet::add` makes).
    pub fn add(&mut self, prefix: impl IntoIterator<Item = T>) -> bool {
        self.inner.set(prefix, true).is_none()
    }

    /// Upstream's `has`.
    pub fn has(&self, prefix: impl IntoIterator<Item = T>) -> bool {
        self.inner.has(prefix)
    }

    /// Upstream's `delete`.
    pub fn delete(&mut self, prefix: impl IntoIterator<Item = T>) -> bool {
        self.inner.delete(prefix)
    }

    /// Upstream's `update`, inherited unmodified from `TrieMap` — see the
    /// module docs for why it is reproduced here despite no test reaching it.
    pub fn update<F: FnOnce(Option<bool>) -> bool>(
        &mut self,
        prefix: impl IntoIterator<Item = T>,
        f: F,
    ) {
        self.inner.update(prefix, f);
    }

    /// Upstream's own `Trie.prototype.find` override: the suffixes only, with
    /// the `TrieMap`-inherited value half dropped.
    pub fn find(&self, prefix: impl IntoIterator<Item = T>) -> Vec<Vec<T>> {
        self.inner
            .find(prefix)
            .into_iter()
            .map(|(suffix, _value)| suffix)
            .collect()
    }

    /// Upstream's `prefixes`/`keys` (the same function, aliased) — `values`
    /// and `entries` are two of the four upstream deletes, since a `Trie` has
    /// no value to project. See
    /// [`TrieMap::walk`](crate::structures::trie_map::TrieMap::walk) for the
    /// suffix-only contract the caller must complete.
    pub fn walk(
        &self,
        prefix: impl IntoIterator<Item = T>,
    ) -> crate::structures::trie_map::Walk<T> {
        self.inner.walk(prefix)
    }

    /// Advance a [`walk`](Trie::walk), dropping the boolean sentinel a
    /// [`TrieMap`] step would carry — a `Trie` has nothing else to project.
    ///
    /// `Trie` deliberately does not expose its inner `TrieMap`, so this is
    /// the one way a caller (the bridge, or a test) steps a walk it started.
    pub fn step(&self, walk: &mut crate::structures::trie_map::Walk<T>) -> Option<Vec<T>> {
        walk.step(&self.inner).map(|(suffix, _value)| suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(word: &str) -> impl Iterator<Item = char> + Clone + '_ {
        word.chars()
    }

    fn suffix_to_string(suffix: &[char]) -> String {
        suffix.iter().collect()
    }

    fn walk_all(trie: &Trie<char>) -> Vec<String> {
        let mut walk = trie.walk(std::iter::empty());
        let mut out = Vec::new();

        while let Some(suffix) = trie.step(&mut walk) {
            out.push(suffix_to_string(&suffix));
        }

        out
    }

    /// 1:1 port of the upstream `it` block set, as a baseline.
    #[test]
    fn reproduces_the_upstream_suite() {
        let mut trie: Trie<char> = Trie::new();

        trie.add(tokens("rat"));
        trie.add(tokens("rate"));
        trie.add(tokens("tar"));

        assert_eq!(trie.size(), 3);
        assert!(trie.has(tokens("rat")));
        assert!(trie.has(tokens("rate")));
        assert!(trie.has(tokens("tar")));
        assert!(!trie.has(tokens("show")));
        assert!(!trie.has(tokens("ra")));
        assert!(!trie.has(tokens("ratings")));
    }

    #[test]
    fn adding_the_same_item_again_does_not_increase_size() {
        let mut trie: Trie<char> = Trie::new();

        trie.add(tokens("rat"));
        trie.add(tokens("rate"));
        trie.add(tokens("rat"));

        assert_eq!(trie.size(), 2);
        assert!(trie.has(tokens("rat")));
    }

    #[test]
    fn the_null_sequence_is_a_valid_member() {
        let mut trie: Trie<char> = Trie::new();

        trie.add(tokens(""));
        assert_eq!(trie.size(), 1);
        assert!(trie.has(tokens("")));
    }

    #[test]
    fn delete_removes_and_prunes_singleton_chains() {
        let mut trie: Trie<char> = Trie::new();

        trie.add(tokens("rat"));
        trie.add(tokens("rate"));
        trie.add(tokens("tar"));

        assert!(!trie.delete(tokens("")));
        assert!(!trie.delete(tokens("hello")));

        assert!(trie.delete(tokens("rat")));
        assert!(!trie.has(tokens("rat")));
        assert!(trie.has(tokens("rate")));
        assert_eq!(trie.size(), 2);

        assert!(trie.delete(tokens("rate")));
        assert_eq!(trie.size(), 1);

        assert!(trie.delete(tokens("tar")));
        assert_eq!(trie.size(), 0);
    }

    #[test]
    fn has_distinguishes_a_stored_word_from_a_mere_prefix_of_one() {
        let mut trie: Trie<char> = Trie::new();

        trie.add(tokens("romanesque"));

        assert!(trie.has(tokens("romanesque")));
        assert!(!trie.has(tokens("roman")));
        assert!(!trie.has(tokens("")));
    }

    #[test]
    fn find_returns_the_suffix_beyond_the_given_prefix() {
        let mut trie: Trie<char> = Trie::new();

        trie.add(tokens("roman"));
        trie.add(tokens("romanesque"));
        trie.add(tokens("romanesques"));
        trie.add(tokens("greek"));

        let render = |prefix: &str| -> Vec<String> {
            trie.find(tokens(prefix))
                .into_iter()
                .map(|suffix| suffix_to_string(&suffix))
                .collect()
        };

        assert_eq!(
            render("roman"),
            vec![String::new(), "esque".into(), "esques".into()]
        );
        assert_eq!(render("hello"), Vec::<String>::new());
        assert_eq!(
            render(""),
            vec![
                "greek".to_string(),
                "roman".to_string(),
                "romanesque".to_string(),
                "romanesques".to_string(),
            ]
        );
    }

    #[test]
    fn walk_visits_every_word() {
        let mut trie: Trie<char> = Trie::new();

        trie.add(tokens("rat"));
        trie.add(tokens("rate"));

        assert_eq!(walk_all(&trie), vec!["rat".to_string(), "rate".to_string()]);
    }

    /// `update` is inherited from `TrieMap` unmodified upstream; no test in
    /// `test/trie.js` calls it, but it works, so this pins that it still does
    /// here.
    #[test]
    fn update_is_inherited_from_trie_map_and_still_works() {
        let mut trie: Trie<char> = Trie::new();

        trie.update(tokens("cat"), |old| old.unwrap_or(false) || true);
        assert!(trie.has(tokens("cat")));
        assert_eq!(trie.size(), 1);

        // A second update on the same prefix does not increase size.
        trie.update(tokens("cat"), |old| old.unwrap_or(false));
        assert_eq!(trie.size(), 1);
    }

    #[test]
    fn clear_resets_size_and_removes_everything() {
        let mut trie: Trie<char> = Trie::new();
        trie.add(tokens("a"));
        trie.add(tokens("b"));

        trie.clear();

        assert_eq!(trie.size(), 0);
        assert!(!trie.has(tokens("a")));
        assert_eq!(walk_all(&trie), Vec::<String>::new());
    }
}

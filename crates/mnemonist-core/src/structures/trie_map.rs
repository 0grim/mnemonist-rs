//! Port of upstream `trie-map.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! A trie over sequences of tokens, keyed on a plain nested object upstream:
//! each node is `{}`, a per-token property holds the child reached by that
//! token, and one reserved property — `SENTINEL`, `String.fromCharCode(0)` —
//! holds the value stored for the sequence that ends at that node, when one
//! is stored there at all. `crate::structures::trie::Trie` is a thin wrapper
//! over this engine with the value type fixed to `bool`, mirroring upstream's
//! own architecture: `trie.js` copies every method off `TrieMap.prototype`
//! and deletes four of them (see that module's docs).
//!
//! # The node, and why value and children are separate fields here
//!
//! Upstream's `node[token]` and `node[SENTINEL]` are properties of the *same*
//! plain object, which has one genuinely surprising consequence: a token that
//! happens to equal the sentinel string collides with the value slot. Verified
//! against real Node 24.18.1 — `new TrieMap()`, `set('a', 1)`, then
//! `set('a' + TrieMap.SENTINEL + 'b', 2)` — reports `size === 2` and the
//! second value is **unreachable through any public method**: `root` shows
//! only `{a: {'\x00': 1}}`, no trace of the second entry at all. The mechanism
//! is `node = node[token] || (node[token] = {})`: once `node` becomes the
//! *value* `1` (a JS primitive, since the walk just read `node[SENTINEL]`
//! where `SENTINEL` collided with the real token), every further property
//! write on it is a silent no-op in sloppy mode, and the loop's local `node`
//! ends up rebinding to a chain of fresh, unlinked `{}` objects that vanish
//! with the call. `size` still increments, because the final orphan object
//! genuinely has no `SENTINEL` property of its own. See BUG-TRIE-MAP-1 in NOTES.md.
//!
//! Reproducing this exactly would mean modelling JavaScript's primitive/object
//! duality — a plain value that is sometimes indexable and sometimes not —
//! purely to recreate one write silently going nowhere. No upstream test ever
//! embeds the sentinel character in a real token (both `test/trie.js` and
//! `test/trie-map.js` use only ordinary words), and nothing else in this port
//! needs that duality. So [`Node`] keeps the value and the children in two
//! separate fields instead of one shared keyspace, which makes a token equal
//! to the sentinel string an utterly ordinary token here — stored, retrieved
//! and iterated like any other, never colliding with anything. This is a
//! deliberate, disclosed divergence: DIV-TRIE-MAP-1 in DECISIONS-CANDIDATES.md.
//!
//! # Enumeration order, which the test suite depends on
//!
//! `find`, `keys`/`prefixes`, `values` and `entries` all walk a node's own
//! properties via `for (k in node)`, and the result order is asserted
//! directly — `trie.find('roman')` must come back
//! `['roman', 'romanesque', 'romanesques']`, in exactly that order. JS
//! enumerates a plain object's string keys in **insertion order** (with one
//! exception this port does not reproduce; see "Deliberate divergences"
//! below), and `SENTINEL` is a key exactly like any token: whichever of "this
//! node's own value" or "a child at token T" was written first is enumerated
//! first. [`Node`] is therefore an insertion-ordered list of [`Slot`]s, not a
//! value field plus a hash map — the two are stored in the SAME ordered
//! sequence for exactly this reason, even though they no longer share a
//! keyspace. Losing the interleaving would get every DFS order wrong the
//! moment a shorter word is added after a longer one that shares its prefix.
//!
//! # Deliberate divergences
//!
//! * **DIV-TRIE-MAP-1** (above): the sentinel/token collision is not reproduced.
//! * **DIV-TRIE-MAP-2**: `values`/`keys`/`entries`/`prefixes` are implemented as
//!   [`Walk`], which re-navigates from the root by token path on every step
//!   rather than holding upstream's live object references. The two agree on
//!   every sequence of operations the original test suite performs, and agree
//!   when a `delete` unlinks a node no pending walk frame has reached — but
//!   they can disagree when a `delete` removes a node's *reference* from its
//!   parent while leaving that node's own `SENTINEL` property untouched (the
//!   "prune from an ancestor" branch of `delete`, below): upstream's iterator
//!   still holds the orphaned object directly and keeps reporting its stale
//!   value, where this port's path-based walk finds nothing at that path and
//!   moves on. Confirmed against real Node 24.18.1 — no test in either
//!   original suite interleaves a `delete` with an open walk over the deleted
//!   region. See DECISIONS-CANDIDATES.md.
//! * **DIV-TRIE-MAP-3**: `Object.keys` order for a plain object special-cases
//!   integer-like keys (`"0"`, `"1"`, …), enumerating them ascending *before*
//!   any other key regardless of insertion order. [`Node`] does not reproduce
//!   this — every entry enumerates in insertion order, full stop. No token
//!   in either original test file is ever a digit, so gate 4 never reaches
//!   this rule.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::structures::trie_map::TrieMap;
//!
//! let mut trie: TrieMap<char, u32> = TrieMap::new();
//! trie.set("rat".chars(), 1);
//! trie.set("rate".chars(), 2);
//!
//! assert_eq!(trie.get("rat".chars()), Some(&1));
//! assert_eq!(trie.get("ra".chars()), None);
//! assert_eq!(trie.size(), 2);
//! ```

use std::mem;

/// One entry in a node's own, insertion-ordered property list.
///
/// See the module docs for why this is a list of two-variant slots rather
/// than a value field plus a children map: it is what makes the sentinel and
/// every token share one enumeration order, matching upstream's plain object.
#[derive(Debug, Clone)]
enum Slot<T, V> {
    /// `node[SENTINEL] = value` — a sequence ends here.
    Word(V),
    /// `node[token] = child` — traversal continues.
    Child(T, Node<T, V>),
}

/// One trie node: upstream's `{}`.
#[derive(Debug, Clone)]
struct Node<T, V> {
    entries: Vec<Slot<T, V>>,
}

/// A read-only view of one node's own entries, in enumeration order — see
/// [`TrieMap::root`].
pub struct NodeView<'a, T, V> {
    node: &'a Node<T, V>,
}

/// One of a [`NodeView`]'s own entries.
pub enum Entry<'a, T, V> {
    /// `node[SENTINEL]` — a sequence ends here.
    Word(&'a V),
    /// `node[token]` — a child, reached by `token`.
    Child(&'a T, NodeView<'a, T, V>),
}

impl<'a, T, V> NodeView<'a, T, V> {
    /// Own entries, in the same insertion order [`TrieMap::find`]/[`Walk`]
    /// scan them in.
    pub fn entries(&self) -> impl Iterator<Item = Entry<'a, T, V>> + 'a {
        self.node.entries.iter().map(|slot| match slot {
            Slot::Word(value) => Entry::Word(value),
            Slot::Child(token, child) => Entry::Child(token, NodeView { node: child }),
        })
    }
}

impl<T, V> Node<T, V> {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// `Object.keys(node).length === 0`.
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Own entries, in insertion order — `for (k in node)`, minus the
    /// integer-like-key rule (DIV-TRIE-MAP-3).
    fn iter(&self) -> impl Iterator<Item = &Slot<T, V>> {
        self.entries.iter()
    }

    /// Every stored value reachable from this node, mutably, appended to
    /// `out` in the same order [`iter`](Node::iter) would visit them at each
    /// level (depth-first, a node's own value before its children's).
    fn collect_values_mut<'a>(&'a mut self, out: &mut Vec<&'a mut V>) {
        for entry in &mut self.entries {
            match entry {
                Slot::Word(value) => out.push(value),
                Slot::Child(_token, child) => child.collect_values_mut(out),
            }
        }
    }
}

impl<T: PartialEq, V> Node<T, V> {
    fn word_index(&self) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| matches!(entry, Slot::Word(_)))
    }

    /// `node[SENTINEL]`, read-only.
    fn word(&self) -> Option<&V> {
        self.entries.iter().find_map(|entry| match entry {
            Slot::Word(value) => Some(value),
            Slot::Child(..) => None,
        })
    }

    fn child_index(&self, token: &T) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| matches!(entry, Slot::Child(candidate, _) if candidate == token))
    }

    /// `node[token]`, read-only.
    fn child(&self, token: &T) -> Option<&Node<T, V>> {
        let index = self.child_index(token)?;

        match &self.entries[index] {
            Slot::Child(_, child) => Some(child),
            Slot::Word(_) => unreachable!("child_index only ever finds a Child slot"),
        }
    }

    fn child_mut(&mut self, token: &T) -> Option<&mut Node<T, V>> {
        let index = self.child_index(token)?;

        match &mut self.entries[index] {
            Slot::Child(_, child) => Some(child),
            Slot::Word(_) => unreachable!("child_index only ever finds a Child slot"),
        }
    }

    /// `node[token] || (node[token] = {})` — return the existing child,
    /// preserving its position, or append a fresh one at the end.
    fn ensure_child(&mut self, token: T) -> &mut Node<T, V>
    where
        T: Clone,
    {
        let index = match self.child_index(&token) {
            Some(index) => index,
            None => {
                self.entries.push(Slot::Child(token, Node::new()));
                self.entries.len() - 1
            }
        };

        match &mut self.entries[index] {
            Slot::Child(_, child) => child,
            Slot::Word(_) => unreachable!("just inserted or looked up a Child slot"),
        }
    }

    /// `delete node[token]`.
    fn remove_child(&mut self, token: &T) -> Option<Node<T, V>> {
        let index = self.child_index(token)?;

        match self.entries.remove(index) {
            Slot::Child(_, child) => Some(child),
            Slot::Word(_) => unreachable!("child_index only ever finds a Child slot"),
        }
    }

    /// `delete node[SENTINEL]`.
    fn remove_word(&mut self) -> Option<V> {
        let index = self.word_index()?;

        match self.entries.remove(index) {
            Slot::Word(value) => Some(value),
            Slot::Child(..) => unreachable!("word_index only ever finds a Word slot"),
        }
    }

    /// `node[SENTINEL] = value`, in place if the key already existed
    /// (preserving enumeration position — an ordinary value overwrite is
    /// never a delete-then-reinsert), appended otherwise. Returns the value
    /// displaced, if any.
    fn set_word(&mut self, value: V) -> Option<V> {
        match self.word_index() {
            Some(index) => match &mut self.entries[index] {
                Slot::Word(existing) => Some(mem::replace(existing, value)),
                Slot::Child(..) => unreachable!("word_index only ever finds a Word slot"),
            },
            None => {
                self.entries.push(Slot::Word(value));

                None
            }
        }
    }

    /// The combined read-modify-write behind [`TrieMap::update`], as one call
    /// so the entry is never observably absent between reading the old value
    /// and writing the new one.
    ///
    /// Returns whether this created a new word (upstream's
    /// `!(SENTINEL in node)`, checked before the callback runs).
    fn update_word<F: FnOnce(Option<V>) -> V>(&mut self, update: F) -> bool {
        match self.word_index() {
            Some(index) => {
                let Slot::Word(old) = self.entries.remove(index) else {
                    unreachable!("word_index only ever finds a Word slot")
                };

                self.entries.insert(index, Slot::Word(update(Some(old))));

                false
            }
            None => {
                self.entries.push(Slot::Word(update(None)));

                true
            }
        }
    }
}

/// A live, resumable walk over a [`TrieMap`]'s stored words, depth-first.
///
/// This is `TrieMap.prototype.values`/`prefixes`/`entries`'s own bespoke
/// generator, not [`crate::cursor::Sequence`] — there is no frozen length
/// here at all, upstream's closure just holds two live JS arrays
/// (`nodeStack`/`prefixStack`) and re-scans whatever node it pops. [`Walk`]
/// reproduces the *shape* of that scan (see [`Walk::step`]) over **paths**
/// rather than live references, which is what lets it be resumed from a
/// fresh `&TrieMap` on every step — required at the FFI boundary, where
/// nothing can hold a borrow across calls. See the module docs for DIV-TRIE-MAP-2, the
/// one place this design disagrees with upstream's.
#[derive(Debug, Clone)]
pub struct Walk<T> {
    /// How many leading tokens of every path below are the caller's own
    /// starting prefix rather than part of a reported suffix. See
    /// [`Walk::step`].
    base_len: usize,
    /// Pending node paths to expand, LIFO — upstream's `nodeStack`, but each
    /// entry is the token path from the root rather than a live reference.
    pending: Vec<Vec<T>>,
}

impl<T: Clone + PartialEq> Walk<T> {
    fn new(base: Vec<T>) -> Self {
        Self {
            base_len: base.len(),
            pending: vec![base],
        }
    }

    /// `Iterator.empty()` — the prefix argument did not resolve to a node.
    fn empty() -> Self {
        Self {
            base_len: 0,
            pending: Vec::new(),
        }
    }

    /// One `next()`.
    ///
    /// Returns the **suffix** beyond the walk's starting prefix, not the full
    /// path — a caller that started the walk with a non-empty prefix (a JS
    /// value with its own, possibly uncoerced, shape) is responsible for
    /// prepending its own starting value to this suffix, exactly as
    /// upstream's `prefix + k` / `prefix.concat(k)` does. See
    /// `mnemonist_napi::trie_map` for where that concatenation happens.
    ///
    /// May silently walk through and discard any number of childless,
    /// wordless nodes before returning — upstream's own `while` loop does
    /// exactly this, since `for (k in currentNode)` never partially resumes
    /// a node once it has been popped.
    pub fn step<'a, V>(&mut self, map: &'a TrieMap<T, V>) -> Option<(Vec<T>, &'a V)> {
        while let Some(path) = self.pending.pop() {
            // DIV-TRIE-MAP-2: re-navigate from the root rather than dereference a
            // held pointer. A path that no longer resolves — the node it
            // named, or an ancestor of it, was pruned since this frame was
            // queued — is simply skipped, which is where this walk can
            // disagree with upstream's live-reference one.
            let Some(node) = map.navigate(path.iter().cloned()) else {
                continue;
            };

            let mut found = None;

            for entry in node.iter() {
                match entry {
                    Slot::Word(value) => found = Some(value),
                    Slot::Child(token, _child) => {
                        let mut next = path.clone();
                        next.push(token.clone());
                        self.pending.push(next);
                    }
                }
            }

            if let Some(value) = found {
                let suffix = path[self.base_len..].to_vec();

                return Some((suffix, value));
            }
        }

        None
    }
}

/// Upstream's `TrieMap`.
///
/// Generic in the token type `T` — a Rust caller might use `char`, `&str`, an
/// enum, anything comparable — and in the stored value `V`. The bridge
/// instantiates this with a JavaScript-flavoured token (see
/// `mnemonist_napi::trie_map`'s module docs); core never hears about strings,
/// UTF-16, or coercion at all.
#[derive(Debug, Clone)]
pub struct TrieMap<T, V> {
    root: Node<T, V>,
    size: usize,
}

impl<T, V> Default for TrieMap<T, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, V> TrieMap<T, V> {
    pub fn new() -> Self {
        Self {
            root: Node::new(),
            size: 0,
        }
    }

    /// Upstream's `size` property.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Upstream's `clear`: a fresh root, and `size` reset to zero.
    pub fn clear(&mut self) {
        self.root = Node::new();
        self.size = 0;
    }

    /// Every stored value, mutably, depth-first. Exists for the bridge: a
    /// value that holds a JS reference must have it released — on `clear`
    /// and at finalization — before it is dropped, which needs `&mut V`
    /// reachable without walking the tree by hand at every call site.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        let mut out = Vec::new();
        self.root.collect_values_mut(&mut out);

        out.into_iter()
    }
}

impl<T: Clone + PartialEq, V> TrieMap<T, V> {
    /// Upstream's `set`.
    ///
    /// Returns the value displaced, if the prefix was already stored —
    /// upstream returns `this` for chaining instead and exposes no way to
    /// see the old value, but the bridge and the fuzz spec both want it.
    pub fn set(&mut self, prefix: impl IntoIterator<Item = T>, value: V) -> Option<V> {
        let node = Self::ensure_path(&mut self.root, prefix);
        let displaced = node.set_word(value);

        if displaced.is_none() {
            self.size += 1;
        }

        displaced
    }

    /// Upstream's `update`.
    ///
    /// `update` is called with the old value (`None` for "not stored"),
    /// exactly once, and its result becomes the new stored value — matching
    /// `updateFunction(node[SENTINEL])` where reading an absent property is
    /// `undefined`.
    pub fn update<F: FnOnce(Option<V>) -> V>(&mut self, prefix: impl IntoIterator<Item = T>, f: F) {
        let node = Self::ensure_path(&mut self.root, prefix);

        if node.update_word(f) {
            self.size += 1;
        }
    }

    /// Upstream's `get`.
    pub fn get(&self, prefix: impl IntoIterator<Item = T>) -> Option<&V> {
        self.navigate(prefix)?.word()
    }

    /// Upstream's `has`.
    pub fn has(&self, prefix: impl IntoIterator<Item = T>) -> bool {
        self.navigate(prefix)
            .is_some_and(|node| node.word_index().is_some())
    }

    /// Upstream's `delete`.
    ///
    /// Returns the removed value, unlike upstream's plain boolean — the
    /// bridge needs it to release the JS reference a displaced value can
    /// hold; a caller that only wants the boolean uses `.is_some()`.
    ///
    /// Implemented as a standard recursive bottom-up prune — remove the
    /// terminal word, then remove any ancestor that has become entirely
    /// empty, stopping at the first one that has not — rather than upstream's
    /// single-pass "remember the highest safe truncation point" walk. The two
    /// are equivalent for every OBSERVABLE outcome (`root`, `size`, `has`,
    /// `find`, and every walk that is not already open across the delete —
    /// see DIV-TRIE-MAP-2): a node upstream's algorithm would prune from is, by
    /// construction, the root of a chain of nodes that each have fewer than
    /// two own keys, and removing the single reference at the top of that
    /// chain leaves exactly the same nodes unreachable as removing each one
    /// bottom-up would.
    pub fn delete(&mut self, prefix: impl IntoIterator<Item = T>) -> Option<V> {
        let tokens: Vec<T> = prefix.into_iter().collect();
        let removed = Self::delete_rec(&mut self.root, &tokens);

        removed.map(|(value, _child_now_empty)| {
            self.size -= 1;

            value
        })
    }

    /// One node's worth of upstream's `find`.
    ///
    /// Returns the **suffix** beyond `prefix` for every stored word reachable
    /// from it — see [`Walk::step`] for why the split matters and who is
    /// responsible for the other half.
    pub fn find(&self, prefix: impl IntoIterator<Item = T>) -> Vec<(Vec<T>, &V)> {
        let Some(start) = self.navigate(prefix) else {
            return Vec::new();
        };

        let mut matches = Vec::new();
        let mut stack: Vec<(&Node<T, V>, Vec<T>)> = vec![(start, Vec::new())];

        // Matches upstream's `nodeStack`/`prefixStack` push order and LIFO
        // pop order exactly: for every popped node, every entry is scanned in
        // insertion order, a `Word` is reported immediately, and a `Child` is
        // pushed to be visited — meaning the LAST child in insertion order at
        // a given node is visited NEXT, not the first.
        while let Some((node, suffix)) = stack.pop() {
            for entry in node.iter() {
                match entry {
                    Slot::Word(value) => matches.push((suffix.clone(), value)),
                    Slot::Child(token, child) => {
                        let mut next = suffix.clone();
                        next.push(token.clone());
                        stack.push((child, next));
                    }
                }
            }
        }

        matches
    }

    /// Upstream's `values`/`prefixes`/`keys`/`entries` — one shared walk;
    /// which of `(suffix, value)` a caller projects out is the only
    /// difference between the four.
    ///
    /// `prefix` is upstream's optional starting prefix; an empty one (the
    /// default when none is given) walks the whole trie.
    pub fn walk(&self, prefix: impl IntoIterator<Item = T>) -> Walk<T> {
        let base: Vec<T> = prefix.into_iter().collect();

        match self.navigate(base.iter().cloned()) {
            Some(_) => Walk::new(base),
            None => Walk::empty(),
        }
    }

    /// A read-only view of the root node's own entries — upstream's `root`
    /// property, generalized so `Node`/`Slot` never need to be public types.
    /// Exists for the bridge, which rebuilds the exact nested structure
    /// `assert.deepStrictEqual` compares `root` against.
    pub fn root(&self) -> NodeView<'_, T, V> {
        NodeView { node: &self.root }
    }

    fn navigate(&self, prefix: impl IntoIterator<Item = T>) -> Option<&Node<T, V>> {
        let mut node = &self.root;

        for token in prefix {
            node = node.child(&token)?;
        }

        Some(node)
    }

    fn ensure_path(root: &mut Node<T, V>, prefix: impl IntoIterator<Item = T>) -> &mut Node<T, V> {
        let mut node = root;

        for token in prefix {
            node = node.ensure_child(token);
        }

        node
    }

    /// Returns `Some((removed_value, child_is_now_empty))` when a word was
    /// actually removed, `None` when the prefix does not resolve to a stored
    /// word at all. `removed_value` is threaded up unchanged from the
    /// terminal frame; `child_is_now_empty` is recomputed at every level.
    fn delete_rec(node: &mut Node<T, V>, tokens: &[T]) -> Option<(V, bool)> {
        match tokens.split_first() {
            None => {
                let value = node.remove_word()?;

                Some((value, node.is_empty()))
            }
            Some((first, rest)) => {
                let child = node.child_mut(first)?;
                let (value, child_now_empty) = Self::delete_rec(child, rest)?;

                if child_now_empty {
                    node.remove_child(first);
                }

                Some((value, node.is_empty()))
            }
        }
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

    /// 1:1 port of the upstream `it` block set, as a baseline.
    #[test]
    fn reproduces_the_upstream_suite() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();

        trie.set(tokens("rat"), 1);
        trie.set(tokens("rate"), 2);
        trie.set(tokens("tar"), 3);

        assert_eq!(trie.size(), 3);
        assert_eq!(trie.get(tokens("rat")), Some(&1));
        assert_eq!(trie.get(tokens("rate")), Some(&2));
        assert_eq!(trie.get(tokens("tar")), Some(&3));
        assert_eq!(trie.get(tokens("show")), None);
        assert_eq!(trie.get(tokens("ra")), None);
        assert_eq!(trie.get(tokens("ratings")), None);
    }

    #[test]
    fn setting_the_same_prefix_again_does_not_increase_size() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();

        trie.set(tokens("rat"), 1);
        trie.set(tokens("rate"), 2);
        trie.set(tokens("rat"), 3);

        assert_eq!(trie.size(), 2);
        assert_eq!(trie.get(tokens("rat")), Some(&3));
    }

    #[test]
    fn the_null_sequence_is_a_valid_prefix() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();

        trie.set(tokens(""), 45);
        assert_eq!(trie.size(), 1);
        assert_eq!(trie.get(tokens("")), Some(&45));
    }

    #[test]
    fn update_calls_back_with_the_old_value_and_creates_when_absent() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();

        trie.update(tokens("rat"), |old| old.unwrap_or(0) + 1);
        trie.update(tokens("rate"), |old| old.unwrap_or(0) + 2);
        trie.update(tokens("rat"), |old| old.unwrap_or(0) + 3);

        assert_eq!(trie.size(), 2);
        assert_eq!(trie.get(tokens("rat")), Some(&4));
    }

    #[test]
    fn delete_removes_and_prunes_singleton_chains() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();

        trie.set(tokens("rat"), 1);
        trie.set(tokens("rate"), 2);
        trie.set(tokens("tar"), 3);

        assert!(trie.delete(tokens("")).is_none());
        assert!(trie.delete(tokens("hello")).is_none());

        assert_eq!(trie.delete(tokens("rat")), Some(1));
        assert!(!trie.has(tokens("rat")));
        assert!(trie.has(tokens("rate")));
        assert_eq!(trie.size(), 2);

        assert!(trie.delete(tokens("rate")).is_some());
        assert_eq!(trie.size(), 1);

        assert!(trie.delete(tokens("tar")).is_some());
        assert_eq!(trie.size(), 0);
    }

    /// `delete` on "rats" must not prune "rat", which is itself stored, even
    /// though it is the parent of the deleted word's only other branch.
    #[test]
    fn delete_does_not_prune_an_ancestor_that_is_itself_a_stored_word() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();

        trie.set(tokens("rat"), 1);
        trie.set(tokens("rats"), 2);
        trie.set(tokens("rate"), 3);

        assert_eq!(trie.delete(tokens("rats")), Some(2));
        assert!(trie.has(tokens("rat")));
        assert!(trie.has(tokens("rate")));
        assert_eq!(trie.size(), 2);
    }

    #[test]
    fn has_distinguishes_a_stored_word_from_a_mere_prefix_of_one() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();

        trie.set(tokens("romanesque"), 1);

        assert!(trie.has(tokens("romanesque")));
        assert!(!trie.has(tokens("roman")));
        assert!(!trie.has(tokens("")));
    }

    #[test]
    fn find_returns_the_suffix_beyond_the_given_prefix() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();

        trie.set(tokens("roman"), 1);
        trie.set(tokens("romanesque"), 2);
        trie.set(tokens("romanesques"), 3);
        trie.set(tokens("greek"), 4);

        let render = |prefix: &str| -> Vec<(String, u32)> {
            trie.find(tokens(prefix))
                .into_iter()
                .map(|(suffix, value)| (suffix_to_string(&suffix), *value))
                .collect()
        };

        assert_eq!(
            render("roman"),
            vec![
                (String::new(), 1),
                ("esque".into(), 2),
                ("esques".into(), 3)
            ]
        );
        assert_eq!(
            render("romanesque"),
            vec![(String::new(), 2), ("s".into(), 3)]
        );
        assert_eq!(render("hello"), Vec::<(String, u32)>::new());
        assert_eq!(
            render(""),
            vec![
                ("greek".into(), 4),
                ("roman".into(), 1),
                ("romanesque".into(), 2),
                ("romanesques".into(), 3),
            ]
        );
    }

    fn walk_all(trie: &TrieMap<char, u32>) -> Vec<(String, u32)> {
        let mut walk = trie.walk(std::iter::empty());
        let mut out = Vec::new();

        while let Some((suffix, value)) = walk.step(trie) {
            out.push((suffix_to_string(&suffix), *value));
        }

        out
    }

    #[test]
    fn walk_visits_every_word_in_the_same_order_as_find() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();

        trie.set(tokens("rat"), 1);
        trie.set(tokens("rate"), 2);

        assert_eq!(walk_all(&trie), vec![("rat".into(), 1), ("rate".into(), 2)]);

        trie.set(tokens("rater"), 3);
        trie.set(tokens("rates"), 4);

        let mut walk = trie.walk(tokens("rate"));
        let mut out = Vec::new();

        while let Some((suffix, value)) = walk.step(&trie) {
            out.push((suffix_to_string(&suffix), *value));
        }

        assert_eq!(
            out,
            vec![(String::new(), 2), ("s".into(), 4), ("r".into(), 3)]
        );
    }

    #[test]
    fn walk_over_a_prefix_that_does_not_exist_is_empty() {
        let trie: TrieMap<char, u32> = TrieMap::new();
        let mut walk = trie.walk(tokens("nope"));

        assert!(walk.step(&trie).is_none());
    }

    /// DIV-TRIE-MAP-1: a token equal to what upstream reserves as SENTINEL is an
    /// ordinary token here, never colliding with a node's own value.
    #[test]
    fn a_token_equal_to_the_sentinel_character_is_an_ordinary_token() {
        let sentinel = '\u{0}';
        let mut trie: TrieMap<char, &'static str> = TrieMap::new();

        trie.set(tokens("a"), "word-a");
        trie.set(
            "a".chars()
                .chain(std::iter::once(sentinel))
                .chain("b".chars()),
            "word-a0b",
        );

        // Unlike upstream (BUG-TRIE-MAP-1: size overcounts and the second value is
        // lost), both words are genuinely stored here.
        assert_eq!(trie.size(), 2);
        assert_eq!(trie.get(tokens("a")), Some(&"word-a"));
        assert_eq!(
            trie.get(
                "a".chars()
                    .chain(std::iter::once(sentinel))
                    .chain("b".chars())
            ),
            Some(&"word-a0b")
        );
    }

    /// DIV-TRIE-MAP-2: an add reachable from a frame the walk has queued but not yet
    /// expanded IS seen, because each step re-reads the live node — matching
    /// upstream, which is reading the same still-linked object live. Needs
    /// two branches so the walk returns with the interesting one
    /// (`'a'`'s subtree) still pending rather than draining it in the same
    /// call that discovers it.
    #[test]
    fn an_addition_inside_an_already_queued_branch_is_visible_to_an_open_walk() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();
        trie.set(tokens("a"), 1);
        trie.set(tokens("b"), 2);

        let mut walk = trie.walk(std::iter::empty());
        // Root's children are pushed in insertion order (`a` then `b`) and
        // popped LIFO, so `b` — with no children of its own — is visited,
        // and fully drained, first; `a`'s frame is still pending afterwards.
        assert_eq!(walk.step(&trie), Some((Vec::from(['b']), &2u32)));

        // `a` has not been expanded yet: this lands a new child under the
        // SAME node object the pending frame will re-navigate to.
        trie.set(tokens("ac"), 3);

        assert_eq!(walk.step(&trie), Some((Vec::from(['a']), &1u32)));
        assert_eq!(walk.step(&trie), Some((Vec::from(['a', 'c']), &3u32)));
        assert_eq!(walk.step(&trie), None);
    }

    /// Exercised for the bridge's benefit: every stored value must be
    /// reachable mutably, in one pass, so a JS reference each one holds can
    /// be released before the map itself is dropped.
    #[test]
    fn values_mut_reaches_every_stored_value() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();

        trie.set(tokens("rat"), 1);
        trie.set(tokens("rate"), 2);
        trie.set(tokens("tar"), 3);

        let mut seen: Vec<u32> = trie.values_mut().map(|value| *value).collect();
        seen.sort_unstable();

        assert_eq!(seen, vec![1, 2, 3]);
    }

    /// `root()`'s own entries mirror what a plain-object-based upstream
    /// would enumerate: value before children, in insertion order.
    #[test]
    fn root_exposes_entries_in_insertion_order() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();
        trie.set(tokens("a"), 1);
        trie.set(tokens("ab"), 2);

        let root = trie.root();
        let mut top = root.entries();

        let Some(Entry::Child(token, child)) = top.next() else {
            panic!("expected a single child 'a' at the root");
        };
        assert_eq!(*token, 'a');
        assert!(top.next().is_none());

        let mut a_entries = child.entries();

        let Some(Entry::Word(value)) = a_entries.next() else {
            panic!("expected 'a' node's own value first");
        };
        assert_eq!(*value, 1);

        let Some(Entry::Child(token, _)) = a_entries.next() else {
            panic!("expected 'a' node's child 'b' second");
        };
        assert_eq!(*token, 'b');
        assert!(a_entries.next().is_none());
    }

    #[test]
    fn clear_resets_size_and_removes_everything() {
        let mut trie: TrieMap<char, u32> = TrieMap::new();
        trie.set(tokens("a"), 1);
        trie.set(tokens("b"), 2);

        trie.clear();

        assert_eq!(trie.size(), 0);
        assert!(!trie.has(tokens("a")));
        assert_eq!(walk_all(&trie), Vec::<(String, u32)>::new());
    }
}

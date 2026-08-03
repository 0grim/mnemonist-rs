//! Port of upstream `default-weak-map.js` (v0.40.4).
//!
//! `DefaultMap`'s twin: `get` manufactures and stores a value from a factory
//! when a key is unseen, `peek` does the same lookup without the factory, and
//! `set`/`has`/`delete`/`clear` delegate straight to a backing `WeakMap`. The
//! whole file is 99 lines because a `WeakMap` gives it nothing to manage:
//! no `size`, no iteration, no ordering — see the module docs for
//! [`crate::structures::default_map`], read first, for the shape this one
//! specialises.
//!
//! # What a `WeakMap` removes from the surface, and why that matters here
//!
//! A JS `WeakMap` holds its keys **weakly**: an entry is eligible for
//! reclamation the moment nothing outside the map still references its key,
//! and JavaScript deliberately gives no way to observe when that happens —
//! there is no `size`, no `forEach`, no `keys`/`values`/`entries`, not even a
//! count. `DefaultWeakMap` inherits that exactly: its whole public surface is
//! `clear`, `get`, `peek`, `set`, `has`, `delete` — six methods, all of them
//! keyed on one still-referenced object at a time, none of them a "the whole
//! map" question.
//!
//! That is not a testing inconvenience to work around; it is the reason a
//! `WeakMap` exists at all, and it means this module's fuzz spec
//! (`crates/difffuzz/src/modules/default_weak_map.rs`) has an easier job than
//! `default-map`'s in one respect and a harder one in another:
//!
//! * **Easier:** there is no `size`/`items` pair to compare after every op —
//!   upstream never exposes one, so there is nothing to omit and nothing to
//!   invent. Every comparison this unit's fuzzer makes is a return value:
//!   `get`, `peek`, `has`, `delete`, `set`'s chaining `this`. That is not a
//!   narrower campaign than it should be; it is the **entire** observable
//!   surface, faithfully covered.
//! * **Harder:** the keys have to be objects with a *stable identity* the
//!   oracle and the port can each recognise as "the same key" across many
//!   calls, and — because this project keeps campaigns closed to
//!   non-determinism (`planning/NOTES.md`'s running theme) — the fuzz grammar
//!   deliberately never asks either side whether an unreferenced key has been
//!   reclaimed. Garbage collection timing is not observable through this
//!   module's API in the first place (nothing above lists it), so there is no
//!   faithful oracle answer to invent one for. See the fuzz spec's own docs
//!   for the object-identity pool this uses instead.
//!
//! # Why this is a linear scan, not a `HashMap`
//!
//! `DefaultMap` is generic over `K: Hash + Eq + Clone` and stores its entries
//! in [`crate::map::OrderedMap`]. This module cannot do that: `WeakMap`
//! compares keys by **identity**, and there is no hash for JS object identity
//! reachable from a value-agnostic core (see
//! `crates/mnemonist-napi/src/js_key.rs`'s own discussion of the identical
//! problem for `Map` keys — rejected there because no T3 test uses an object
//! key; unavoidable here, because an object key is the *entire point* of a
//! `WeakMap`). So [`DefaultWeakMap`] takes an identity test as a **predicate
//! supplied per call**, `impl FnMut(&K) -> bool`, rather than requiring
//! `K: Eq`: a predicate can close over whatever the caller needs to decide
//! "is this the key I mean" — a napi `Env` and a live `napi_value`, for the
//! bridge, or a plain integer for the fuzz spec's own mirror — none of which
//! is a comparison core could express as a trait bound without knowing what
//! `K` means. Every lookup is therefore O(n) in the number of live entries.
//! Correct, not fast, and stated as such: nothing about this 60-line upstream
//! test file, or a `WeakMap`'s own contract, asks for anything faster.
//!
//! # The `get`/`peek` split reproduces the same defect class as BUG-DEFAULT-MAP-1, minus the drift
//!
//! ```js
//! DefaultWeakMap.prototype.get = function(key) {
//!   var value = this.items.get(key);
//!   if (typeof value === 'undefined') {     // tests the VALUE, not the key
//!     value = this.factory(key);
//!     this.items.set(key, value);
//!   }
//!   return value;
//! };
//! ```
//!
//! Identical shape to `DefaultMap.prototype.get` — a stored value of
//! `undefined` is indistinguishable, at this line, from "no such key" — but
//! **without** the `this.size++` that makes BUG-DEFAULT-MAP-1's drift visible, because a
//! `WeakMap` has no `size` to drift. The consequence that remains: the
//! factory **re-runs on every `get`** of a key whose stored value is
//! `undefined`, and [`DefaultWeakMap::has`] still reports that key present the
//! whole time, because `has` asks the `WeakMap` about the key and `get`'s
//! bug asks about the value. Confirmed against Node 24.18.1 and recorded as
//! **BUG-DEFAULT-WEAK-MAP-1** in NOTES.md — the same defect, a different file, one fewer
//! symptom.
//!
//! [`DefaultWeakMap::write_from_factory`] and [`DefaultWeakMap::set`] both
//! reuse the *stored* key predicate match rather than allocating a new
//! identity for a key already present — see their docs — which is what keeps
//! a re-triggered factory from leaking a fresh weak reference on every read,
//! at the bridge.
//!
//! # `undefined` is spelled `None`, exactly as in `default-map`
//!
//! Same reasoning, same shape: [`DefaultWeakMap<K, V>`] stores `Option<V>`
//! internally, `None` is `undefined`, and [`DefaultWeakMap::peek`] flattens
//! "missing key" and "stored `undefined`" into one `None`, because upstream's
//! own `items.get(key)` cannot tell them apart either.

/// Upstream's `DefaultWeakMap`.
///
/// See the module docs for why this is a linear scan over `(K, Option<V>)`
/// rather than a hash table, and why `K` carries no `Eq`/`Hash` bound at all
/// — identity is a predicate supplied per call, not a trait `core` could
/// express without knowing what a "key" means here.
pub struct DefaultWeakMap<K, V> {
    entries: Vec<(K, Option<V>)>,
}

impl<K, V> Default for DefaultWeakMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> DefaultWeakMap<K, V> {
    /// An empty map — `new DefaultWeakMap(factory)`, minus the factory, which
    /// this port takes per call. See [`DefaultWeakMap`] on why the backing
    /// store is a `Vec` and not a hash map.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Upstream's `clear`: `this.items = new WeakMap();`.
    ///
    /// Drops every stored key and value. Unlike `LinkedList`'s arena, there
    /// is no "a live cursor might still need this" concern to preserve here
    /// — this module has no cursors at all (see the module docs) — so a
    /// clear can and does drop everything immediately.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Every stored value, mutably, for the bridge's `clear`/finalize to
    /// release before the entries themselves are dropped.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Option<V>> {
        self.entries.iter_mut().map(|(_, value)| value)
    }

    /// Every stored `(key, value)` pair, mutably. The bridge needs both
    /// halves on `clear`/finalize: the value may hold a retained JS
    /// reference to release, and the key (there, a genuinely weak `napi_ref`
    /// wrapper) holds one of its own to delete — a release core cannot do
    /// itself, since a plain `K` here carries no JavaScript-specific meaning
    /// at all.
    pub fn entries_mut(&mut self) -> impl Iterator<Item = (&mut K, &mut Option<V>)> {
        self.entries.iter_mut().map(|(key, value)| (key, value))
    }

    /// Index of the first entry whose key satisfies `matches`, or `None`.
    ///
    /// Private: every public method needs "find, then do something with the
    /// slot", never the bare index.
    fn position(&self, mut matches: impl FnMut(&K) -> bool) -> Option<usize> {
        self.entries.iter().position(|(key, _)| matches(key))
    }

    /// Upstream's `peek`: `this.items.get(key)`, with no factory call.
    ///
    /// `None` covers both "no matching key" and "the matching key's value is
    /// `undefined`" — upstream's own `items.get` cannot distinguish them
    /// either.
    pub fn peek(&self, matches: impl FnMut(&K) -> bool) -> Option<&V> {
        self.position(matches)
            .and_then(|index| self.entries[index].1.as_ref())
    }

    /// Upstream's `has`: `this.items.has(key)`, which asks about the key and
    /// not the value — the distinction BUG-DEFAULT-WEAK-MAP-1 depends on.
    pub fn has(&self, matches: impl FnMut(&K) -> bool) -> bool {
        self.position(matches).is_some()
    }

    /// Upstream's `delete`: `this.items.delete(key)`.
    ///
    /// `None` is upstream's `false` (no such key). `Some((key, value))` is
    /// `true`, carrying **both** halves of what was removed — `value` is
    /// `None` if that was a stored `undefined`.
    ///
    /// The removed `K` is handed back rather than dropped here on purpose:
    /// at the bridge, `K` is `mnemonist_napi::default_weak_map::WeakKey`,
    /// which owns a `napi_ref` that must be explicitly deleted with an `Env`
    /// core does not have. Silently dropping it here — the first cut of
    /// this method did exactly that, keeping only `.1` — leaks that
    /// reference. Found under `tests/bridge/default-weak-map.js`'s own
    /// `it('should be possible to delete keys.')` block, and only under a
    /// forced GC (`node --expose-gc`) that let the finalizer's absence show:
    /// a plain `u32` mirror key in the fuzz spec has nothing of its own to
    /// leak, so the differential fuzzer could not have caught this either —
    /// see `docs/modules/default-weak-map.md`.
    pub fn delete(&mut self, matches: impl FnMut(&K) -> bool) -> Option<(K, Option<V>)> {
        self.position(matches)
            .map(|index| self.entries.remove(index))
    }

    /// The shared write path behind [`DefaultWeakMap::set`] and
    /// [`DefaultWeakMap::write_from_factory`]: overwrite in place if a
    /// matching key is already stored, otherwise insert a fresh entry.
    ///
    /// `make_key` is called **at most once, and only on a miss** — the whole
    /// reason this is a method of its own rather than inlined twice. A
    /// factory re-triggered by BUG-DEFAULT-WEAK-MAP-1 matches the *existing* entry and simply
    /// overwrites its value, never allocating a second identity for the same
    /// underlying key. At the bridge, where `make_key` creates a weak
    /// `napi_ref`, that is what keeps re-reading an `undefined`-valued key
    /// from leaking one reference per read.
    ///
    /// Returns the index written, so callers can hand back a reference into
    /// `entries` without a second lookup.
    fn upsert(
        &mut self,
        matches: impl FnMut(&K) -> bool,
        make_key: impl FnOnce() -> K,
        value: Option<V>,
    ) -> (usize, Option<V>) {
        match self.position(matches) {
            Some(index) => {
                let displaced = std::mem::replace(&mut self.entries[index].1, value);

                (index, displaced)
            }
            None => {
                self.entries.push((make_key(), value));

                (self.entries.len() - 1, None)
            }
        }
    }

    /// Upstream's `set`, minus the chaining `return this` — the bridge's
    /// job.
    ///
    /// Returns the **defined** value it displaced, so a caller managing
    /// external resources (the bridge's retained JS values) knows whether
    /// there is anything to release: `None` covers both "no previous entry"
    /// and "the previous entry held `undefined`", neither of which needs
    /// releasing.
    pub fn set(
        &mut self,
        matches: impl FnMut(&K) -> bool,
        make_key: impl FnOnce() -> K,
        value: Option<V>,
    ) -> Option<V> {
        self.upsert(matches, make_key, value).1
    }

    /// The write half of upstream's `get`, for a caller that has already run
    /// the factory (which, at the bridge, is a JS callback that must run
    /// with nothing borrowed — see `crates/mnemonist-napi/src/default_weak_map.rs`
    /// and `default_map`'s identical split for why).
    ///
    /// Unconditional, exactly like `DefaultMap::insert_from_factory`:
    /// upstream does not re-check the key after the factory returns, so a
    /// factory that inserted the same key itself is simply overwritten here
    /// too.
    pub fn write_from_factory(
        &mut self,
        matches: impl FnMut(&K) -> bool,
        make_key: impl FnOnce() -> K,
        value: Option<V>,
    ) -> Option<&V> {
        let (index, _) = self.upsert(matches, make_key, value);

        self.entries[index].1.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny identity-keyed instance: `K = u32`, matched by equality. Stands
    /// in for the bridge's `napi_strict_equals`-based predicate, which this
    /// module cannot exercise from pure Rust (see the module docs).
    fn fresh_map() -> DefaultWeakMap<u32, Vec<u32>> {
        DefaultWeakMap::new()
    }

    fn eq(key: u32) -> impl FnMut(&u32) -> bool {
        move |candidate: &u32| *candidate == key
    }

    impl<K: PartialEq + Copy, V> DefaultWeakMap<K, V> {
        /// Test-only: a mutable handle to a stored, defined value — the
        /// upstream idiom `map.get(key).push(v)` needs a mutable reference
        /// back into the map, which this module's real API deliberately does
        /// not expose (see the module docs: every public accessor takes a
        /// predicate, never a bare `K: Eq`). Mirrors `default_map`'s own
        /// `items_mut_for_test`.
        fn value_mut_for_test(&mut self, key: K) -> &mut V {
            self.entries
                .iter_mut()
                .find(|(candidate, _)| *candidate == key)
                .expect("the key was just materialised")
                .1
                .as_mut()
                .expect("the factory returned a defined value")
        }
    }

    /// `map.get(key).push(v)` — the same idiom `default-map`'s suite is built
    /// on, translated to this module's split API: `peek` first, and if that
    /// misses, run the "factory" and write it with `write_from_factory`.
    fn get_or_insert(
        map: &mut DefaultWeakMap<u32, Vec<u32>>,
        key: u32,
        factory: impl FnOnce() -> Vec<u32>,
    ) -> &Vec<u32> {
        if map.peek(eq(key)).is_none() {
            let value = factory();
            map.write_from_factory(eq(key), || key, Some(value));
        }

        map.peek(eq(key)).expect("just confirmed or inserted")
    }

    // ---- 1:1 port of the upstream suite, as a baseline --------------------

    #[test]
    fn reproduces_the_upstream_suite() {
        // …set & get, with unknown keys manufacturing an empty Vec…
        let mut map = fresh_map();
        let (one, two, unknown) = (1, 2, 3);

        get_or_insert(&mut map, one, Vec::new);
        map.value_mut_for_test(one).push(1);
        map.set(eq(two), || two, Some(vec![2]));

        assert_eq!(map.peek(eq(one)), Some(&vec![1]));
        assert_eq!(map.peek(eq(two)), Some(&vec![2]));
        assert_eq!(
            get_or_insert(&mut map, unknown, Vec::new),
            &Vec::<u32>::new()
        );

        map.clear();
        assert_eq!(get_or_insert(&mut map, one, Vec::new), &Vec::<u32>::new());

        // …delete…
        let mut deletes: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();
        deletes.set(eq(one), || one, Some(1));
        assert!(deletes.has(eq(one)));
        assert_eq!(deletes.delete(eq(one)), Some((one, Some(1))));
        assert!(!deletes.has(eq(one)));
        assert_eq!(deletes.delete(eq(one)), None);

        // …peek…
        let mut peeked = fresh_map();
        get_or_insert(&mut peeked, one, Vec::new);
        peeked.value_mut_for_test(one).push(1);
        assert_eq!(peeked.peek(eq(one)), Some(&vec![1]));
        assert_eq!(peeked.peek(eq(two)), None);
        assert!(!peeked.has(eq(two)));
    }

    // ---- BUG-DEFAULT-WEAK-MAP-1 -------------------------------------------------------

    #[test]
    fn b_242_the_factory_re_runs_on_every_get_of_a_stored_undefined_value() {
        let mut map: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();
        let key = 1;
        let mut calls = 0;

        map.set(eq(key), || key, None);

        for _ in 0..3 {
            let ran = map.peek(eq(key)).is_none();
            if ran {
                calls += 1;
                map.write_from_factory(eq(key), || key, None);
            }
        }

        assert_eq!(calls, 3, "the factory re-ran on every read");
        assert!(
            map.has(eq(key)),
            "`has` asks about the key, which is present the whole time"
        );
        assert_eq!(map.peek(eq(key)), None);
    }

    #[test]
    fn a_defined_value_written_by_the_factory_ends_the_b_242_re_run() {
        let mut map: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();
        map.set(eq(1), || 1, None);

        map.write_from_factory(eq(1), || 1, Some(7));
        assert_eq!(map.peek(eq(1)), Some(&7));

        // A second "get" now hits the defined value and does not re-run.
        assert_eq!(map.peek(eq(1)), Some(&7));
    }

    #[test]
    fn a_re_triggered_factory_overwrites_in_place_rather_than_duplicating_the_key() {
        let mut map: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();
        map.set(eq(1), || 1, None);
        map.write_from_factory(eq(1), || panic!("must not allocate a new key"), None);
        map.write_from_factory(eq(1), || panic!("must not allocate a new key"), Some(9));

        assert_eq!(map.entries.len(), 1, "one entry, overwritten twice");
        assert_eq!(map.peek(eq(1)), Some(&9));
    }

    // ---- Everything else --------------------------------------------

    #[test]
    fn set_overwrites_an_existing_key_in_place() {
        let mut map: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();

        assert_eq!(map.set(eq(1), || 1, Some(1)), None);
        assert_eq!(map.set(eq(1), || 1, Some(2)), Some(1));
        assert_eq!(map.set(eq(1), || 1, None), Some(2));
        assert_eq!(
            map.set(eq(1), || 1, Some(3)),
            None,
            "undefined displaced nothing defined"
        );
        assert_eq!(map.entries.len(), 1);
    }

    #[test]
    fn delete_distinguishes_a_missing_key_from_a_stored_undefined() {
        let mut map: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();
        map.set(eq(1), || 1, None);

        assert_eq!(
            map.delete(eq(1)),
            Some((1, None)),
            "removed, value was undefined"
        );
        assert_eq!(map.delete(eq(1)), None, "upstream would return false");
    }

    /// The port's own bug, found late: an earlier cut of `delete` kept only
    /// the removed *value*, dropping the removed key inline. At the bridge
    /// the key owns a `napi_ref` a caller must delete with an `Env`, which
    /// `mnemonist-core` never has -- so that drop ran through Rust's
    /// ordinary `Drop`, silently, with nothing to release it. Pinned here so
    /// a future refactor of this method cannot reintroduce it without a
    /// clean Rust test going red.
    #[test]
    fn delete_hands_back_the_removed_key_as_well_as_the_value() {
        let mut map: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();
        map.set(eq(7), || 7, Some(1));

        assert_eq!(map.delete(eq(7)), Some((7, Some(1))));
    }

    #[test]
    fn has_and_peek_disagree_on_a_stored_undefined() {
        let mut map: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();
        map.set(eq(1), || 1, None);

        assert!(map.has(eq(1)));
        assert_eq!(map.peek(eq(1)), None);
    }

    #[test]
    fn clear_drops_every_entry() {
        let mut map: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();
        map.set(eq(1), || 1, Some(1));
        map.set(eq(2), || 2, None);

        map.clear();

        assert!(!map.has(eq(1)));
        assert!(!map.has(eq(2)));
        assert_eq!(map.entries.len(), 0);
    }

    #[test]
    fn an_empty_map_reports_nothing() {
        let map: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();

        assert_eq!(map.peek(eq(1)), None);
        assert!(!map.has(eq(1)));
    }

    #[test]
    fn values_mut_reaches_every_stored_slot_including_the_undefined_ones() {
        let mut map: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();
        map.set(eq(1), || 1, Some(1));
        map.set(eq(2), || 2, None);

        let seen: Vec<Option<u32>> = map.values_mut().map(|value| *value).collect();

        assert_eq!(seen, vec![Some(1), None]);
    }

    #[test]
    fn identity_not_content_decides_a_match_two_equal_but_distinct_keys() {
        // Two DIFFERENT identities that happen to compare equal under a
        // naive `==` would still be two entries under a real WeakMap, since
        // it compares by reference, not value. This module cannot enforce
        // that itself -- identity is entirely the caller's predicate -- but
        // it must not collapse two calls with two DIFFERENT predicates into
        // one entry.
        let mut map: DefaultWeakMap<u32, u32> = DefaultWeakMap::new();

        // Two "keys" that use the SAME underlying id (1) but are matched by
        // predicates that never consider each other equal, modelling two
        // distinct JS objects that happen to carry the same debug id.
        let mut seen_first = false;
        map.set(
            |_candidate: &u32| false, // never matches: forces an insert
            || 1,
            Some(10),
        );
        map.set(
            move |_candidate: &u32| {
                if seen_first {
                    return false;
                }
                seen_first = true;
                false
            },
            || 1,
            Some(20),
        );

        assert_eq!(map.entries.len(), 2, "two distinct identities, two entries");
    }
}

//! Port of upstream `fuzzy-multi-map.js` (mnemonist v0.40.4).
//!
//! "Same as the fuzzy map but relying on a `MultiMap` rather than a `Map`" —
//! upstream's own header comment, verbatim, and it is exactly this thin here
//! too: a [`FuzzyMultiMap`] is a [`crate::structures::multi_map::MultiMap`]
//! whose keys are always the *result of a hash function*, applied by the
//! bridge before core ever sees one — the same division `crate::structures::
//! fuzzy_map` draws for its own hash functions.
//!
//! Every method upstream's `FuzzyMultiMap` defines delegates straight to
//! `this.items`'s (a `MultiMap`) own method of the same name, so this module
//! adds no new *behaviour* over `MultiMap` — only the wrapping.

use std::hash::Hash;

use crate::structures::multi_map::{Bucket, ContainerKind, MultiMap};

/// Upstream's `FuzzyMultiMap`.
#[derive(Debug, Clone)]
pub struct FuzzyMultiMap<K, V> {
    items: MultiMap<K, V>,
}

impl<K, V> FuzzyMultiMap<K, V> {
    /// `new FuzzyMultiMap(descriptor, Container)`, already past the
    /// `descriptor` → hash-function resolution the bridge performs.
    pub fn new(kind: ContainerKind) -> Self {
        Self {
            items: MultiMap::new(kind),
        }
    }

    /// Upstream's `size`: `this.items.size`, kept in lockstep by every write
    /// going through [`FuzzyMultiMap::set_with`].
    pub fn size(&self) -> usize {
        self.items.size()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// The wrapped `MultiMap`, for the bridge's flattened value cursor
    /// (`values()`/`forEach`, which discard the hashed key and keep only the
    /// value — see `mnemonist_napi::fuzzy_multi_map`).
    pub fn items(&self) -> &MultiMap<K, V> {
        &self.items
    }

    /// Every stored value, mutably — see `MultiMap::values_mut`.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.items.values_mut()
    }
}

impl<K: Hash + Eq + Clone, V: Clone> FuzzyMultiMap<K, V> {
    /// Upstream's `dimension`: `this.items.dimension`.
    pub fn dimension(&self) -> usize {
        self.items.dimension()
    }

    /// Upstream's `has`, given the already-hashed key.
    pub fn has(&self, key: &K) -> bool {
        self.items.has(key)
    }

    /// Upstream's `get`, given the already-hashed key — the whole bucket
    /// (`undefined` for a key never set, matching `MultiMap::get`).
    pub fn get(&self, key: &K) -> Option<&Bucket<V>> {
        self.items.get(key)
    }

    /// The write half of both upstream's `add` and `set` — see
    /// `crate::structures::fuzzy_map::FuzzyMap::set` for why both funnel
    /// through one function here: the difference between them is entirely
    /// in *which* value the bridge hashes to get `key` (the item itself for
    /// `add`, the caller's key for `set`), which is JavaScript's concern.
    ///
    /// Takes the fallible form directly, never the `V: PartialEq`
    /// convenience `MultiMap::set` provides: `FuzzyMultiMap`'s own values
    /// are exactly the case `MultiMap`'s module docs motivate this for —
    /// arbitrary JS values, including plain objects, whose `Set`-kind
    /// membership is JavaScript's SameValueZero and may need to call back
    /// into the engine to decide.
    ///
    /// Returns the candidate back, unstored, exactly when
    /// `MultiMap::set_with` would — see that method's docs.
    pub fn set_with<E>(
        &mut self,
        key: K,
        value: V,
        eq: impl Fn(&V, &V) -> Result<bool, E>,
    ) -> Result<Option<V>, E> {
        self.items.set_with(key, value, eq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_infallible<K: Hash + Eq + Clone, V: Clone + PartialEq>(
        map: &mut FuzzyMultiMap<K, V>,
        key: K,
        value: V,
    ) {
        let outcome: Result<Option<V>, std::convert::Infallible> =
            map.set_with(key, value, |a, b| Ok(a == b));

        outcome.unwrap();
    }

    #[test]
    fn reproduces_the_upstream_walkthrough() {
        // Hashing itself is the bridge's job; the test hashes by hand,
        // mirroring `item.title.toLowerCase()`.
        let mut map: FuzzyMultiMap<&str, &str> = FuzzyMultiMap::new(ContainerKind::List);

        set_infallible(&mut map, "hello", "Hello");
        set_infallible(&mut map, "hello", "Hello");
        set_infallible(&mut map, "world", "World");

        assert_eq!(map.size(), 3);
        assert_eq!(map.dimension(), 2);
    }

    #[test]
    fn clear_resets_size_and_dimension() {
        let mut map: FuzzyMultiMap<&str, &str> = FuzzyMultiMap::new(ContainerKind::List);

        set_infallible(&mut map, "hello", "Hello");
        set_infallible(&mut map, "world", "World");
        map.clear();

        assert_eq!(map.size(), 0);
        assert_eq!(map.dimension(), 0);
    }

    #[test]
    fn get_returns_every_item_hashed_to_the_same_key() {
        let mut map: FuzzyMultiMap<&str, &str> = FuzzyMultiMap::new(ContainerKind::List);

        set_infallible(&mut map, "hello", "Hello1");
        set_infallible(&mut map, "hello", "Hello2");
        set_infallible(&mut map, "world", "World");

        assert_eq!(
            map.get(&"hello").unwrap().values(),
            &["Hello1", "Hello2"] as &[&str]
        );
        assert!(map.get(&"shawarama").is_none());
    }

    #[test]
    fn has_matches_the_hashed_key() {
        let mut map: FuzzyMultiMap<&str, &str> = FuzzyMultiMap::new(ContainerKind::List);

        set_infallible(&mut map, "hello", "hello");

        assert!(map.has(&"hello"));
        assert!(!map.has(&"test"));
    }

    #[test]
    fn set_kind_deduplicates_by_the_supplied_equality() {
        let mut map: FuzzyMultiMap<&str, i32> = FuzzyMultiMap::new(ContainerKind::Set);

        set_infallible(&mut map, "hello", 1);
        set_infallible(&mut map, "hello", 2);
        set_infallible(&mut map, "hello", 1);

        assert_eq!(map.size(), 2);
        assert_eq!(map.get(&"hello").unwrap().values(), &[1, 2]);
    }
}

//! Port of upstream `fuzzy-map.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! A `FuzzyMap` is a `Map` whose keys are computed by a hash function before
//! every read or write, so several distinct queries can resolve to the same
//! stored item — "a map with lowercased keys" is upstream's own example.
//! Everything about *hashing* is a JavaScript concern (the hash function is a
//! user-supplied callback) and lives in the bridge; this module is the T3
//! inheritance minus the one thing `default-map` has that this does not: a
//! factory that manufactures a *missing* value. `FuzzyMap` has no such thing —
//! a miss is just `undefined`, exactly like a plain `Map` — so this is
//! `default-map`'s core shape with `get_or_insert_with` deleted and `set`
//! reinstated as the only write.
//!
//! # `Option<V>` for the same reason as `default-map`
//!
//! `this.items.get(key)` returns `undefined` for both "no such key" and "the
//! key holds `undefined`", and upstream's `get`/`has` therefore diverge on a
//! stored `undefined` the same way `default-map`'s do: `get` cannot tell the
//! two apart, `has` can. So the value type here is `Option<V>` with `None`
//! spelling `undefined`, exactly as in
//! [`crate::structures::default_map::DefaultMap`].
//!
//! # No `delete`
//!
//! Upstream's `fuzzy-map.js` does not define one — `clear` is the only way to
//! shrink a `FuzzyMap`. Nothing here should invent one.
//!
//! # `forEach` walks values twice, not entries
//!
//! ```js
//! FuzzyMap.prototype.forEach = function(callback, scope) {
//!   scope = arguments.length > 1 ? scope : this;
//!   this.items.forEach(function(value) {
//!     callback.call(scope, value, value);
//!   });
//! };
//! ```
//!
//! The inner callback upstream hands to the backing `Map`'s own `forEach`
//! declares one parameter, discarding the key `Map.prototype.forEach` would
//! also supply — and calls the *user's* callback with `value` twice, not with
//! `(value, key)`. So a `FuzzyMap` walk (core or bridge) never needs the key
//! half of an entry at all; see [`FuzzyMap::values_mut`] and the bridge's
//! `for_each`, which project the value out of the cursor step twice.

use std::hash::Hash;

use crate::map::{MapCursor, OrderedMap};

/// Upstream's `FuzzyMap`.
///
/// `V` is the *defined* value type; the map stores `Option<V>` and `None` is
/// JavaScript's `undefined`. The hash function(s) are not stored here — they
/// are JavaScript callbacks, applied by the bridge before any core call, the
/// same division `DefaultMap`'s factory makes.
#[derive(Debug, Clone)]
pub struct FuzzyMap<K, V> {
    items: OrderedMap<K, Option<V>>,
}

impl<K, V> Default for FuzzyMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> FuzzyMap<K, V> {
    /// An empty map — `new FuzzyMap(hashFunction)`, minus the hash function:
    /// callers hash the key themselves and pass the result to
    /// [`FuzzyMap::set`], keeping the JavaScript callback at the boundary.
    pub fn new() -> Self {
        Self {
            items: OrderedMap::new(),
        }
    }

    /// Upstream's `size` property. Unlike `default-map`'s, this cannot drift:
    /// every write goes through [`FuzzyMap::set`], which always resynchronises
    /// from `items.len()` (there is no `get`-time auto-insert to lose track
    /// of).
    pub fn size(&self) -> usize {
        self.items.len()
    }

    /// The backing map, for the bridge's cursors and the differential fuzzer.
    pub fn items(&self) -> &OrderedMap<K, Option<V>> {
        &self.items
    }

    /// The stored values, mutably, in insertion order — for the bridge's
    /// `clear`, which must release every live napi reference before dropping
    /// them.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Option<V>> {
        self.items.values_mut()
    }

    /// Upstream's `clear`.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// A fresh cursor over the map's *values* — upstream's `values()`, the
    /// only iteration method this module has.
    pub fn cursor(&self) -> MapCursor {
        MapCursor::open()
    }
}

impl<K: Hash + Eq + Clone, V> FuzzyMap<K, V> {
    /// Upstream's `get`, given the *already-hashed* key.
    ///
    /// `None` covers both "no such key" and "the value is `undefined`" —
    /// upstream's caller cannot tell those apart either. See
    /// [`FuzzyMap::has`] for the distinction.
    pub fn get(&self, key: &K) -> Option<&V> {
        self.items.get(key)?.as_ref()
    }

    /// Upstream's `has`, given the already-hashed key. Asks about the key, so
    /// it is `true` for a key stored with `undefined`.
    pub fn has(&self, key: &K) -> bool {
        self.items.contains_key(key)
    }

    /// The write half of both upstream's `add` and `set` — the difference
    /// between them is entirely in *which* value the bridge hashes to get
    /// `key` (the item itself for `add`, the caller's key for `set`), which is
    /// a JavaScript concern and does not appear here.
    ///
    /// Returns the *defined* value it displaced, mirroring
    /// [`crate::structures::default_map::DefaultMap::set`], so the bridge can
    /// release the napi reference a displaced value held.
    pub fn set(&mut self, key: K, value: Option<V>) -> Option<V> {
        self.items.set(key, value).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk<V: Clone>(map: &FuzzyMap<&'static str, V>) -> Vec<Option<V>> {
        let mut cursor = map.cursor();
        let mut out = Vec::new();

        while let Some((_, value)) = cursor.step(map.items()) {
            out.push(value.clone());
        }

        out
    }

    /// A hash applied by the *test*, standing in for the bridge's JS call —
    /// lowercasing, upstream's own example.
    fn lower(text: &str) -> &'static str {
        // Leaked rather than owned: the test fixtures below are all `&str`
        // maps, matching upstream's string-keyed examples without dragging
        // `String` into every assertion.
        Box::leak(text.to_lowercase().into_boxed_str())
    }

    /// 1:1 port of the upstream suite's assertions that do not need the real
    /// hash-function plumbing (that lives in the bridge's tests).
    #[test]
    fn reproduces_the_upstream_suite() {
        // …add (mirrored as set, since core never hashes)…
        let mut map: FuzzyMap<&str, &str> = FuzzyMap::new();
        map.set(lower("Hello"), Some("Hello-item"));
        map.set(lower("World"), Some("World-item"));
        assert_eq!(map.size(), 2);

        // …clear…
        map.clear();
        assert_eq!(map.size(), 0);

        // …get / has…
        let mut got: FuzzyMap<&str, &str> = FuzzyMap::new();
        got.set(lower("Hello"), Some("Hello-item"));
        got.set(lower("World"), Some("World-item"));
        assert_eq!(got.get(&"hello"), Some(&"Hello-item"));
        assert_eq!(got.get(&"shawarama"), None);
        assert!(got.has(&"hello"));
        assert!(!got.has(&"test"));

        // …forEach / values / for…of, all walk the same cursor…
        assert_eq!(walk(&got), vec![Some("Hello-item"), Some("World-item")]);
    }

    #[test]
    fn set_overwrites_in_place_and_reports_the_displaced_value() {
        let mut map: FuzzyMap<&str, u32> = FuzzyMap::new();

        assert_eq!(map.set("a", Some(1)), None);
        assert_eq!(map.set("b", Some(2)), None);
        assert_eq!(
            map.set("a", Some(9)),
            Some(1),
            "overwrite reports the old value"
        );

        assert_eq!(
            map.items().keys().copied().collect::<Vec<_>>(),
            vec!["a", "b"],
            "overwrite must not move the key"
        );
        assert_eq!(map.size(), 2);
    }

    #[test]
    fn has_and_get_disagree_on_a_stored_undefined() {
        let mut map: FuzzyMap<&str, u32> = FuzzyMap::new();
        map.set("a", None);

        assert!(map.has(&"a"));
        assert_eq!(map.get(&"a"), None);
        assert_eq!(
            map.size(),
            1,
            "the key counts even though its value is undefined"
        );
    }

    #[test]
    fn overwriting_a_stored_undefined_reports_no_displaced_value() {
        let mut map: FuzzyMap<&str, u32> = FuzzyMap::new();
        map.set("a", None);

        // The old value was `undefined`, so nothing is reported as displaced —
        // matching `DefaultMap::set`'s `Option<V>::flatten`.
        assert_eq!(map.set("a", Some(1)), None);
        assert_eq!(map.get(&"a"), Some(&1));
    }

    #[test]
    fn there_is_no_delete_only_clear() {
        let mut map: FuzzyMap<&str, u32> = FuzzyMap::new();
        map.set("a", Some(1));
        map.set("b", Some(2));

        map.clear();

        assert_eq!(map.size(), 0);
        assert!(!map.has(&"a"));
        assert_eq!(walk(&map), Vec::<Option<u32>>::new());
    }

    #[test]
    fn values_mut_reaches_every_stored_slot_including_the_undefined_ones() {
        let mut map: FuzzyMap<&str, u32> = FuzzyMap::new();
        map.set("a", Some(1));
        map.set("b", None);

        let seen: Vec<Option<u32>> = map.values_mut().map(|value| *value).collect();

        assert_eq!(seen, vec![Some(1), None]);
    }

    #[test]
    fn a_cursor_sees_entries_set_after_it_was_opened() {
        let mut map: FuzzyMap<&str, u32> = FuzzyMap::new();
        map.set("a", Some(1));

        let mut cursor = map.cursor();
        assert_eq!(cursor.step(map.items()), Some((&"a", &Some(1))));

        map.set("b", Some(2));

        assert_eq!(cursor.step(map.items()), Some((&"b", &Some(2))));
        assert_eq!(cursor.step(map.items()), None);
    }

    #[test]
    fn an_empty_map_reports_nothing() {
        let map: FuzzyMap<&str, u32> = FuzzyMap::new();

        assert_eq!(map.size(), 0);
        assert_eq!(map.get(&"nope"), None);
        assert!(!map.has(&"nope"));
        assert_eq!(walk(&map), Vec::<Option<u32>>::new());
    }
}

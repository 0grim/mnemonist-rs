//! Port of upstream `default-map.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! Python's `defaultdict`: reading a missing key manufactures a value from a
//! factory and stores it. Upstream is 162 lines of delegation to one
//! `new Map()`, which is why it was chosen as the pilot for bridge tier T3 —
//! all of the difficulty is in reproducing `Map` ([`crate::map`]), and none of
//! it is in the structure.
//!
//! Almost none. There is one genuine defect here, and it is in the four lines
//! that are *not* delegation.
//!
//! # `size` is a counter, not a view — and it drifts
//!
//! ```js
//! DefaultMap.prototype.get = function(key) {
//!   var value = this.items.get(key);
//!   if (typeof value === 'undefined') {          // (1) tests the VALUE
//!     value = this.factory(key, this.size);
//!     this.items.set(key, value);
//!     this.size++;                               // (2) not `this.items.size`
//!   }
//!   return value;
//! };
//! ```
//!
//! Line (1) asks whether the stored *value* is `undefined`, not whether the
//! *key* is absent — those differ for any key whose value is `undefined`. Line
//! (2) then increments a counter instead of re-reading `items.size`, which
//! `set` and `delete` both do. Together they make `size` unbounded on a map
//! that holds one entry:
//!
//! ```text
//! m.set('a', undefined);   size 1   items.size 1
//! m.get('a');              size 2   items.size 1   factory called again
//! m.get('a');              size 3   items.size 1   factory called again
//! m.delete('a');           size 0   items.size 0   resynchronised
//! ```
//!
//! Measured against Node 24.18.1; recorded as **B-40** in NOTES.md. It is
//! reproduced here rather than corrected, so [`DefaultMap::size`] is a stored
//! counter and [`DefaultMap::items`]`().len()` is the truth. A port that made
//! `size` return `items.len()` would be *tidier and wrong*, and no upstream
//! assertion would notice.
//!
//! # `undefined` is spelled `None`
//!
//! Reaching that defect at all requires a value type with an `undefined` in
//! it. Core has no JavaScript values, but it does have the exact same idea:
//! this map stores `Option<V>`, and `None` **is** `undefined`. So
//! [`DefaultMap::set`] takes an `Option<V>`, the factory returns one, and the
//! `typeof value === 'undefined'` test is `Option::is_none`.
//!
//! That also gets `peek` right for free. Upstream's `peek` cannot distinguish
//! a missing key from a key whose value is `undefined` — both are `undefined`
//! at the call site — and [`DefaultMap::peek`] flattens the two the same way.
//! [`DefaultMap::has`] is the method that still tells them apart, because
//! upstream's does.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::structures::default_map::DefaultMap;
//!
//! let mut map: DefaultMap<&str, Vec<u32>> = DefaultMap::new();
//!
//! map.get_or_insert_with("one", |_, _| Some(Vec::new()));
//! map.set("two", Some(vec![2]));
//!
//! assert_eq!(map.peek(&"one"), Some(&vec![]));
//! assert_eq!(map.peek(&"two"), Some(&vec![2]));
//! assert_eq!(map.size(), 2);
//!
//! // Reading an unknown key manufactures and stores it.
//! map.get_or_insert_with("unknown", |_, _| Some(Vec::new()));
//! assert_eq!(map.size(), 3);
//! ```

use std::hash::Hash;

use crate::map::{MapCursor, OrderedMap};

/// Upstream's `DefaultMap`.
///
/// `V` is the *defined* value type; the map stores `Option<V>` and `None` is
/// JavaScript's `undefined`. See the module docs.
///
/// The factory is **not** stored. Upstream keeps it on the instance and
/// validates it in the constructor; here it is supplied per call to
/// [`DefaultMap::get_or_insert_with`]. Two reasons: the constructor's
/// `typeof factory !== 'function'` check is a JavaScript type test that
/// belongs at the boundary, and a stored `F` would put a JS callback inside a
/// crate that must not know JavaScript exists. The bridge holds the callback.
#[derive(Debug, Clone)]
pub struct DefaultMap<K, V> {
    items: OrderedMap<K, Option<V>>,
    size: usize,
}

impl<K, V> Default for DefaultMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> DefaultMap<K, V> {
    pub fn new() -> Self {
        Self {
            items: OrderedMap::new(),
            size: 0,
        }
    }

    /// Upstream's `size` **property**.
    ///
    /// A stored counter, and deliberately not `items().len()`. See the module
    /// docs and B-40: the two disagree, permanently, once a value of
    /// `undefined` has been read back through
    /// [`get_or_insert_with`](DefaultMap::get_or_insert_with).
    pub fn size(&self) -> usize {
        self.size
    }

    /// The backing map — upstream's `items`, which is a public property and
    /// what its `inspect()` returns.
    ///
    /// Exposed because the differential fuzzer compares it entry by entry:
    /// agreeing on `size` while disagreeing on the entries is exactly the
    /// drift B-40 describes, and only a direct comparison catches it.
    pub fn items(&self) -> &OrderedMap<K, Option<V>> {
        &self.items
    }

    /// The stored values, mutably, in insertion order.
    ///
    /// Exists for the bridge: releasing the napi reference a value holds takes
    /// `&mut`, and [`clear`](DefaultMap::clear) has to do it for every live
    /// entry before dropping them.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Option<V>> {
        self.items.values_mut()
    }

    /// Upstream's `clear`. Resets **both** counters, which is why `clear` is
    /// the one operation that reliably repairs a drifted `size`.
    pub fn clear(&mut self) {
        self.items.clear();
        self.size = 0;
    }

    /// A fresh cursor over the entries — upstream's `entries`, `keys` and
    /// `values`, which all return an iterator over the same backing `Map`.
    ///
    /// The three differ only in what the caller projects out of each step, so
    /// they are one method here and three in the bridge.
    pub fn cursor(&self) -> MapCursor {
        MapCursor::open()
    }
}

impl<K: Hash + Eq + Clone, V> DefaultMap<K, V> {
    /// Upstream's `peek`: `items.get(key)`, with no factory call and no
    /// counter change.
    ///
    /// `None` covers both "no such key" and "the value is `undefined`",
    /// because upstream's caller cannot tell those apart either. Use
    /// [`has`](DefaultMap::has) for the distinction.
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.items.get(key)?.as_ref()
    }

    /// Upstream's `has`: `items.has(key)`.
    ///
    /// Asks about the **key**, while
    /// [`get_or_insert_with`](DefaultMap::get_or_insert_with) asks about the
    /// value. So `has` is `true` for a key stored with `undefined`, even
    /// though reading that key will run the factory again.
    pub fn has(&self, key: &K) -> bool {
        self.items.contains_key(key)
    }

    /// Upstream's `set`.
    ///
    /// Returns the *defined* value it displaced, so the bridge can release the
    /// napi reference that value held; `None` means there was nothing to
    /// release, whether because the key was new or because it held
    /// `undefined`. Resynchronises `size` from the backing map, which is what
    /// makes B-40 self-healing on any write.
    pub fn set(&mut self, key: K, value: Option<V>) -> Option<V> {
        let displaced = self.items.set(key, value);
        self.size = self.items.len();

        displaced.flatten()
    }

    /// Upstream's `delete`, which returns a boolean.
    ///
    /// `None` is that `false` — no such key. `Some(value)` is the `true`, and
    /// carries the removed value so the bridge can release it; the inner
    /// `None` is a stored `undefined`. Also resynchronises `size`.
    pub fn delete(&mut self, key: &K) -> Option<Option<V>> {
        let removed = self.items.delete(key);
        self.size = self.items.len();

        removed
    }
}

impl<K: Hash + Eq + Clone, V> DefaultMap<K, V> {
    /// Upstream's `get`.
    ///
    /// Named for what it does rather than for what upstream calls it: this is
    /// a mutating read that can call `factory`, store its result and bump
    /// `size`. `factory` receives the key and **`self.size` as it is now**,
    /// which is the drifted counter rather than the entry count — upstream
    /// passes `this.size`, and `DefaultMap.autoIncrement()` is the documented
    /// use of that second argument even though it ignores both.
    ///
    /// The returned `Option<&V>` is upstream's return value: `None` is
    /// `undefined`, which a factory is free to produce.
    ///
    /// The factory runs *before* the insert, exactly as upstream's does, so a
    /// factory that panics leaves the map untouched — `size` included. For a
    /// factory that can fail without unwinding, see
    /// [`try_get_or_insert_with`](DefaultMap::try_get_or_insert_with).
    pub fn get_or_insert_with<F>(&mut self, key: K, factory: F) -> Option<&V>
    where
        F: FnOnce(&K, usize) -> Option<V>,
    {
        match self.try_get_or_insert_with(key, |key, size| {
            Ok::<Option<V>, std::convert::Infallible>(factory(key, size))
        }) {
            Ok(value) => value,
            Err(never) => match never {},
        }
    }

    /// The **write half** of upstream's `get`, for a caller that has already
    /// run the factory itself.
    ///
    /// ```js
    /// this.items.set(key, value);
    /// this.size++;
    /// ```
    ///
    /// Exists for the bridge, and the reason is specific: there the factory is
    /// a JavaScript function, and the bridge holds the map in a `RefCell`
    /// (B-31), so it cannot run the factory from *inside*
    /// [`try_get_or_insert_with`](DefaultMap::try_get_or_insert_with) without
    /// keeping a borrow alive across a call that may re-enter. Upstream does
    /// not hold its map either — its factory runs between the read and the
    /// write — so splitting the two halves is closer to upstream, not further
    /// from it.
    ///
    /// Unconditional on purpose. Upstream does not re-check the key after the
    /// factory returns, so a factory that inserted the same key gets
    /// overwritten and `size` is incremented a second time. `size += 1` rather
    /// than a resynchronisation is B-40, and it is deliberate; see
    /// [`set`](DefaultMap::set) for the write that heals it.
    pub fn insert_from_factory(&mut self, key: K, value: Option<V>) -> Option<&V> {
        self.items.set(key.clone(), value);
        self.size += 1;

        self.items
            .get(&key)
            .expect("the value was just inserted under this key")
            .as_ref()
    }

    /// [`get_or_insert_with`](DefaultMap::get_or_insert_with) with a factory
    /// that can fail.
    ///
    /// Exists for the bridge, where the factory is a JavaScript function and
    /// "it threw" is an ordinary outcome rather than a panic. On `Err` the map
    /// is **left exactly as it was** — no entry, no `size` increment — which is
    /// what upstream does, because its `this.items.set` and `this.size++` are
    /// both after the call that threw.
    pub fn try_get_or_insert_with<F, E>(&mut self, key: K, factory: F) -> Result<Option<&V>, E>
    where
        F: FnOnce(&K, usize) -> Result<Option<V>, E>,
    {
        // Resolved to a slot first so the borrow ends before the factory runs.
        let defined = self.items.slot_of(&key).filter(
            |&slot| matches!(self.items.entry_at(slot), Some((_, value)) if value.is_some()),
        );

        if let Some(slot) = defined {
            let (_, value) = self
                .items
                .entry_at(slot)
                .expect("the slot was just confirmed live");

            return Ok(value.as_ref());
        }

        let value = factory(&key, self.size)?;

        // `set`, not a raw append: a stored `undefined` under this key keeps
        // its position and is overwritten, which is what
        // `this.items.set(key, value)` does. `size` is then *incremented*
        // rather than resynchronised -- that asymmetry with `DefaultMap::set`
        // is the whole of B-40, and it is deliberate.
        self.items.set(key.clone(), value);
        self.size += 1;

        Ok(self
            .items
            .get(&key)
            .expect("the value was just inserted under this key")
            .as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The upstream test file's `FACTORY`: a fresh empty list, ignoring both
    /// arguments.
    fn lists() -> DefaultMap<&'static str, Vec<u32>> {
        DefaultMap::new()
    }

    /// `map.get(key).push(n)` — the idiom the whole upstream test file is
    /// built on. In JS the array comes back by reference; in core the caller
    /// reaches back through the map, which is the same thing without the
    /// aliasing.
    fn push(map: &mut DefaultMap<&'static str, Vec<u32>>, key: &'static str, value: u32) {
        map.get_or_insert_with(key, |_, _| Some(Vec::new()));
        map.items_mut_for_test(key).push(value);
    }

    impl DefaultMap<&'static str, Vec<u32>> {
        fn items_mut_for_test(&mut self, key: &'static str) -> &mut Vec<u32> {
            self.items
                .get_mut(&key)
                .expect("the key was just materialised")
                .as_mut()
                .expect("the factory returned a defined value")
        }
    }

    fn walk<K: Clone, V: Clone>(map: &DefaultMap<K, V>) -> Vec<(K, Option<V>)> {
        let mut cursor = map.cursor();
        let mut out = Vec::new();

        while let Some((key, value)) = cursor.step(map.items()) {
            out.push((key.clone(), value.clone()));
        }

        out
    }

    /// 1:1 port of the seven upstream `it` blocks, as a baseline.
    #[test]
    fn reproduces_the_upstream_suite() {
        // …set & get…
        let mut map = lists();

        push(&mut map, "one", 1);
        map.set("two", Some(vec![2]));

        assert_eq!(map.peek(&"one"), Some(&vec![1]));
        assert_eq!(map.peek(&"two"), Some(&vec![2]));
        assert_eq!(map.size(), 2);

        assert_eq!(
            map.get_or_insert_with("unknown", |_, _| Some(Vec::new())),
            Some(&vec![])
        );
        assert_eq!(map.size(), 3);

        map.clear();
        assert_eq!(map.size(), 0);
        assert_eq!(
            map.get_or_insert_with("one", |_, _| Some(Vec::new())),
            Some(&vec![])
        );

        // …delete…
        let mut deletes: DefaultMap<&str, u32> = DefaultMap::new();
        deletes.set("one", Some(1));
        assert!(deletes.has(&"one"));
        assert_eq!(deletes.delete(&"one"), Some(Some(1)));
        assert_eq!(deletes.size(), 0);
        assert!(!deletes.has(&"one"));
        assert_eq!(deletes.delete(&"one"), None);

        // …forEach / iterators…
        let mut iterated = lists();
        push(&mut iterated, "one", 1);
        push(&mut iterated, "two", 2);
        assert_eq!(
            walk(&iterated),
            vec![("one", Some(vec![1])), ("two", Some(vec![2]))]
        );

        // …autoIncrement…
        let mut counter = 0u32;
        let mut auto: DefaultMap<&str, u32> = DefaultMap::new();
        let mut auto_increment = |map: &mut DefaultMap<&'static str, u32>, key| {
            map.get_or_insert_with(key, |_, _| {
                let next = counter;
                counter += 1;
                Some(next)
            })
            .copied()
        };
        assert_eq!(auto_increment(&mut auto, "test"), Some(0));
        assert_eq!(auto_increment(&mut auto, "test2"), Some(1));
        assert_eq!(auto.size(), 2);

        // …peek…
        let mut peeked = lists();
        push(&mut peeked, "one", 1);
        assert_eq!(peeked.peek(&"one"), Some(&vec![1]));
        assert_eq!(peeked.peek(&"two"), None);
        assert_eq!(peeked.size(), 1);
        assert!(!peeked.has(&"two"));
    }

    #[test]
    fn the_factory_receives_the_key_and_the_current_size() {
        let mut map: DefaultMap<&str, String> = DefaultMap::new();

        assert_eq!(
            map.get_or_insert_with("a", |key, size| Some(format!("{key}@{size}"))),
            Some(&String::from("a@0"))
        );
        assert_eq!(
            map.get_or_insert_with("b", |key, size| Some(format!("{key}@{size}"))),
            Some(&String::from("b@1"))
        );
    }

    /// B-40, the whole chain. Nothing in the upstream suite reaches this.
    #[test]
    fn size_drifts_when_a_stored_value_is_undefined() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();
        let mut calls = 0;

        map.set("a", None);
        assert_eq!((map.size(), map.items().len(), calls), (1, 1, 0));

        for expected_size in 2..=4 {
            map.get_or_insert_with("a", |_, _| {
                calls += 1;
                None
            });
            assert_eq!(map.size(), expected_size);
            assert_eq!(map.items().len(), 1, "one entry throughout");
        }

        assert_eq!(calls, 3, "the factory re-ran on every read");
        assert!(
            map.has(&"a"),
            "`has` asks about the key, and the key is there"
        );
        assert_eq!(map.peek(&"a"), None);
    }

    /// The other half of B-40: any write resynchronises `size`, so the drift
    /// is silent rather than permanent.
    #[test]
    fn a_write_resynchronises_a_drifted_size() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();

        map.set("a", None);
        map.get_or_insert_with("a", |_, _| None);
        map.get_or_insert_with("a", |_, _| None);
        assert_eq!(map.size(), 3);

        map.set("b", Some(1));
        assert_eq!(map.size(), 2, "`set` reads items.size");

        map.get_or_insert_with("a", |_, _| None);
        assert_eq!(map.size(), 3);

        map.delete(&"b");
        assert_eq!(map.size(), 1, "`delete` reads items.size");
    }

    /// A stored `undefined` keeps its slot when the factory re-runs, so the
    /// drift is invisible to iteration order as well as to `items.size`.
    #[test]
    fn a_refilled_undefined_keeps_its_insertion_position() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();

        map.set("a", None);
        map.set("b", Some(2));
        map.get_or_insert_with("a", |_, _| Some(9));

        assert_eq!(walk(&map), vec![("a", Some(9)), ("b", Some(2))]);
        assert_eq!(map.size(), 3, "still drifted");
    }

    /// Once the factory returns something defined, the re-running stops.
    #[test]
    fn a_defined_value_written_by_the_factory_ends_the_drift() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();
        let mut calls = 0;

        map.set("a", None);
        map.get_or_insert_with("a", |_, _| {
            calls += 1;
            Some(7)
        });
        map.get_or_insert_with("a", |_, _| {
            calls += 1;
            Some(8)
        });

        assert_eq!(calls, 1);
        assert_eq!(map.peek(&"a"), Some(&7));
    }

    #[test]
    fn set_reports_the_defined_value_it_displaced() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();

        assert_eq!(map.set("a", Some(1)), None);
        assert_eq!(map.set("a", Some(2)), Some(1));
        assert_eq!(map.set("a", None), Some(2));
        assert_eq!(map.set("a", Some(3)), None, "undefined displaced nothing");
        assert_eq!(map.size(), 1);
    }

    #[test]
    fn delete_distinguishes_a_missing_key_from_a_stored_undefined() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();
        map.set("a", None);

        assert_eq!(map.delete(&"a"), Some(None), "removed, value undefined");
        assert_eq!(map.delete(&"a"), None, "upstream would return false");
    }

    #[test]
    fn a_deleted_key_is_reinserted_at_the_end() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();

        map.set("a", Some(1));
        map.set("b", Some(2));
        map.set("c", Some(3));
        map.delete(&"a");
        map.get_or_insert_with("a", |_, _| Some(9));

        assert_eq!(
            walk(&map),
            vec![("b", Some(2)), ("c", Some(3)), ("a", Some(9))]
        );
        assert_eq!(map.size(), 3);
    }

    #[test]
    fn a_cursor_sees_entries_the_factory_creates_after_it_was_opened() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();
        map.set("a", Some(1));

        let mut cursor = map.cursor();
        assert_eq!(cursor.step(map.items()), Some((&"a", &Some(1))));

        map.get_or_insert_with("b", |_, _| Some(2));

        assert_eq!(cursor.step(map.items()), Some((&"b", &Some(2))));
        assert_eq!(cursor.step(map.items()), None);
    }

    #[test]
    fn clear_repairs_a_drifted_size_and_empties_the_map() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();

        map.set("a", None);
        map.get_or_insert_with("a", |_, _| None);
        assert_eq!(map.size(), 2);

        map.clear();

        assert_eq!(map.size(), 0);
        assert_eq!(map.items().len(), 0);
        assert!(!map.has(&"a"));
        assert_eq!(walk(&map), vec![]);
    }

    #[test]
    fn an_empty_map_reports_nothing() {
        let map: DefaultMap<&str, u32> = DefaultMap::new();

        assert_eq!(map.size(), 0);
        assert_eq!(map.peek(&"nope"), None);
        assert!(!map.has(&"nope"));
        assert_eq!(walk(&map), vec![]);
    }

    /// `has` and `peek` disagree on purpose: one asks about the key, the other
    /// cannot see past the value.
    #[test]
    fn has_and_peek_disagree_on_a_stored_undefined() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();
        map.set("a", None);

        assert!(map.has(&"a"));
        assert_eq!(map.peek(&"a"), None);
        assert_eq!(map.get_or_insert_with("a", |_, _| Some(1)), Some(&1));
    }

    /// A throwing factory must leave nothing behind — not the entry, and not
    /// the `size` increment. Upstream's `set` and `size++` are both after the
    /// call that threw.
    #[test]
    fn a_failing_factory_leaves_the_map_untouched() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();
        map.set("a", Some(1));

        let outcome: Result<Option<&u32>, &str> =
            map.try_get_or_insert_with("b", |_, _| Err("boom"));

        assert_eq!(outcome, Err("boom"));
        assert_eq!(map.size(), 1);
        assert!(!map.has(&"b"));
        assert_eq!(walk(&map), vec![("a", Some(1))]);
    }

    /// …including when the key exists but holds `undefined`, which is the
    /// path that would otherwise re-run the factory.
    #[test]
    fn a_failing_factory_leaves_a_stored_undefined_untouched() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();
        map.set("a", None);

        let outcome: Result<Option<&u32>, &str> =
            map.try_get_or_insert_with("a", |_, _| Err("boom"));

        assert_eq!(outcome, Err("boom"));
        assert_eq!(map.size(), 1, "no drift from a call that threw");
        assert!(map.has(&"a"));
    }

    #[test]
    fn values_mut_reaches_every_stored_slot_including_the_undefined_ones() {
        let mut map: DefaultMap<&str, u32> = DefaultMap::new();
        map.set("a", Some(1));
        map.set("b", None);

        let seen: Vec<Option<u32>> = map.values_mut().map(|value| *value).collect();

        assert_eq!(seen, vec![Some(1), None]);
    }
}

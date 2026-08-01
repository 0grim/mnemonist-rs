//! Port of upstream `multi-map.js` (mnemonist v0.40.4).
//!
//! A `MultiMap` is a `Map` whose values are *containers*: `set(key, value)`
//! appends `value` to whatever container lives at `key`, creating one on
//! first use. Upstream lets the container class be anything with a `.push`
//! method (`Array`, `Vector`, a caller's own class) or exactly `Set`, decided
//! once at construction and checked thereafter with `this.Container === Set`
//! — a single object-identity branch that governs every write.
//!
//! # Why `V` needs no `Hash`/`Eq` bound here
//!
//! `Set`-kind membership ("was this value already in the bucket?") is the one
//! operation that would normally want `V: Hash + Eq`. This module asks for it
//! a different way: [`MultiMap::set_with`]/[`MultiMap::remove_with`] take the
//! equivalence relation as a fallible callback, `Fn(&V, &V) -> Result<bool,
//! E>`, and scan the bucket linearly. [`MultiMap::set`]/[`MultiMap::remove`]
//! are the convenience wrappers for a `V: PartialEq` that can never fail.
//!
//! This is not a performance choice — a real hash-based `Set` would be
//! faster — it is a *correctness* one. `fuzzy-multi-map`'s own test stores
//! plain JavaScript **objects** as values (`{title: 'Hello'}`), and a
//! `Set`'s membership test on an object is JavaScript's SameValueZero, which
//! for an object means identity: "is this the very same object", not "does
//! it look the same". No hash for an arbitrary JS object is reachable from
//! Rust (`crate::structures::set`'s own bridge refuses one for exactly this
//! reason), and identity comparison for two already-retained references
//! needs `napi_strict_equals`, which needs an `Env`. A pure-Rust `Hash + Eq`
//! bound could not express that at all, so the equality test is a parameter
//! instead — the same move `crate::utils::comparators::Comparator` makes for
//! a JavaScript comparator callback, applied to a JavaScript equality
//! callback.
//!
//! `mnemonist_napi::multi_map` (values are always `crate::structures::set`-
//! like primitives) uses the infallible convenience methods; `fuzzy_multi_map`'s
//! bridge (values are arbitrary retained JS values) uses the fallible ones
//! with a SameValueZero check that calls back into the engine only for
//! non-primitive values.
//!
//! # `dimension` is never stored
//!
//! Upstream keeps `this.dimension` as a counter, incremented/decremented
//! exactly when a key bucket is created/removed. Every one of those
//! transitions here already goes through [`crate::map::OrderedMap`]'s own
//! `set`/`delete`, so `dimension()` reads `items.len()` directly rather than
//! keeping a second counter that could drift from it — the same
//! simplification `crate::structures::fuzzy_map` makes for its `size`.
//!
//! # `remove` is one function, not two
//!
//! Upstream's `remove` branches on `this.Container === Set` for a `Set`'s
//! `delete(value)` versus an `Array`'s `indexOf` + `splice(index, 1)`. Both
//! remove the *first* bucket entry equal to `value` and both decrement
//! `size` by exactly one on success (verified against Node 24.18.1 — the
//! `Set` branch's `this.size--` is gated on `wasDeleted`, the `Array`
//! branch's is not, but a live bucket is never empty by construction, so the
//! branches agree on every reachable input). [`MultiMap::remove_with`] is
//! that single behaviour, since a linear scan-and-remove is exactly what
//! both amount to once membership is a supplied equality rather than a
//! `Hash` lookup.
//!
//! # The flattened cursor snapshots each bucket
//!
//! `values()`/`entries()`/`forEach` walk every key's bucket in turn. Upstream
//! obtains a *live* inner iterator per key (`container.values()` for a
//! `Set`, an index against `container.length` captured once for an `Array`),
//! so a mutation to the *very bucket currently being walked* would, in
//! principle, be visible to a `Set` bucket's own live iterator and invisible
//! (for appends past the captured length) to an `Array` bucket's.
//! [`FlattenedCursor`] instead clones the bucket's contents once, when the
//! outer step reaches that key, and walks the clone. This is a stated
//! simplification: it reproduces every case in `test/multi-map.js` (none of
//! which mutates a bucket from inside a walk over it) and it reproduces the
//! *outer* liveness exactly (a key deleted ahead of the cursor is skipped,
//! and a key deleted mid-inner-walk no longer stops the walk early, both via
//! the ordinary [`crate::map::MapCursor`] machinery), but it does not
//! reproduce a same-bucket mutation mid-inner-walk. See
//! `docs/modules/multi-map.md`.

use std::hash::Hash;

use crate::map::{MapCursor, OrderedMap};

/// Which membership rule a bucket obeys — decided once, at construction, and
/// fixed for the map's lifetime (upstream's `this.Container`, checked by
/// reference equality against the global `Set`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    /// Every `set` appends; a value may repeat. Upstream's default (`Array`)
    /// and anything else that is not exactly `Set` — including a custom
    /// `Vector` subclass. See `docs/modules/multi-map.md` for what "anything
    /// else" is simplified to at the bridge.
    List,
    /// `set` appends only when the value is not already present, matching
    /// `Set.prototype.add`.
    Set,
}

/// The container stored at one key: upstream's `Array` or `Set` instance.
///
/// Represented as a plain, insertion-ordered `Vec` regardless of `kind` — see
/// the module docs for why `Set`-kind membership is a linear scan against a
/// supplied equality rather than a hash lookup.
#[derive(Debug, Clone)]
pub struct Bucket<V> {
    kind: ContainerKind,
    values: Vec<V>,
}

impl<V> Bucket<V> {
    fn new(kind: ContainerKind) -> Self {
        Self {
            kind,
            values: Vec::new(),
        }
    }

    pub fn kind(&self) -> ContainerKind {
        self.kind
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The bucket's contents in insertion order — upstream's `Array`/`Set`
    /// instance, minus its identity.
    pub fn values(&self) -> &[V] {
        &self.values
    }
}

/// Upstream's `MultiMap`.
#[derive(Debug, Clone)]
pub struct MultiMap<K, V> {
    items: OrderedMap<K, Bucket<V>>,
    kind: ContainerKind,
    /// Upstream's `size`: total values across every bucket. **Not**
    /// `items.len()`, which is [`MultiMap::dimension`] — the distinct-key
    /// count.
    size: usize,
}

impl<K, V> MultiMap<K, V> {
    /// `new MultiMap(Container)`, already resolved to a [`ContainerKind`] —
    /// the JS-identity check against the global `Set` is the bridge's job.
    pub fn new(kind: ContainerKind) -> Self {
        Self {
            items: OrderedMap::new(),
            kind,
            size: 0,
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Distinct keys — upstream's `dimension`, read from the map rather than
    /// tracked separately. See the module docs.
    pub fn dimension(&self) -> usize {
        self.items.len()
    }

    /// `#.clear`.
    pub fn clear(&mut self) {
        self.items.clear();
        self.size = 0;
    }

    /// The backing map, for the bridge's cursors and the differential fuzzer.
    pub fn items(&self) -> &OrderedMap<K, Bucket<V>> {
        &self.items
    }

    /// Every stored value, mutably, bucket after bucket — for a bridge whose
    /// `V` owns a napi reference and must release it (`fuzzy_multi_map`'s
    /// `clear`/finalizer), the same role `OrderedMap::values_mut` plays for
    /// `default_map`/`fuzzy_map`.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.items
            .values_mut()
            .flat_map(|bucket| bucket.values.iter_mut())
    }
}

impl<K: Hash + Eq + Clone, V: Clone> MultiMap<K, V> {
    pub fn has(&self, key: &K) -> bool {
        self.items.contains_key(key)
    }

    /// `#.get` — the whole bucket, or `None` for a key never set.
    pub fn get(&self, key: &K) -> Option<&Bucket<V>> {
        self.items.get(key)
    }

    /// `#.multiplicity` / `#.count` — `0` for a missing key, never an error.
    pub fn multiplicity(&self, key: &K) -> usize {
        self.items.get(key).map_or(0, Bucket::len)
    }

    /// `#.delete` — drops the whole bucket, returning whether one existed.
    pub fn delete(&mut self, key: &K) -> bool {
        match self.items.delete(key) {
            Some(bucket) => {
                self.size -= bucket.len();
                true
            }
            None => false,
        }
    }

    /// `#.set` for a `V` with its own notion of equality — the path
    /// `mnemonist_napi::multi_map` uses, since its values are
    /// `crate::js_key`-shaped and comparing them cannot fail.
    pub fn set(&mut self, key: K, value: V)
    where
        V: PartialEq,
    {
        // `Infallible` witnesses that the closure below never returns `Err`.
        let result: Result<Option<V>, std::convert::Infallible> =
            self.set_with(key, value, |a, b| Ok(a == b));

        result.expect("a `PartialEq` comparison cannot fail");
    }

    /// The general form of `#.set`: `eq(candidate, existing)` decides
    /// `Set`-kind membership and may fail. Never called at all for a
    /// `List`-kind bucket, which always appends. See the module docs.
    ///
    /// Returns the candidate **back** when a `Set`-kind bucket already had an
    /// equal member and therefore never stored it — upstream's `Set.add`
    /// silently drops such a duplicate, which is fine for a plain value but
    /// not for a bridge value that owns a resource (a retained JS reference):
    /// `mnemonist_napi::fuzzy_multi_map` uses this to release exactly the
    /// duplicates it did not end up storing, rather than leaking them. `Ok(None)`
    /// means the value was stored (appended, or added as a genuinely new
    /// `Set` member).
    pub fn set_with<E>(
        &mut self,
        key: K,
        value: V,
        eq: impl Fn(&V, &V) -> Result<bool, E>,
    ) -> Result<Option<V>, E> {
        if !self.items.contains_key(&key) {
            self.items.set(key.clone(), Bucket::new(self.kind));
        }

        let bucket = self
            .items
            .get_mut(&key)
            .expect("just ensured the key is present");

        match bucket.kind {
            ContainerKind::List => {
                bucket.values.push(value);
                self.size += 1;

                Ok(None)
            }
            ContainerKind::Set => {
                let mut present = false;

                for existing in &bucket.values {
                    if eq(&value, existing)? {
                        present = true;
                        break;
                    }
                }

                if present {
                    return Ok(Some(value));
                }

                bucket.values.push(value);
                self.size += 1;

                Ok(None)
            }
        }
    }

    /// `#.remove` for a `V` with its own notion of equality.
    pub fn remove(&mut self, key: K, value: &V) -> bool
    where
        V: PartialEq,
    {
        let result: Result<bool, std::convert::Infallible> =
            self.remove_with(key, value, |a, b| Ok(a == b));

        result.expect("a `PartialEq` comparison cannot fail")
    }

    /// The general form of `#.remove`: removes the first bucket entry `eq`
    /// reports equal to `value`. See the module docs for why this single
    /// function is both of upstream's `Set`/`Array` branches.
    pub fn remove_with<E>(
        &mut self,
        key: K,
        value: &V,
        eq: impl Fn(&V, &V) -> Result<bool, E>,
    ) -> Result<bool, E> {
        let Some(bucket) = self.items.get_mut(&key) else {
            return Ok(false);
        };

        let mut found = None;

        for (index, existing) in bucket.values.iter().enumerate() {
            if eq(value, existing)? {
                found = Some(index);
                break;
            }
        }

        let Some(index) = found else {
            return Ok(false);
        };

        bucket.values.remove(index);
        self.size -= 1;

        if bucket.values.is_empty() {
            self.items.delete(&key);
        }

        Ok(true)
    }

    /// A fresh flattened `(key, value)` cursor — upstream's `values()`/
    /// `entries()`, and the walk `forEach` drives eagerly. See the module
    /// docs for what "flattened" simplifies.
    pub fn cursor(&self) -> FlattenedCursor<K, V> {
        FlattenedCursor::open()
    }
}

/// A live cursor over every `(key, value)` pair a [`MultiMap`] holds, in
/// upstream's `values()`/`entries()`/`forEach` order: every bucket's values,
/// bucket after bucket, in key insertion order.
///
/// See the module docs for the one simplification this makes (a bucket's
/// contents are snapshotted when the outer step reaches it, rather than
/// re-read live on every inner step).
#[derive(Debug, Clone)]
pub struct FlattenedCursor<K, V> {
    outer: MapCursor,
    current_key: Option<K>,
    pending: Vec<V>,
    position: usize,
}

impl<K, V> Default for FlattenedCursor<K, V> {
    fn default() -> Self {
        Self::open()
    }
}

impl<K, V> FlattenedCursor<K, V> {
    pub fn open() -> Self {
        Self {
            outer: MapCursor::open(),
            current_key: None,
            pending: Vec::new(),
            position: 0,
        }
    }
}

impl<K: Hash + Eq + Clone, V: Clone> FlattenedCursor<K, V> {
    /// One step. `None` is permanent, exactly like [`MapCursor::step`].
    pub fn step(&mut self, items: &OrderedMap<K, Bucket<V>>) -> Option<(K, V)> {
        loop {
            if self.position < self.pending.len() {
                let value = self.pending[self.position].clone();
                self.position += 1;

                return Some((
                    self.current_key
                        .clone()
                        .expect("pending is only ever filled alongside current_key"),
                    value,
                ));
            }

            let (key, bucket) = self.outer.step(items)?;

            self.current_key = Some(key.clone());
            self.pending = bucket.values.clone();
            self.position = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(map: &MultiMap<&'static str, &'static str>) -> Vec<(&'static str, &'static str)> {
        let mut cursor = map.cursor();
        let mut out = Vec::new();

        while let Some(pair) = cursor.step(map.items()) {
            out.push(pair);
        }

        out
    }

    #[test]
    fn reproduces_the_upstream_walkthrough() {
        let mut map: MultiMap<&str, &str> = MultiMap::new(ContainerKind::List);

        map.set("one", "hello");
        map.set("one", "world");

        assert_eq!(map.size(), 2);
        assert_eq!(map.dimension(), 1);
        assert!(map.has(&"one"));
        assert!(!map.has(&"two"));
        assert_eq!(map.multiplicity(&"one"), 2);
        assert_eq!(map.multiplicity(&"three"), 0);

        assert_eq!(
            map.get(&"one").unwrap().values(),
            &["hello", "world"] as &[&str]
        );

        assert_eq!(drain(&map), vec![("one", "hello"), ("one", "world")]);
    }

    #[test]
    fn set_kind_deduplicates_by_the_supplied_equality() {
        let mut map: MultiMap<&str, i32> = MultiMap::new(ContainerKind::Set);

        map.set("one", 1);
        map.set("one", 1);
        map.set("two", 9);

        assert_eq!(map.multiplicity(&"one"), 1);
        assert_eq!(map.multiplicity(&"two"), 1);
        assert_eq!(map.size(), 2);
    }

    #[test]
    fn set_with_hands_a_rejected_duplicate_back_instead_of_dropping_it() {
        // A resource-owning `V` (a retained JS reference, in the bridge)
        // must not be silently dropped when `Set`-kind membership rejects
        // it as a duplicate -- `mnemonist_napi::fuzzy_multi_map` relies on
        // getting it back to release it. `String` stands in for "a type
        // whose drop would be observable" well enough for this test's
        // purpose, which is only to check the *value*, not any drop glue.
        let mut map: MultiMap<&str, String> = MultiMap::new(ContainerKind::Set);

        let outcome: Result<Option<String>, std::convert::Infallible> =
            map.set_with("one", "a".to_owned(), |a, b| Ok(a == b));
        assert_eq!(outcome.unwrap(), None, "a genuinely new member is stored");

        let outcome: Result<Option<String>, std::convert::Infallible> =
            map.set_with("one", "a".to_owned(), |a, b| Ok(a == b));
        assert_eq!(
            outcome.unwrap(),
            Some("a".to_owned()),
            "a duplicate is handed back, not dropped"
        );

        assert_eq!(map.multiplicity(&"one"), 1);
    }

    #[test]
    fn list_kind_never_deduplicates() {
        let mut map: MultiMap<&str, i32> = MultiMap::new(ContainerKind::List);

        map.set("one", 1);
        map.set("one", 1);

        assert_eq!(map.multiplicity(&"one"), 2);
        assert_eq!(map.size(), 2);
    }

    #[test]
    fn remove_matches_upstream_size_and_deletion_bookkeeping() {
        let mut map: MultiMap<&str, i32> = MultiMap::new(ContainerKind::List);

        map.set("one", 1);
        map.set("one", 2);
        map.set("one", 1);

        assert!(map.remove("one", &1));
        assert_eq!(map.get(&"one").unwrap().values(), &[2, 1]);
        assert_eq!(map.size(), 2);
        assert_eq!(map.dimension(), 1);

        assert!(map.remove("one", &1));
        assert_eq!(map.get(&"one").unwrap().values(), &[2]);
        assert_eq!(map.size(), 1);

        // A third removal of a value no longer present is a no-op.
        assert!(!map.remove("one", &1));
        assert_eq!(map.size(), 1);
        assert_eq!(map.dimension(), 1);

        assert!(map.remove("one", &2));
        assert!(map.get(&"one").is_none());
        assert_eq!(map.size(), 0);
        assert_eq!(map.dimension(), 0);
    }

    #[test]
    fn remove_on_a_set_kind_bucket_drops_the_key_once_it_empties() {
        let mut map: MultiMap<&str, i32> = MultiMap::new(ContainerKind::Set);

        map.set("one", 1);
        map.set("one", 2);

        assert!(map.remove("one", &1));
        assert_eq!(map.dimension(), 1);
        assert!(map.remove("one", &2));
        assert_eq!(map.dimension(), 0);
        assert!(!map.has(&"one"));
    }

    #[test]
    fn delete_removes_the_whole_bucket() {
        let mut map: MultiMap<&str, &str> = MultiMap::new(ContainerKind::List);

        map.set("one", "hello");
        map.set("one", "world");
        map.set("two", "hello");

        assert!(map.delete(&"one"));
        assert_eq!(map.size(), 1);
        assert_eq!(map.dimension(), 1);
        assert!(!map.has(&"one"));
        assert!(
            !map.delete(&"one"),
            "deleting twice is a no-op, not a panic"
        );
    }

    #[test]
    fn clear_resets_size_and_dimension() {
        let mut map: MultiMap<&str, &str> = MultiMap::new(ContainerKind::List);

        map.set("one", "hello");
        map.set("one", "world");
        map.clear();

        assert_eq!(map.size(), 0);
        assert_eq!(map.dimension(), 0);
        assert!(!map.has(&"one"));
    }

    #[test]
    fn a_key_deleted_ahead_of_a_live_cursor_is_skipped() {
        let mut map: MultiMap<&str, &str> = MultiMap::new(ContainerKind::List);

        map.set("one", "a");
        map.set("two", "b");
        map.set("three", "c");

        let mut cursor = map.cursor();
        assert_eq!(cursor.step(map.items()), Some(("one", "a")));

        map.delete(&"two");

        assert_eq!(cursor.step(map.items()), Some(("three", "c")));
        assert_eq!(cursor.step(map.items()), None);
    }

    #[test]
    fn fallible_equality_short_circuits_on_the_first_error() {
        let mut map: MultiMap<&str, i32> = MultiMap::new(ContainerKind::Set);

        map.set("one", 1);

        let outcome: Result<Option<i32>, &'static str> =
            map.set_with("one", 2, |_, _| Err("comparator refused"));

        assert_eq!(outcome, Err("comparator refused"));
        // The failed comparison must not have mutated the bucket.
        assert_eq!(map.multiplicity(&"one"), 1);
    }
}

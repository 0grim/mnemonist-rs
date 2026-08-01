//! Port of upstream `lru-cache.js`, `lru-map.js`, `lru-cache-with-delete.js`
//! and `lru-map-with-delete.js` (one unit — DESIGN.md §1.1: `test/lru-cache.js`
//! requires all four).
//!
//! A doubly-linked list over static index arrays (`forward`/`backward`), so
//! promoting an entry to the front (`splayOnTop`) never allocates. What varies
//! across the four upstream files is only the *pointer index* used to find a
//! key's slot:
//!
//! | file | index | key stored back |
//! |---|---|---|
//! | `lru-cache` | plain object, keys **string-coerced** by the index | raw, untouched |
//! | `lru-map` | real `Map`, SameValueZero | raw, untouched |
//! | `*-with-delete` | either of the above, plus a hole list for reused pointers | — |
//!
//! Upstream's own `for (var k in LRUCache.prototype) LRUCacheWithDelete.prototype[k] = …`
//! and `LRUMap.prototype.keys = LRUCache.prototype.keys` make the algorithm
//! itself identical across all four; only the index's *key equality* and
//! whether holes are ever populated differ. So this module is **one generic
//! engine**, [`LruCache<IK, K, V>`], parameterised on:
//!
//! * `IK` — the *index key*: what the pointer map is keyed by. `Hash + Eq`
//!   only, because equality is the only thing the index needs and it is a
//!   property of the bridge's key type (`PropertyKey` for the object-backed
//!   pair, `JsKey` for the `Map`-backed pair — see `mnemonist-napi`).
//! * `K` — the *stored key*, handed back by `keys()`/`entries()`. Kept
//!   **separate** from `IK` because the object-backed index string-coerces
//!   its lookup key while `this.K[pointer]` keeps the raw value —
//!   `test/lru-cache.js:65` asserts both halves independently.
//! * `V` — the stored value.
//!
//! `holes`/`holes_len` are always present and are the *entire* difference
//! between the plain and `-with-delete` variants: [`LruCache::delete`] and
//! [`LruCache::remove`] are simply never called by the two bridges that don't
//! expose them, so `holes_len` stays `0` and [`LruCache::set`]'s "cache is not
//! yet full" branch always takes the `pointer = self.size` arm — which is
//! upstream's own base `set` in its entirety, since it has no `deletedSize` at
//! all. One algorithm, reproduced once, serves all four.
//!
//! # The one place a raw key and an index key can disagree
//!
//! Eviction removes the *displaced* entry from the index by re-deriving its
//! index key from the **stored** `K`, exactly as upstream's
//! `delete this.items[this.K[pointer]]` does — not from the index key that was
//! originally used to insert it. For the `Map`-backed pair the two always
//! agree (`IK` and `K` are both `JsKey`, unmodified). For the object-backed
//! pair they can differ when a caller supplies a *narrowing* `Keys` array
//! class (e.g. `Uint8Array`): `this.K[pointer]` is the coerced value, and
//! evicting under its string form rather than the original raw key's is
//! upstream's own behaviour, bug-for-bug — see `to_index` below and
//! `docs/modules/lru-cache.md`.
//!
//! # `Sequence`, one impl for `keys`/`values`/`entries`/`forEach`
//!
//! All four of upstream's walks share one shape: `l = this.size` and
//! `pointer = this.head` are frozen at creation, then every step reads
//! `keys[pointer]`/`values[pointer]` live and advances via `forward[pointer]`
//! — also read live. That is exactly [`crate::cursor::Sequence`]'s hybrid
//! capture, with the frozen pointer riding in `Frozen` as a `Cell<usize>`
//! (mutated *inside* `slot`, which only ever receives `&Self::Frozen`) next to
//! a [`Projection`] tag that says which of the three walks this is — the same
//! trick `SparseMap` uses for its three cursors over one frozen `size`.
//!
//! Because `forward`/`backward`/`keys`/`values` never shrink below `capacity`
//! for the life of the structure, a slot at any pointer in `0..capacity` is
//! always in bounds — so [`crate::cursor::Step::Gap`] can never arise here,
//! unlike `Stack`/`Queue`'s growable backing arrays. `slot` always returns
//! `Some`.

use std::cell::Cell;
use std::hash::Hash;

use crate::cursor::{CursorState, Sequence};
use crate::utils::typed_arrays::{get_pointer_array, PointerVec};

/// Why [`LruCache::new`] refused a capacity.
///
/// Both variants are unreachable through the two bridges, which validate a
/// finite positive integer capacity before core ever sees it (the two upstream
/// messages differ only in *wording*, not in the numeric checks themselves —
/// see `mnemonist_napi::lru_cache`). Exposed for Rust callers and for the
/// `capacity` upper bound, which is a real `mnemonist-core` concern:
/// `typed.getPointerArray` throwing is upstream behaviour, not a bridge
/// artefact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewError {
    /// `capacity == 0`. The non-numeric half of upstream's guard (`NaN`,
    /// `Infinity`, a non-integer) is a floating-point concern the bridge
    /// resolves before calling in; `usize` can only express the boundary
    /// case.
    ZeroCapacity,
    /// `capacity` exceeds what `getPointerArray` can index — see
    /// [`crate::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE`].
    TooLarge,
}

/// What a `keys()`/`values()`/`entries()` cursor projects out of one slot.
///
/// Mirrors `mnemonist_core::structures::sparse_map::Projected`: three walks
/// over one frozen state, told apart by [`Projection`] rather than by three
/// `Sequence` impls, because Rust allows only one per type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Projected<K, V> {
    Key(K),
    Value(V),
    Entry(K, V),
}

impl<K, V> Projected<K, V> {
    pub fn key(self) -> Option<K> {
        match self {
            Self::Key(key) | Self::Entry(key, _) => Some(key),
            Self::Value(_) => None,
        }
    }

    pub fn value(self) -> Option<V> {
        match self {
            Self::Value(value) | Self::Entry(_, value) => Some(value),
            Self::Key(_) => None,
        }
    }

    pub fn entry(self) -> Option<(K, V)> {
        match self {
            Self::Entry(key, value) => Some((key, value)),
            Self::Key(_) | Self::Value(_) => None,
        }
    }
}

/// Which of the three walks a [`LruCache`] cursor is performing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    Keys,
    Values,
    Entries,
}

/// Upstream's `setpop` return: `null`, `{evicted: false, ...}` or
/// `{evicted: true, ...}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetPop<K, V> {
    /// Neither an overwrite nor an eviction: `null`.
    None,
    /// The key already existed; `key`/`value` are the caller's new pair and
    /// the *old* value is what was displaced. Upstream returns the key
    /// unchanged (it did not move), so no `K: Clone` is needed here — the
    /// caller's own `key` argument is handed back.
    Overwritten { key: K, value: V },
    /// The cache was full; `key`/`value` are the evicted pair.
    Evicted { key: K, value: V },
}

/// The shared engine behind all four upstream files. See the module docs.
pub struct LruCache<IK, K, V>
where
    IK: Hash + Eq,
{
    capacity: usize,
    forward: PointerVec,
    backward: PointerVec,
    /// Freed pointers, reusable by [`LruCache::set`]/[`LruCache::set_pop`].
    /// Always allocated; only ever populated by [`LruCache::delete`]/
    /// [`LruCache::remove`], which the two non-`-with-delete` bridges never
    /// call. See the module docs.
    holes: PointerVec,
    holes_len: usize,
    keys: Vec<Option<K>>,
    values: Vec<Option<V>>,
    index: std::collections::HashMap<IK, usize>,
    size: usize,
    head: usize,
    tail: usize,
}

impl<IK: Hash + Eq, K, V> LruCache<IK, K, V> {
    /// `new LRUCache(capacity)`, with the capacity already validated as a
    /// finite positive integer (upstream's two `throw`s are JS type
    /// questions and live at the boundary — see the module docs).
    pub fn new(capacity: usize) -> Result<Self, NewError> {
        if capacity == 0 {
            return Err(NewError::ZeroCapacity);
        }

        let width = get_pointer_array(capacity as f64).map_err(|_| NewError::TooLarge)?;

        Ok(Self {
            capacity,
            forward: PointerVec::zeroed(width, capacity),
            backward: PointerVec::zeroed(width, capacity),
            holes: PointerVec::zeroed(width, capacity),
            holes_len: 0,
            keys: std::iter::repeat_with(|| None).take(capacity).collect(),
            values: std::iter::repeat_with(|| None).take(capacity).collect(),
            index: std::collections::HashMap::new(),
            size: 0,
            head: 0,
            tail: 0,
        })
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Upstream's `size` property — the live entry count, unlike
    /// `DefaultMap`'s drifting counter (B-40); nothing in this family
    /// diverges the two.
    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Upstream's `clear`. Resets the index and the bookkeeping only —
    /// `keys`/`values`/`forward`/`backward` are left exactly as they were,
    /// because upstream's `this.items = {}` / `new Map()` rebinds only the
    /// index and never touches `this.K`/`this.V`. A stored value already held
    /// alive (an object reference, at the bridge) stays alive until
    /// overwritten, precisely as it would in JS.
    pub fn clear(&mut self) {
        self.size = 0;
        self.head = 0;
        self.tail = 0;
        self.holes_len = 0;
        self.index.clear();
    }

    /// Upstream's `splayOnTop`: unlink `pointer` and relink it as `head`.
    /// A no-op when it already is the head.
    fn splay_on_top(&mut self, pointer: usize) {
        let old_head = self.head;

        if old_head == pointer {
            return;
        }

        let previous = self.backward.get(pointer) as usize;
        let next = self.forward.get(pointer) as usize;

        if self.tail == pointer {
            self.tail = previous;
        } else {
            self.backward.set(next, previous as u32);
        }

        self.forward.set(previous, next as u32);
        self.backward.set(old_head, pointer as u32);
        self.head = pointer;
        self.forward.set(pointer, old_head as u32);
    }

    /// Acquire a pointer for a brand-new key and thread it onto the front of
    /// the list, evicting the tail first if the cache is already full.
    ///
    /// `to_index` re-derives the evicted slot's index key from its **stored**
    /// `K`, not from any index key the caller still has lying around —
    /// upstream's `delete this.items[this.K[pointer]]` does the same
    /// re-derivation, and the module docs cover why that can matter.
    fn insert_new<F: Fn(&K) -> IK>(
        &mut self,
        index_key: IK,
        key: K,
        value: V,
        to_index: F,
    ) -> Option<(K, V)> {
        let (pointer, evicted) = if self.size < self.capacity {
            let pointer = if self.holes_len > 0 {
                self.holes_len -= 1;
                self.holes.get(self.holes_len) as usize
            } else {
                self.size
            };

            self.size += 1;

            (pointer, None)
        } else {
            let pointer = self.tail;
            self.tail = self.backward.get(pointer) as usize;

            let old_key = self.keys[pointer]
                .take()
                .expect("the tail pointer always holds a live key");
            let old_value = self.values[pointer]
                .take()
                .expect("the tail pointer always holds a live value");
            let old_index_key = to_index(&old_key);
            self.index.remove(&old_index_key);

            (pointer, Some((old_key, old_value)))
        };

        self.index.insert(index_key, pointer);
        self.keys[pointer] = Some(key);
        self.values[pointer] = Some(value);

        self.forward.set(pointer, self.head as u32);
        self.backward.set(self.head, pointer as u32);
        self.head = pointer;

        evicted
    }

    /// The linked-list splice shared by `delete` and `remove` once the index
    /// entry is already gone.
    fn unlink(&mut self, pointer: usize) {
        self.keys[pointer] = None;
        self.values[pointer] = None;

        if self.size == 1 {
            self.size = 0;
            self.head = 0;
            self.tail = 0;
            self.holes_len = 0;
            return;
        }

        let previous = self.backward.get(pointer) as usize;
        let next = self.forward.get(pointer) as usize;

        if self.head == pointer {
            self.head = next;
        }
        if self.tail == pointer {
            self.tail = previous;
        }

        self.forward.set(previous, next as u32);
        self.backward.set(next, previous as u32);
        self.size -= 1;

        self.holes.set(self.holes_len, pointer as u32);
        self.holes_len += 1;
    }

    /// Upstream's `has`: asks the index, not the stored value.
    pub fn has(&self, index_key: &IK) -> bool {
        self.index.contains_key(index_key)
    }

    /// Upstream's `peek`: no splay, no promotion.
    pub fn peek(&self, index_key: &IK) -> Option<&V> {
        let &pointer = self.index.get(index_key)?;

        self.values[pointer].as_ref()
    }

    /// Upstream's `get`: splays the entry to the front on a hit.
    pub fn get(&mut self, index_key: &IK) -> Option<&V> {
        let &pointer = self.index.get(index_key)?;

        self.splay_on_top(pointer);

        self.values[pointer].as_ref()
    }

    /// Upstream's `set`. `key` is the value `keys()`/`entries()` will read
    /// back; `to_index` is only ever invoked if this insertion evicts.
    pub fn set<F: Fn(&K) -> IK>(&mut self, index_key: IK, key: K, value: V, to_index: F) {
        if let Some(&pointer) = self.index.get(&index_key) {
            self.splay_on_top(pointer);
            self.values[pointer] = Some(value);
            return;
        }

        self.insert_new(index_key, key, value, to_index);
    }

    /// Upstream's `setpop`.
    pub fn set_pop<F: Fn(&K) -> IK>(
        &mut self,
        index_key: IK,
        key: K,
        value: V,
        to_index: F,
    ) -> SetPop<K, V> {
        if let Some(&pointer) = self.index.get(&index_key) {
            self.splay_on_top(pointer);

            let old_value = self.values[pointer]
                .replace(value)
                .expect("a live slot always holds a value");

            return SetPop::Overwritten {
                key,
                value: old_value,
            };
        }

        match self.insert_new(index_key, key, value, to_index) {
            None => SetPop::None,
            Some((old_key, old_value)) => SetPop::Evicted {
                key: old_key,
                value: old_value,
            },
        }
    }

    /// Upstream's `delete` (the `-with-delete` pair only). `false` for a
    /// missing key; the linked-list splice and hole recording are shared with
    /// [`LruCache::remove`] via [`LruCache::unlink`].
    pub fn delete(&mut self, index_key: &IK) -> bool {
        let Some(pointer) = self.index.remove(index_key) else {
            return false;
        };

        self.unlink(pointer);

        true
    }

    /// Upstream's `remove` (the `-with-delete` pair only): like
    /// [`LruCache::delete`], but returns the value. `None` is "no such key";
    /// the bridge substitutes the caller's `missing` argument for that case
    /// and passes a *present* `undefined`-valued entry through unchanged,
    /// since the two are told apart by whether this returns at all, not by
    /// what it returns.
    pub fn remove(&mut self, index_key: &IK) -> Option<V> {
        let pointer = self.index.remove(index_key)?;
        let value = self.values[pointer].take();

        self.unlink(pointer);

        value
    }
}

impl<IK: Hash + Eq, K: Clone, V: Clone> Sequence for LruCache<IK, K, V> {
    type Item = Projected<K, V>;
    /// The travelling pointer, plus which of the three walks this is.
    /// `Cell` because [`Sequence::slot`] only ever receives `&Self::Frozen`,
    /// and advancing the pointer is exactly upstream's
    /// `if (i < l) pointer = forward[pointer]`.
    type Frozen = (Cell<usize>, Projection);

    fn freeze(&self) -> (Self::Frozen, usize) {
        ((Cell::new(self.head), Projection::Entries), self.size)
    }

    /// Always `Some`: `forward`/`backward`/`keys`/`values` never shrink below
    /// `capacity`, so every pointer in `0..capacity` is in bounds for the life
    /// of the structure. See the module docs.
    fn slot(&self, frozen: &Self::Frozen, _ordinal: usize) -> Option<Self::Item> {
        let pointer = frozen.0.get();
        let key = self.keys[pointer]
            .clone()
            .expect("a pointer reachable from `head` within `size` steps is always live");
        let value = self.values[pointer]
            .clone()
            .expect("a pointer reachable from `head` within `size` steps is always live");

        frozen.0.set(self.forward.get(pointer) as usize);

        Some(match frozen.1 {
            Projection::Keys => Projected::Key(key),
            Projection::Values => Projected::Value(value),
            Projection::Entries => Projected::Entry(key, value),
        })
    }
}

impl<IK: Hash + Eq, K: Clone, V: Clone> LruCache<IK, K, V> {
    /// Open one of the three walks — upstream's `keys`/`values`/`entries`,
    /// and the shape `forEach` uses internally too. See the module docs for
    /// why one `Sequence` impl, selected by [`Projection`], serves all four.
    pub fn walk(&self, projection: Projection) -> CursorState<Self> {
        CursorState::open_projected(self, (Cell::new(self.head), projection))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `IK == K`: the `lru-map` shape, where the index key and the stored key
    /// are the same value and eviction can never disagree with itself.
    fn identity(key: &&'static str) -> &'static str {
        key
    }

    fn cache(capacity: usize) -> LruCache<&'static str, &'static str, i32> {
        LruCache::new(capacity).expect("capacity is positive and in range")
    }

    fn entries(cache: &LruCache<&'static str, &'static str, i32>) -> Vec<(&'static str, i32)> {
        entries_of(cache)
    }

    fn entries_of<IK: Hash + Eq, K: Clone + std::fmt::Debug, V: Clone>(
        cache: &LruCache<IK, K, V>,
    ) -> Vec<(K, V)> {
        let mut walk = cache.walk(Projection::Entries);
        let mut out = Vec::new();

        while let Some(item) = walk.step(cache).item() {
            out.push(item.entry().expect("an Entries walk yields Entry"));
        }

        out
    }

    #[test]
    fn zero_capacity_is_refused() {
        assert!(matches!(
            LruCache::<&str, &str, i32>::new(0),
            Err(NewError::ZeroCapacity)
        ));
    }

    #[test]
    fn reproduces_the_upstream_walkthrough() {
        let mut cache = cache(3);

        cache.set("one", "one", 1, identity);
        cache.set("two", "two", 2, identity);
        assert_eq!(cache.len(), 2);
        assert_eq!(entries(&cache), vec![("two", 2), ("one", 1)]);

        cache.set("three", "three", 3, identity);
        assert_eq!(entries(&cache), vec![("three", 3), ("two", 2), ("one", 1)]);

        cache.set("four", "four", 4, identity);
        assert_eq!(cache.len(), 3);
        assert_eq!(entries(&cache), vec![("four", 4), ("three", 3), ("two", 2)]);

        cache.set("two", "two", 5, identity);
        assert_eq!(entries(&cache), vec![("two", 5), ("four", 4), ("three", 3)]);

        assert!(cache.has(&"four"));
        assert!(!cache.has(&"one"));
        assert_eq!(cache.get(&"one"), None);
        assert_eq!(cache.get(&"four"), Some(&4));
        assert_eq!(entries(&cache), vec![("four", 4), ("two", 5), ("three", 3)]);
    }

    #[test]
    fn setpop_reports_none_overwritten_and_evicted() {
        let mut cache = cache(2);

        assert_eq!(
            cache.set_pop("a", "a", 1, identity),
            SetPop::None,
            "growth never overwrites or evicts"
        );
        assert_eq!(
            cache.set_pop("a", "a", 9, identity),
            SetPop::Overwritten { key: "a", value: 1 }
        );

        cache.set("b", "b", 2, identity);
        assert_eq!(
            cache.set_pop("c", "c", 3, identity),
            SetPop::Evicted { key: "a", value: 9 },
            "a evicted as the least recently used"
        );
    }

    #[test]
    fn capacity_of_one_evicts_on_every_new_key() {
        let mut cache = cache(1);

        cache.set("one", "one", 1, identity);
        cache.set("two", "two", 2, identity);
        cache.set("three", "three", 3, identity);

        assert_eq!(entries(&cache), vec![("three", 3)]);
        assert_eq!(cache.get(&"one"), None);
        assert_eq!(cache.get(&"three"), Some(&3));
    }

    #[test]
    fn delete_and_remove_maintain_lru_order() {
        let mut cache = cache(3);

        assert!(!cache.delete(&"one"), "deleting from an empty cache");

        cache.set("one", "one", 1, identity);
        cache.set("two", "two", 2, identity);
        cache.set("three", "three", 3, identity);

        assert!(cache.delete(&"three"), "delete the head");
        assert_eq!(entries(&cache), vec![("two", 2), ("one", 1)]);
        assert!(!cache.delete(&"three"), "already gone");

        cache.set("three", "three", 30, identity);
        assert_eq!(entries(&cache), vec![("three", 30), ("two", 2), ("one", 1)]);

        assert_eq!(cache.remove(&"two"), Some(2), "remove the middle");
        assert_eq!(entries(&cache), vec![("three", 30), ("one", 1)]);
        assert_eq!(cache.remove(&"missing"), None);

        assert_eq!(cache.remove(&"one"), Some(1), "remove the tail");
        assert_eq!(cache.remove(&"three"), Some(30), "remove the only key");
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    /// Holes are reused before growth resumes, and the reuse never disturbs
    /// LRU order — the `-with-delete` pair's entire reason to exist.
    #[test]
    fn a_deleted_slot_is_reused_by_the_next_insert() {
        let mut cache = cache(4);

        cache.set("a", "a", 1, identity);
        cache.set("b", "b", 2, identity);
        cache.set("c", "c", 3, identity);
        assert!(cache.delete(&"b"));

        cache.set("d", "d", 4, identity);
        cache.set("e", "e", 5, identity);

        assert_eq!(
            entries(&cache),
            vec![("e", 5), ("d", 4), ("c", 3), ("a", 1)]
        );
    }

    /// The one place a stored key and an index key can disagree: eviction
    /// re-derives the index key from the *stored* `K` via `to_index`, exactly
    /// as upstream's `delete this.items[this.K[pointer]]` re-reads `this.K`
    /// rather than reusing the key `set` was originally called with.
    ///
    /// Modelled on the object-backed pair's real failure mode: a narrowing
    /// `Keys` array class (`Uint8Array`) can store a *different* value than
    /// the one the property-string index was built from (`300` narrows to
    /// `44`). `to_index` here mirrors that gap directly — the insertion index
    /// key is the raw value's string form, but the stored `K` is already the
    /// "coerced" (here: `+ 0`, unchanged) form used for re-derivation, so
    /// eviction computes a DIFFERENT string than the one the entry was filed
    /// under and leaves a stale index entry. Bug-for-bug: see
    /// `docs/modules/lru-cache.md`.
    #[test]
    fn eviction_re_derives_the_index_key_from_the_stored_key_and_can_leave_it_stale() {
        fn to_index(key: &i32) -> String {
            key.to_string()
        }

        let mut cache: LruCache<String, i32, &'static str> =
            LruCache::new(1).expect("capacity is positive");

        // Inserted under index key "300" (as if that were the property-string
        // form of some raw argument), but the *stored* key is already 44 — the
        // narrowed form `to_index` will see again at eviction time.
        cache.set(String::from("300"), 44, "first", to_index);
        assert!(cache.has(&String::from("300")));

        // Forces eviction of the sole entry. `to_index(44)` is "44", not
        // "300", so the index entry filed under "300" is never removed.
        cache.set(String::from("999"), 99, "second", to_index);

        assert_eq!(
            entries_of(&cache),
            vec![(99, "second")],
            "the linked list itself holds only the surviving entry"
        );
        assert!(
            cache.has(&String::from("300")),
            "upstream's own bug, reproduced: the stale index entry is never \
             cleaned up when the stored key and the index key disagree"
        );
    }

    #[test]
    fn peek_does_not_disturb_order() {
        let mut cache = cache(3);

        cache.set("one", "one", 1, identity);
        cache.set("two", "two", 2, identity);
        cache.set("three", "three", 3, identity);

        assert_eq!(cache.peek(&"two"), Some(&2));
        assert_eq!(entries(&cache), vec![("three", 3), ("two", 2), ("one", 1)]);
    }

    #[test]
    fn clear_resets_bookkeeping_but_a_stale_slot_is_never_reachable() {
        let mut cache = cache(2);

        cache.set("one", "one", 1, identity);
        cache.clear();

        assert_eq!(cache.len(), 0);
        assert!(!cache.has(&"one"));
        assert_eq!(entries(&cache), Vec::<(&str, i32)>::new());

        cache.set("two", "two", 2, identity);
        assert_eq!(entries(&cache), vec![("two", 2)]);
    }

    #[test]
    fn keys_and_values_project_the_same_walk_differently() {
        let mut cache = cache(3);

        cache.set("one", "one", 1, identity);
        cache.set("two", "two", 2, identity);

        let mut keys_walk = cache.walk(Projection::Keys);
        let mut keys = Vec::new();
        while let Some(item) = keys_walk.step(&cache).item() {
            keys.push(item.key().expect("a Keys walk yields Key"));
        }
        assert_eq!(keys, vec!["two", "one"]);

        let mut values_walk = cache.walk(Projection::Values);
        let mut values = Vec::new();
        while let Some(item) = values_walk.step(&cache).item() {
            values.push(item.value().expect("a Values walk yields Value"));
        }
        assert_eq!(values, vec![2, 1]);
    }
}

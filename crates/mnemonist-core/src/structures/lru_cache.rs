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
//! # `Sequence`, one impl for `keys`/`values`/`entries` — but not `forEach`
//!
//! `keys`/`values`/`entries` share one shape: `l = this.size` and
//! `pointer = this.head` are frozen at creation, then every step reads
//! `keys[pointer]`/`values[pointer]` live and advances via `forward[pointer]`
//! — also read live, and *before* control ever returns to whatever called
//! `.next()`. That is exactly [`crate::cursor::Sequence`]'s hybrid capture,
//! with the frozen pointer riding in `Frozen` as a `Cell<usize>` (mutated
//! *inside* `slot`, which only ever receives `&Self::Frozen`) next to a
//! [`Projection`] tag that says which of the three walks this is — the same
//! trick `SparseMap` uses for its three cursors over one frozen `size`.
//!
//! `forEach` looks identical at first read — `l`/`pointer` frozen the same
//! way, `forward` read live the same way — but its callback runs **before**
//! upstream's own `pointer = forward[pointer]`, not after, because the
//! callback executes from inside upstream's loop body while `Sequence::slot`
//! advances eagerly, before the caller (whether that is a JS callback or this
//! crate's own difffuzz harness) ever gets control. Reusing `Sequence` for
//! `forEach` reproduced the eager-advance timing everywhere, which is right
//! for the three lazy iterators and wrong for `forEach` — see
//! [`ForEachWalk`], and `docs/modules/lru-cache.md`'s "Bugs this found" for
//! how the differential fuzzer caught it on its first campaign.
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

    /// Upstream's `head` property: the pointer of the most recently used
    /// entry. `test/lru-cache.js` asserts this directly after emptying a
    /// `-with-delete` cache (`cache.head === 0`).
    pub fn head(&self) -> usize {
        self.head
    }

    /// Upstream's `tail` property: the pointer of the least recently used
    /// entry.
    pub fn tail(&self) -> usize {
        self.tail
    }

    /// The pointer index, key by key — upstream's `this.items` itself, which
    /// `test/lru-cache.js:65` inspects directly
    /// (`Object.keys(cache.items).length` / `cache.items.size`). Exposed so
    /// the bridge can rebuild the same shape the original test observes
    /// without needing a second copy of the index.
    pub fn index_entries(&self) -> impl Iterator<Item = (&IK, usize)> {
        self.index.iter().map(|(key, &pointer)| (key, pointer))
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
    ///
    /// Deliberately does **not** clear `self.keys[pointer]`/`self.values[pointer]`.
    /// Neither `delete` nor `remove` touches `this.K`/`this.V` upstream — see
    /// `~/upstream-mnemonist/lru-cache-with-delete.js`, where both simply
    /// splice the linked list and record the hole. A pointer's slot is only
    /// ever overwritten by [`LruCache::insert_new`], on reuse. So a stale
    /// `Some` is left behind on purpose: it is what upstream's own `this.K`/
    /// `this.V` arrays hold too, and it is what fixed a real port bug (see the
    /// module docs above and `docs/modules/lru-cache.md`, "Bugs this
    /// found") — an in-flight `keys()`/`values()`/`entries()`/`forEach` walk
    /// whose frozen bound had not yet reached this pointer used to panic on
    /// the `.expect` in [`LruCache::slot`] the moment `delete`/`remove` ran
    /// underneath it, because the slot the walk was about to visit had just
    /// been nulled. Upstream never nulls it, so it just returns the stale
    /// (soon-to-be-overwritten-or-not) key/value instead of throwing anything
    /// at all — reproduced bug-for-bug here rather than "fixed" a second time.
    fn unlink(&mut self, pointer: usize) {
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
    ///
    /// `V: Clone` here (and only here, not on the surrounding `impl` block):
    /// upstream's `var dead = this.V[pointer];` reads the value without
    /// disturbing the array, so `this.V[pointer]` is still the just-removed
    /// value afterwards — stale, exactly like [`LruCache::unlink`] leaves
    /// `this.K[pointer]`, and reachable the same way, by a walk whose frozen
    /// bound has not yet reached this pointer. Taking ownership outright
    /// (`.take()`) would zero it instead, reintroducing on this one field
    /// exactly the crash [`LruCache::unlink`]'s doc comment describes; a
    /// clone is what lets both the return value AND the stale slot exist at
    /// once, matching upstream's read without a write.
    pub fn remove(&mut self, index_key: &IK) -> Option<V>
    where
        V: Clone,
    {
        let pointer = self.index.remove(index_key)?;
        let value = self.values[pointer].clone();

        self.unlink(pointer);

        value
    }
}

/// Upstream's `forEach` walk — deliberately **not** built on [`Sequence`]/
/// [`crate::cursor::CursorState`], because `forEach`'s timing is different
/// from `keys`/`values`/`entries`' and reusing the wrong one was a real port
/// bug, found by the differential fuzzer on its very first campaign against
/// this grammar (see `docs/modules/lru-cache.md`, "Bugs this found").
///
/// Upstream's loop body is:
///
/// ```js
/// while (i < l) {
///   callback.call(scope, values[pointer], keys[pointer], this);
///   pointer = forward[pointer];   // <-- AFTER the callback, not before
///   i++;
/// }
/// ```
///
/// The lazy iterators' closures advance `pointer` themselves, before ever
/// returning control to whatever called `.next()` — so from a caller's
/// point of view, a mutation it makes between two `.next()` calls can never
/// observe anything about the step that already happened, which is exactly
/// what [`Sequence::slot`]'s eager advance reproduces (see that impl's own
/// docs). `forEach` is different: the callback that might mutate the cache
/// runs **while control is still inside upstream's own loop**, textually
/// before the advance. A `forEach` built on `Sequence` (as an early version
/// of this port's `$forEach` handling and the napi bridge's `for_each_entries`
/// both were) captures `forward[pointer]` before the caller's callback runs
/// — the wrong side of a mutation that relinks the very pointer this walk is
/// about to follow next.
///
/// So this type gives the caller the two halves separately: read
/// [`ForEachWalk::current`], run whatever callback/mutation the caller has,
/// **then** call [`ForEachWalk::advance`] — which reads `forward` live, after
/// that mutation, exactly where upstream's assignment sits in its own loop.
pub struct ForEachWalk {
    pointer: usize,
    remaining: usize,
}

impl<IK: Hash + Eq, K: Clone, V: Clone> LruCache<IK, K, V> {
    /// Open a `forEach`-shaped walk over the cache as it stands right now —
    /// `head`/`size`, frozen, exactly like every other walk here.
    pub fn for_each_walk(&self) -> ForEachWalk {
        ForEachWalk {
            pointer: self.head,
            remaining: self.size,
        }
    }
}

impl ForEachWalk {
    /// The current `(key, value)`, or `None` once exhausted. Does **not**
    /// advance — call [`ForEachWalk::advance`] once the caller's own
    /// callback for this position has run.
    pub fn current<IK: Hash + Eq, K: Clone, V: Clone>(
        &self,
        cache: &LruCache<IK, K, V>,
    ) -> Option<(K, V)> {
        if self.remaining == 0 {
            return None;
        }

        let key = cache.keys[self.pointer]
            .clone()
            .expect("a pointer reachable from `head` within `size` steps is always live");
        let value = cache.values[self.pointer]
            .clone()
            .expect("a pointer reachable from `head` within `size` steps is always live");

        Some((key, value))
    }

    /// Advance past the current position — reading `forward` **now**, live,
    /// after whatever the caller's callback for this position just did. A
    /// no-op once exhausted.
    pub fn advance<IK: Hash + Eq, K, V>(&mut self, cache: &LruCache<IK, K, V>) {
        if self.remaining == 0 {
            return;
        }

        self.pointer = cache.forward.get(self.pointer) as usize;
        self.remaining -= 1;
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
    /// The frozen state one of the three walks starts from: `head`, captured
    /// now, next to which walk this is. Exposed so the napi bridge can open a
    /// [`crate::cursor::CellCursor`] directly (it needs the `Frozen` payload,
    /// not a borrowing [`CursorState`]) without reaching into private fields.
    pub fn frozen(&self, projection: Projection) -> <Self as Sequence>::Frozen {
        (Cell::new(self.head), projection)
    }

    /// Open one of the three walks — upstream's `keys`/`values`/`entries`,
    /// and the shape `forEach` uses internally too. See the module docs for
    /// why one `Sequence` impl, selected by [`Projection`], serves all four.
    pub fn walk(&self, projection: Projection) -> CursorState<Self> {
        CursorState::open_projected(self, self.frozen(projection))
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

    /// A port-only defect, found by reading (not fuzzing) before the fuzz
    /// grammar existed at all, and fixed before it could ever have run: an
    /// in-flight `entries()`/`keys()`/`values()` walk whose frozen `size`
    /// bound has not yet reached a pointer, when `delete` unlinks exactly that
    /// pointer, used to panic — `unlink` nulled `self.keys[pointer]`, and the
    /// walk's `.expect("a pointer reachable ... is always live")` then found
    /// `None`. Upstream's `delete` never touches `this.K`/`this.V` (confirmed
    /// against `~/upstream-mnemonist/lru-cache-with-delete.js`), so it just
    /// returns the *stale* key/value at that position instead of throwing
    /// anything — this test is the same three-op program that panicked before
    /// the fix, now pinned to upstream's actual (unglamorous) answer.
    #[test]
    fn a_delete_of_the_walks_next_unvisited_pointer_yields_stale_data_not_a_panic() {
        let mut cache = cache(4);

        cache.set("a", "a", 1, identity);
        cache.set("b", "b", 2, identity);
        cache.set("c", "c", 3, identity);
        cache.set("d", "d", 4, identity);
        // head -> tail: d, c, b, a
        let mut walk = cache.walk(Projection::Entries);

        assert_eq!(
            walk.step(&cache).item().and_then(Projected::entry),
            Some(("d", 4)),
            "first step reads the head and advances the walk's own pointer to c"
        );

        // Delete "c" -- the key the walk's internal pointer now sits on, one
        // step ahead of what it has actually yielded, and has therefore not
        // yet visited.
        assert!(cache.delete(&"c"));

        assert_eq!(
            walk.step(&cache).item().and_then(Projected::entry),
            Some(("c", 3)),
            "stale, not None: upstream's this.K[pointer]/this.V[pointer] were \
             never cleared by delete, so the frozen walk reads exactly what \
             was there a moment ago rather than anything decided by the \
             deletion"
        );
    }

    /// [`LruCache::remove`]'s half of the same defect: it independently reads
    /// the value before unlinking, so simply not-nulling `unlink` was not
    /// enough on its own -- `remove` had to stop taking ownership of the slot
    /// too. Same shape, the other method.
    #[test]
    fn a_remove_of_the_walks_next_unvisited_pointer_yields_stale_data_not_a_panic() {
        let mut cache = cache(4);

        cache.set("a", "a", 1, identity);
        cache.set("b", "b", 2, identity);
        cache.set("c", "c", 3, identity);
        cache.set("d", "d", 4, identity);
        let mut walk = cache.walk(Projection::Entries);

        assert_eq!(
            walk.step(&cache).item().and_then(Projected::entry),
            Some(("d", 4))
        );
        assert_eq!(cache.remove(&"c"), Some(3));
        assert_eq!(
            walk.step(&cache).item().and_then(Projected::entry),
            Some(("c", 3)),
            "remove reads the value by cloning it, not by taking it, so the \
             slot stays populated for a walk already in flight"
        );
    }

    /// The sharper version of the same hazard: the pointer `delete` frees is
    /// reused by a LATER `set` before the walk that predates the delete ever
    /// reaches it. The walk then reads the NEW occupant of that slot, and
    /// follows ITS `forward` link rather than the old one -- exactly what
    /// upstream's own array-of-pointers algorithm would do, character for
    /// character, since nothing in either language distinguishes "stale" from
    /// "reused" at the array level. Not a bug: a faithful reproduction of an
    /// algorithm that was never defended against concurrent mutation on
    /// either side.
    #[test]
    fn a_freed_pointer_reused_before_a_stale_walk_reaches_it_surfaces_the_new_occupant() {
        let mut cache = cache(4);

        cache.set("a", "a", 1, identity);
        cache.set("b", "b", 2, identity);
        cache.set("c", "c", 3, identity);
        cache.set("d", "d", 4, identity);
        // head -> tail: d, c, b, a
        let mut walk = cache.walk(Projection::Entries);

        assert_eq!(
            walk.step(&cache).item().and_then(Projected::entry),
            Some(("d", 4))
        );
        assert!(cache.delete(&"c")); // frees c's pointer; head -> tail: d, b, a
        cache.set("e", "e", 5, identity); // reuses c's freed pointer, becomes head

        // The walk's own cursor is still sitting on the pointer that was "c"
        // and is now "e" -- the SAME pointer, reused. It reads "e", not the
        // stale "c", because the slot really was overwritten this time.
        assert_eq!(
            walk.step(&cache).item().and_then(Projected::entry),
            Some(("e", 5))
        );
    }
}

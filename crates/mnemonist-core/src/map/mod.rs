//! JavaScript `Map` semantics, ported from ECMA-262 (DESIGN.md 3.8).
//!
//! Every module in bridge tier T3 — `default-map`, `bi-map`, `fuzzy-map`,
//! `multi-map`, `multi-set`, `lru-map` and friends — keeps its state in a
//! `new Map()`. So "porting T3" is, precisely, "reproducing `Map`". This
//! module is that, written once, exactly as [`crate::cursor`] is
//! `obliterator` written once.
//!
//! [`OrderedMap`] is generic over `K: Hash + Eq + Clone`. It never sees a
//! JavaScript value: SameValueZero is a property of the *key type*, and the
//! bridge supplies a key type that has it (`mnemonist_napi::js_key::JsKey`).
//! A Rust caller gets an ordinary insertion-ordered map.
//!
//! # The four behaviours `std::collections::HashMap` does not have
//!
//! **1. Guaranteed insertion order.** `HashMap` iterates arbitrarily. Every
//! T3 test file asserts iteration order — `deepStrictEqual` on `entries()`,
//! `keys()`, `values()` or `forEach` output — so order is not a nicety here,
//! it is most of the assertions.
//!
//! **2. Delete-then-reinsert moves the key to the end; overwrite does not.**
//!
//! ```js
//! var m = new Map([['a',1],['b',2],['c',3]]);
//! m.delete('a'); m.set('a', 9);  // keys: b, c, a
//! m.set('b', 9);                 // keys: b, c, a  — unmoved
//! ```
//!
//! Confirmed against Node 24.18.1. [`OrderedMap::set`] therefore updates in
//! place when the key is present and appends only when it is not — and
//! keeps the *original* key, which is what the spec's "if p.[[Key]] is
//! SameValueZero(key)" clause means.
//!
//! **3. Iterators are live, not snapshots.** A `Map` iterator is an index
//! into the entry list. Entries appended after it was created **are**
//! visited; entries deleted ahead of it are **skipped**; and once it has
//! reported `{done: true}` it detaches and never yields again, even if the
//! map grows afterwards. All four confirmed against Node 24.18.1:
//!
//! ```text
//! set after create, then next()          -> yields the new entry
//! delete ahead of the cursor, then next() -> skips it
//! next() past the end, then set, next()   -> {done: true} forever
//! clear(), set(), next()                  -> yields the new entry
//! ```
//!
//! This is a **different discipline from [`crate::cursor`]**, which freezes a
//! length at creation and reads elements lazily. Both are faithful; they are
//! faithful to different things. `obliterator` wraps an indexable sequence,
//! `Map` owns its entry list. Nothing here implements [`crate::cursor::Sequence`],
//! and that is deliberate — forcing one abstraction over both would get one
//! of them wrong.
//!
//! **4. Not restartable, and identity under `Symbol.iterator`.** Shared with
//! `obliterator`, and the reason [`OrderedMap`] does not implement
//! [`IntoIterator`] either (DIV-STACK-1).
//!
//! # How delete is made O(1) without breaking live cursors
//!
//! `delete` **tombstones**: the entry is emptied, the slot stays. Shifting the
//! vector would be O(n) per delete, and V8's own `OrderedHashMap` does not
//! shift either. Tombstones accumulate, so the vector is **compacted** once
//! the dead outnumber the live — again as V8 does, which rehashes on shrink.
//!
//! Compaction moves entries, which would invalidate a cursor holding a
//! physical index. So a cursor does not hold one. Every slot carries a
//! monotonically increasing **`id`**, assigned at insertion and never reused,
//! and `slots` is therefore always sorted strictly ascending by `id`
//! regardless of how many compactions have run. A [`MapCursor`] stores the
//! *id it wants next*, and locates it by binary search — with a physical-index
//! hint that makes the common, uncompacted step O(1).
//!
//! V8 solves the same problem by chaining old tables to new ones and
//! transitioning live iterators through a recorded hole list. The id is the
//! same idea with the bookkeeping deleted: it needs no communication between
//! the map and its cursors at all, so a [`MapCursor`] is `Copy`, holds no
//! borrow, and cannot be invalidated. That matters at the FFI boundary, where
//! a JS cursor outlives the call that produced it and the map stays mutable
//! underneath it.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::map::{MapCursor, OrderedMap};
//!
//! let mut map: OrderedMap<&str, u32> = OrderedMap::new();
//! map.set("a", 1);
//! map.set("b", 2);
//! map.set("c", 3);
//!
//! // A live cursor: the delete ahead of it is skipped.
//! let mut cursor = MapCursor::open();
//! assert_eq!(cursor.step(&map), Some((&"a", &1)));
//! map.delete(&"b");
//! assert_eq!(cursor.step(&map), Some((&"c", &3)));
//!
//! // Delete-then-reinsert moves to the end.
//! map.set("b", 9);
//! assert_eq!(map.keys().copied().collect::<Vec<_>>(), vec!["a", "c", "b"]);
//! ```

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;

/// Smallest slot vector worth compacting.
///
/// Below this the copy costs more than the tombstones do, and the churn shows
/// up in every `delete` on a two-element map.
const COMPACT_FLOOR: usize = 8;

/// One position in the entry list: alive, or a tombstone.
///
/// The `id` outlives the entry. It is what keeps `slots` sorted across
/// compactions and therefore what makes [`MapCursor`] compaction-proof; a
/// tombstone that dropped its id would break the ordering the binary search
/// depends on.
#[derive(Debug, Clone)]
struct Slot<K, V> {
    id: u64,
    entry: Option<(K, V)>,
}

/// An insertion-ordered map with JavaScript `Map` semantics.
///
/// See the module docs for the four behaviours this has and
/// [`std::collections::HashMap`] does not.
#[derive(Clone)]
pub struct OrderedMap<K, V> {
    /// Entry list, strictly ascending by `id`, with tombstones.
    slots: Vec<Slot<K, V>>,
    /// Key to physical index in `slots`. Live keys only.
    ///
    /// The key is stored twice, here and in the slot. `indexmap` avoids that
    /// with `hashbrown`'s raw-entry API; the core crate is zero-dependency by
    /// declaration, and `std::collections::HashMap` exposes no equivalent on
    /// stable. Recorded as a cost rather than hidden: it is one extra `K` per
    /// live entry, and for the bridge's key type that is one `String` clone.
    index: HashMap<K, usize>,
    /// Live entries — upstream's `Map.prototype.size`.
    live: usize,
    /// Next id to hand out. Never reset, not even by [`OrderedMap::clear`];
    /// see that method for why.
    next_id: u64,
}

impl<K, V> Default for OrderedMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for OrderedMap<K, V> {
    /// Entries in iteration order. Tombstones and ids are representation, and
    /// showing them would make every assertion message about the mechanism
    /// rather than about the map.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K, V> OrderedMap<K, V> {
    /// An empty map. `new Map()`.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            index: HashMap::new(),
            live: 0,
            next_id: 0,
        }
    }

    /// Live entries — upstream's `size` getter.
    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// Entries in insertion order.
    ///
    /// A convenience for Rust callers and for the differential fuzzer's state
    /// dump. It is **not** the JS iterator: it restarts, it borrows, and it
    /// cannot observe a mutation. [`MapCursor`] is the faithful one.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.slots.iter().filter_map(|slot| {
            let (key, value) = slot.entry.as_ref()?;

            Some((key, value))
        })
    }

    /// Keys in insertion order. See [`OrderedMap::iter`] for the caveat.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(key, _)| key)
    }

    /// Values in insertion order. See [`OrderedMap::iter`] for the caveat.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, value)| value)
    }

    /// Values in insertion order, mutably.
    ///
    /// Exists for the bridge: releasing the napi reference a value holds takes
    /// `&mut`, and `clear` has to do it for every live entry before dropping
    /// them.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.slots.iter_mut().filter_map(|slot| {
            let (_, value) = slot.entry.as_mut()?;

            Some(value)
        })
    }

    /// The entry at a physical slot, if that slot is alive.
    ///
    /// Physical indices are **not** stable across [`OrderedMap::delete`], which
    /// may compact. Use one only between the call that produced it and the
    /// next mutation; [`MapCursor`] is what to use for anything longer-lived.
    pub fn entry_at(&self, slot: usize) -> Option<(&K, &V)> {
        let (key, value) = self.slots.get(slot)?.entry.as_ref()?;

        Some((key, value))
    }

    /// `Map.prototype.clear`.
    ///
    /// `next_id` is deliberately **not** reset. The spec empties the entry
    /// records but leaves the list, so a cursor that has not yet finished
    /// sees entries added after the `clear`:
    ///
    /// ```js
    /// var m = new Map([['a',1]]), it = m[Symbol.iterator]();
    /// m.clear(); m.set('c',3);
    /// it.next();   // -> {value: ['c',3], done: false}
    /// ```
    ///
    /// Confirmed against Node 24.18.1. Keeping `next_id` monotonic makes every
    /// post-`clear` id greater than any live cursor's, which reproduces that
    /// exactly; resetting it to zero would make the new entries invisible to
    /// the cursor, which is the same bug one level down.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.index.clear();
        self.live = 0;
    }
}

impl<K: Hash + Eq + Clone, V> OrderedMap<K, V> {
    /// `Map.prototype.get`.
    pub fn get(&self, key: &K) -> Option<&V> {
        let slot = *self.index.get(key)?;
        let (_, value) = self.slots[slot].entry.as_ref()?;

        Some(value)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let slot = *self.index.get(key)?;
        let (_, value) = self.slots[slot].entry.as_mut()?;

        Some(value)
    }

    /// `Map.prototype.has`.
    pub fn contains_key(&self, key: &K) -> bool {
        self.index.contains_key(key)
    }

    /// The physical slot a key occupies, for callers that then want
    /// [`OrderedMap::entry_at`]. See that method on the stability of the index.
    pub fn slot_of(&self, key: &K) -> Option<usize> {
        self.index.get(key).copied()
    }

    /// `Map.prototype.set`, returning the value it displaced.
    ///
    /// `Some(old)` means the key was already present, so its position was kept
    /// and `old` was overwritten; `None` means it was appended. The displaced
    /// value is returned rather than dropped because the bridge has to release
    /// the napi reference it holds, and a silently dropped `V` there is a leak.
    ///
    /// On overwrite the **existing** key is kept and `key` is dropped, which
    /// is the spec's behaviour and matters for any key type where two equal
    /// keys are distinguishable — `-0` and `+0` most obviously.
    pub fn set(&mut self, key: K, value: V) -> Option<V> {
        if let Some(&slot) = self.index.get(&key) {
            let (_, existing) = self.slots[slot]
                .entry
                .as_mut()
                .expect("the index only ever points at a live slot");

            return Some(std::mem::replace(existing, value));
        }

        let id = self.next_id;
        self.next_id += 1;

        self.index.insert(key.clone(), self.slots.len());
        self.slots.push(Slot {
            id,
            entry: Some((key, value)),
        });
        self.live += 1;

        None
    }

    /// `Map.prototype.delete`, returning the value removed.
    ///
    /// O(1): the slot is tombstoned, not removed. Amortised O(1) including the
    /// compaction that eventually reclaims the tombstones.
    pub fn delete(&mut self, key: &K) -> Option<V> {
        let slot = self.index.remove(key)?;
        let (_, value) = self.slots[slot]
            .entry
            .take()
            .expect("the index only ever points at a live slot");

        self.live -= 1;
        self.compact_if_mostly_dead();

        Some(value)
    }

    /// Drop the tombstones once they outnumber the live entries.
    ///
    /// Halving on each pass makes the amortised cost of a `delete` constant.
    /// Entry order and every `id` survive, so `slots` stays sorted and a live
    /// [`MapCursor`] keeps working — it simply misses its index hint once and
    /// falls back to the binary search.
    fn compact_if_mostly_dead(&mut self) {
        if self.slots.len() < COMPACT_FLOOR || self.live * 2 > self.slots.len() {
            return;
        }

        self.slots.retain(|slot| slot.entry.is_some());

        for (position, slot) in self.slots.iter().enumerate() {
            let (key, _) = slot
                .entry
                .as_ref()
                .expect("tombstones were just retained away");

            self.index.insert(key.clone(), position);
        }
    }
}

impl<K: Hash + Eq + Clone, V> FromIterator<(K, V)> for OrderedMap<K, V> {
    /// `new Map(iterable)`: later duplicates overwrite in place, as `set` does.
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut map = Self::new();

        for (key, value) in iter {
            map.set(key, value);
        }

        map
    }
}

/// A live cursor over an [`OrderedMap`], with `Map` iterator semantics.
///
/// Detached by construction: it holds no borrow of the map, so the map stays
/// mutable while the cursor is alive. That is required at the FFI boundary and
/// it is also what makes the interesting behaviours — a delete ahead of the
/// cursor, an append behind it — expressible in a Rust test at all.
///
/// `Copy`, so cloning one forks the walk. JS cannot do that (there is no way
/// to duplicate a `Map` iterator), which makes it a Rust-only affordance
/// rather than a divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapCursor {
    /// Smallest slot id this cursor has not yet consumed.
    next_id: u64,
    /// Physical index where `next_id` was last seen, if nothing moved since.
    ///
    /// Purely an optimisation: it turns the common step into an O(1) check and
    /// a compaction into one missed guess. It is validated on every use, never
    /// trusted.
    hint: usize,
    /// Detached. `Map` iterators do not resume after reporting the end, even
    /// if the map grows afterwards — confirmed against Node 24.18.1.
    done: bool,
}

impl Default for MapCursor {
    fn default() -> Self {
        Self::open()
    }
}

impl MapCursor {
    /// A cursor positioned before the first entry of any map.
    ///
    /// Takes no map: `next_id` 0 is before every id ever issued, so the same
    /// fresh cursor is correct for a map that has been running for a while.
    pub fn open() -> Self {
        Self {
            next_id: 0,
            hint: 0,
            done: false,
        }
    }

    /// Whether this cursor has already reported the end.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// One step against the map **as it is now**.
    ///
    /// `None` is `{done: true}` and is permanent. Anything else is
    /// `{done: false, value: ...}`. There is no third state: unlike
    /// [`crate::cursor::Step`], a `Map` cursor has no frozen length to run
    /// past, so the `undefined` gap of DESIGN.md 3.7 cannot arise here.
    pub fn step<'m, K, V>(&mut self, map: &'m OrderedMap<K, V>) -> Option<(&'m K, &'m V)> {
        if self.done {
            return None;
        }

        let mut position = self.locate(map);

        while let Some(slot) = map.slots.get(position) {
            // Advanced before the liveness test, so a tombstone is scanned at
            // most once per cursor rather than re-found by every later step.
            self.next_id = slot.id + 1;
            self.hint = position + 1;

            if let Some((key, value)) = slot.entry.as_ref() {
                return Some((key, value));
            }

            position += 1;
        }

        self.done = true;

        None
    }

    /// Physical index of the first slot with `id >= self.next_id`.
    ///
    /// The hint is right whenever no compaction has run since the last step,
    /// which is almost always. Validating it costs two comparisons; being
    /// wrong costs a binary search over a vector that is sorted by `id` by
    /// construction.
    fn locate<K, V>(&self, map: &OrderedMap<K, V>) -> usize {
        // `get`, not indexing: a compaction can leave `hint` past the end of a
        // shrunken vector, and an out-of-range hint must be *rejected*, not
        // panicked on.
        let before_is_consumed = self.hint.checked_sub(1).is_none_or(|previous| {
            map.slots
                .get(previous)
                .is_some_and(|slot| slot.id < self.next_id)
        });
        let at_is_pending = map
            .slots
            .get(self.hint)
            .is_none_or(|slot| slot.id >= self.next_id);

        if before_is_consumed && at_is_pending {
            return self.hint;
        }

        map.slots.partition_point(|slot| slot.id < self.next_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_of(pairs: &[(&'static str, u32)]) -> OrderedMap<&'static str, u32> {
        pairs.iter().copied().collect()
    }

    fn drain(map: &OrderedMap<&'static str, u32>) -> Vec<(&'static str, u32)> {
        let mut cursor = MapCursor::open();
        let mut out = Vec::new();

        while let Some((key, value)) = cursor.step(map) {
            out.push((*key, *value));
        }

        out
    }

    #[test]
    fn iterates_in_insertion_order() {
        let map = map_of(&[("a", 1), ("b", 2), ("c", 3)]);

        assert_eq!(drain(&map), vec![("a", 1), ("b", 2), ("c", 3)]);
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn overwriting_a_key_keeps_its_position() {
        let mut map = map_of(&[("a", 1), ("b", 2)]);

        assert_eq!(map.set("a", 9), Some(1));

        assert_eq!(drain(&map), vec![("a", 9), ("b", 2)]);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn deleting_then_reinserting_moves_the_key_to_the_end() {
        let mut map = map_of(&[("a", 1), ("b", 2), ("c", 3)]);

        assert_eq!(map.delete(&"a"), Some(1));
        assert_eq!(map.set("a", 9), None);

        assert_eq!(drain(&map), vec![("b", 2), ("c", 3), ("a", 9)]);
    }

    #[test]
    fn deleting_a_missing_key_reports_it_and_changes_nothing() {
        let mut map = map_of(&[("a", 1)]);

        assert_eq!(map.delete(&"zzz"), None);
        assert_eq!(map.delete(&"a"), Some(1));
        assert_eq!(map.delete(&"a"), None);
        assert!(map.is_empty());
    }

    #[test]
    fn an_append_behind_a_live_cursor_is_visited() {
        let mut map = map_of(&[("a", 1)]);
        let mut cursor = MapCursor::open();

        assert_eq!(cursor.step(&map), Some((&"a", &1)));
        map.set("b", 2);

        assert_eq!(cursor.step(&map), Some((&"b", &2)));
    }

    #[test]
    fn a_delete_ahead_of_a_live_cursor_is_skipped() {
        let mut map = map_of(&[("a", 1), ("b", 2), ("c", 3)]);
        let mut cursor = MapCursor::open();

        assert_eq!(cursor.step(&map), Some((&"a", &1)));
        map.delete(&"b");

        assert_eq!(cursor.step(&map), Some((&"c", &3)));
        assert_eq!(cursor.step(&map), None);
    }

    #[test]
    fn a_delete_behind_a_live_cursor_is_not_revisited() {
        let mut map = map_of(&[("a", 1), ("b", 2)]);
        let mut cursor = MapCursor::open();

        assert_eq!(cursor.step(&map), Some((&"a", &1)));
        map.delete(&"a");

        assert_eq!(cursor.step(&map), Some((&"b", &2)));
    }

    #[test]
    fn a_cursor_that_reported_the_end_stays_done_even_if_the_map_grows() {
        let mut map = map_of(&[("a", 1)]);
        let mut cursor = MapCursor::open();

        assert_eq!(cursor.step(&map), Some((&"a", &1)));
        assert_eq!(cursor.step(&map), None);
        assert!(cursor.is_done());

        map.set("b", 2);

        assert_eq!(cursor.step(&map), None);
    }

    #[test]
    fn clear_then_set_is_visible_to_a_cursor_that_has_not_finished() {
        let mut map = map_of(&[("a", 1), ("b", 2)]);
        let mut cursor = MapCursor::open();

        map.clear();
        map.set("c", 3);

        assert_eq!(cursor.step(&map), Some((&"c", &3)));
    }

    #[test]
    fn clear_then_a_step_detaches_the_cursor_before_the_set() {
        let mut map = map_of(&[("a", 1), ("b", 2)]);
        let mut cursor = MapCursor::open();

        map.clear();

        assert_eq!(cursor.step(&map), None);

        map.set("c", 3);

        assert_eq!(cursor.step(&map), None);
    }

    #[test]
    fn a_cursor_opened_on_a_used_map_starts_at_the_first_live_entry() {
        let mut map = map_of(&[("a", 1), ("b", 2)]);
        map.delete(&"a");
        map.set("c", 3);

        assert_eq!(drain(&map), vec![("b", 2), ("c", 3)]);
    }

    #[test]
    fn compaction_reclaims_tombstones_without_disturbing_order() {
        let mut map: OrderedMap<u32, u32> = (0..16).map(|n| (n, n * 10)).collect();

        for key in 0..12 {
            map.delete(&key);
        }

        assert_eq!(map.len(), 4);
        assert!(
            map.slots.len() < 16,
            "the tombstones should have been reclaimed, slots = {}",
            map.slots.len()
        );
        assert_eq!(
            map.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
            vec![(12, 120), (13, 130), (14, 140), (15, 150)]
        );

        // The index must still point at the right slots after the rebuild.
        for key in 12..16 {
            assert_eq!(map.get(&key), Some(&(key * 10)));
        }
        for key in 0..12 {
            assert_eq!(map.get(&key), None);
        }
    }

    /// The property the whole `id` scheme exists for.
    ///
    /// The deletions are all **behind** the cursor, which is the case that
    /// actually moves the ground under it: compaction shifts every remaining
    /// entry left, so a cursor holding a physical index resumes past the end
    /// and reports `done` with two entries still to yield.
    ///
    /// The first version of this test deleted the entries *ahead* of the
    /// cursor instead. That compacts too — but it removes only slots the
    /// cursor had not reached, so the cursor's index stays accidentally
    /// correct and the test passes against a broken `locate`. Confirmed by
    /// sabotaging `locate` to `return self.hint` unconditionally: the original
    /// stayed green, this one fails.
    #[test]
    fn a_compaction_under_a_live_cursor_does_not_disturb_the_walk() {
        let mut map: OrderedMap<u32, u32> = (0..16).map(|n| (n, n * 10)).collect();
        let mut cursor = MapCursor::open();

        // Walk over the first half.
        for expected in 0..8 {
            assert_eq!(cursor.step(&map), Some((&expected, &(expected * 10))));
        }

        // Delete exactly what the cursor has already passed. Nothing it still
        // owes changes, but every remaining slot moves.
        for key in 0..8 {
            map.delete(&key);
        }
        assert_eq!(map.slots.len(), 8, "expected a compaction to have run");

        for expected in 8..16 {
            assert_eq!(cursor.step(&map), Some((&expected, &(expected * 10))));
        }
        assert_eq!(cursor.step(&map), None);
    }

    /// The other half of the same property: a compaction driven by deletions
    /// *ahead* of the cursor leaves its physical index accidentally valid, so
    /// this cannot falsify `locate` on its own — it is here to pin the
    /// skipping, not the relocation.
    #[test]
    fn a_compaction_ahead_of_a_live_cursor_skips_the_deleted_entries() {
        let mut map: OrderedMap<u32, u32> = (0..16).map(|n| (n, n * 10)).collect();
        let mut cursor = MapCursor::open();

        for expected in 0..4 {
            assert_eq!(cursor.step(&map), Some((&expected, &(expected * 10))));
        }

        for key in 4..14 {
            map.delete(&key);
        }
        assert!(map.slots.len() < 16, "expected a compaction to have run");

        assert_eq!(cursor.step(&map), Some((&14, &140)));
        assert_eq!(cursor.step(&map), Some((&15, &150)));
        assert_eq!(cursor.step(&map), None);
    }

    #[test]
    fn a_compaction_between_two_maps_of_cursors_is_invisible_to_iteration() {
        let mut map: OrderedMap<u32, u32> = (0..40).map(|n| (n, n)).collect();

        for key in (0..40).step_by(2) {
            map.delete(&key);
        }

        let expected: Vec<(u32, u32)> = (1..40).step_by(2).map(|n| (n, n)).collect();
        let walked: Vec<(u32, u32)> = {
            let mut cursor = MapCursor::open();
            let mut out = Vec::new();
            while let Some((key, value)) = cursor.step(&map) {
                out.push((*key, *value));
            }
            out
        };

        assert_eq!(walked, expected);
    }

    #[test]
    fn from_iter_lets_later_duplicates_overwrite_in_place() {
        let map = map_of(&[("a", 1), ("b", 2), ("a", 3)]);

        assert_eq!(drain(&map), vec![("a", 3), ("b", 2)]);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn get_and_contains_key_agree_with_the_walk() {
        let mut map = map_of(&[("a", 1), ("b", 2)]);
        map.delete(&"a");

        assert_eq!(map.get(&"a"), None);
        assert!(!map.contains_key(&"a"));
        assert_eq!(map.get(&"b"), Some(&2));
        assert!(map.contains_key(&"b"));
    }

    #[test]
    fn get_mut_and_values_mut_reach_the_stored_values() {
        let mut map = map_of(&[("a", 1), ("b", 2)]);

        *map.get_mut(&"a").expect("a is present") = 10;
        for value in map.values_mut() {
            *value += 1;
        }

        assert_eq!(drain(&map), vec![("a", 11), ("b", 3)]);
    }

    #[test]
    fn slot_of_and_entry_at_round_trip() {
        let mut map = map_of(&[("a", 1), ("b", 2)]);
        map.delete(&"a");

        let slot = map.slot_of(&"b").expect("b is present");

        assert_eq!(map.entry_at(slot), Some((&"b", &2)));
        assert_eq!(map.entry_at(999), None);
        // The tombstone left by the delete is addressable but not alive.
        assert_eq!(map.entry_at(0), None);
    }

    #[test]
    fn an_empty_map_yields_nothing_and_reports_done_once() {
        let map: OrderedMap<&str, u32> = OrderedMap::new();
        let mut cursor = MapCursor::open();

        assert_eq!(cursor.step(&map), None);
        assert!(cursor.is_done());
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn debug_shows_iteration_order_not_the_representation() {
        let mut map = map_of(&[("a", 1), ("b", 2)]);
        map.delete(&"a");
        map.set("a", 3);

        assert_eq!(format!("{map:?}"), r#"{"b": 2, "a": 3}"#);
    }
}

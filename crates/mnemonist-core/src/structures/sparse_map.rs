//! Port of upstream `sparse-map.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! [`SparseSet`](crate::structures::sparse_set::SparseSet) with a payload: the
//! same `dense`/`sparse` pair over the integers `0..length`, plus a third array
//! `vals` holding one value per occupied `dense` slot. Membership is unchanged
//! — `sparse[m] < size && dense[sparse[m]] == m` — and so is the O(1) `clear`.
//!
//! # `delete` moves the key and leaves the value behind
//!
//! This is the module's headline defect and it needs no out-of-range input to
//! reach. Upstream's `delete` is [`SparseSet`](crate::structures::sparse_set)'s
//! swap-with-last, copied verbatim:
//!
//! ```js
//! index = this.dense[this.size - 1];
//! this.dense[this.sparse[member]] = index;
//! this.sparse[index]              = this.sparse[member];
//! this.size--;
//! ```
//!
//! The last **member** is moved into the hole. The last **value** is not. So
//! the moved member ends up pointing at the deleted member's value:
//!
//! ```text
//! set(3,'a'); set(4,'b'); set(5,'c');  delete(3)
//!   dense [3,4,5] -> [5,4,5]     vals ['a','b','c'] -> UNCHANGED
//!   get(5) == 'a'                                      ^ should be 'c'
//! ```
//!
//! Measured on Node 24.18.1, and reproduced here rather than fixed. See BUG-SPARSE-MAP-1.
//!
//! # Two value stores, not one
//!
//! `new SparseMap(Values, length)` allocates `new Values(length)`, and the
//! upstream test file constructs both `new SparseMap(10)` (implicitly `Array`)
//! and `new SparseMap(Uint8Array, 10)`. The two are **not** interchangeable and
//! the difference is observable through the public API:
//!
//! | | `Array` | `Uint8Array` & friends |
//! |---|---|---|
//! | initial contents | holes, read as `undefined` | zeroes |
//! | store past the end | **grows the array** | silently dropped |
//! | store of `300` | `300` | `44` |
//!
//! [`Values`] is that choice, and the growth column is the interesting one:
//! once an out-of-range `set` has pushed `size` past `length` (BUG-SPARSE-SET-1, inherited
//! wholesale from `SparseSet`), an `Array`-backed map keeps *every* value while
//! `dense` has long since run out of room. `keys()` then yields `undefined`
//! where `values()` still yields real data.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::structures::sparse_map::SparseMap;
//!
//! let mut map = SparseMap::<f64>::array(10).unwrap();
//! map.set(3, 14.0);
//! map.set(4, 22.0);
//! map.set(3, 35.0);
//!
//! assert_eq!(map.size(), 2);
//! assert_eq!(map.get(3), Some(35.0));
//! assert_eq!(map.get(12), None);
//! ```

use crate::cursor::{Cursor, Sequence};
use crate::utils::typed_arrays::{get_pointer_array, PointerVec, PointerWidth, TypedValue};

/// The `Values` constructor a [`SparseMap`] was built with.
///
/// Upstream takes a JavaScript array constructor and calls `new Values(length)`
/// on it. Rust has no runtime constructor value, so the two shapes that
/// constructor can have are an enum instead — and they really are two shapes,
/// not one parameterised by width: a JS `Array` is growable and sparse where a
/// typed array is neither.
#[derive(Debug, Clone, PartialEq)]
pub enum Values<V> {
    /// `new Array(length)` — `length` holes, and a store past the end grows it.
    ///
    /// `None` is a hole, which JS reads as `undefined`. Note that a hole and a
    /// stored `undefined` are distinguishable in JS but not here; nothing in
    /// this module can store `undefined`, because `set` takes a `V`.
    Array(Vec<Option<V>>),
    /// `new Uint8Array(length)` and friends — fixed length, truncating stores.
    Typed(PointerVec),
}

impl<V: TypedValue> Values<V> {
    /// Read one slot, or `None` where JS would have produced `undefined`.
    pub fn slot(&self, index: usize) -> Option<V> {
        match self {
            Self::Array(slots) => slots.get(index).copied().flatten(),
            Self::Typed(slots) => slots.try_get(index).map(V::from_uint32),
        }
    }

    /// `vals[index] = value`, with each store's own idea of what that means.
    ///
    /// The asymmetry is the whole reason this is an enum. A JS array store past
    /// the end **extends** the array, filling the skipped positions with holes;
    /// a typed-array store past the end is a no-op. Both are reachable from
    /// `set`, because `size` can run past `length`.
    pub fn store(&mut self, index: usize, value: V) {
        match self {
            Self::Array(slots) => {
                if index >= slots.len() {
                    slots.resize(index + 1, None);
                }

                slots[index] = Some(value);
            }
            Self::Typed(slots) => {
                slots.try_set(index, value.to_uint32());
            }
        }
    }

    /// Slots currently allocated — `vals.length`, which for the `Array` form is
    /// not necessarily the map's `length`.
    pub fn len(&self) -> usize {
        match self {
            Self::Array(slots) => slots.len(),
            Self::Typed(slots) => slots.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Which of the three walks a cursor over a [`SparseMap`] is performing.
///
/// Upstream's `keys`, `values` and `entries` are three copies of one closure
/// over one frozen `size`, differing only in what they read out of slot `i`.
/// Carried in [`Sequence::Frozen`] rather than expressed as three `Sequence`
/// impls, because a type may implement a trait once; see
/// [`crate::cursor::CursorState::open_projected`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// `keys()` — `dense[i]`.
    Keys,
    /// `values()` — `vals[i]`.
    Values,
    /// `entries()` — `[dense[i], vals[i]]`, and also `Symbol.iterator`.
    Entries,
}

/// One step of a [`SparseMap`] walk, shaped by the [`Projection`].
///
/// `Entry` carries two [`Option`]s rather than producing a
/// [`Gap`](crate::cursor::Step::Gap): upstream's `entries()` builds the pair
/// `[dense[i], vals[i]]` and yields **the array**, so the iterator itself never
/// produces `undefined` even when both halves are. `keys()` and `values()` read
/// a single slot and therefore can.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projected<V> {
    Key(u32),
    Value(V),
    Entry(Option<u32>, Option<V>),
}

impl<V> Projected<V> {
    /// The key, if this step came from a `keys()` walk.
    pub fn key(self) -> Option<u32> {
        match self {
            Self::Key(key) => Some(key),
            Self::Value(_) | Self::Entry(..) => None,
        }
    }

    /// The value, if this step came from a `values()` walk.
    pub fn value(self) -> Option<V> {
        match self {
            Self::Value(value) => Some(value),
            Self::Key(_) | Self::Entry(..) => None,
        }
    }

    /// The pair, if this step came from an `entries()` walk. Either half may be
    /// `None`, which is JS `undefined` *inside* the yielded array.
    pub fn entry(self) -> Option<(Option<u32>, Option<V>)> {
        match self {
            Self::Entry(key, value) => Some((key, value)),
            Self::Key(_) | Self::Value(_) => None,
        }
    }
}

/// A map from the members `0..length` to values of type `V`.
#[derive(Debug, Clone)]
pub struct SparseMap<V> {
    length: usize,
    size: usize,
    dense: PointerVec,
    sparse: PointerVec,
    vals: Values<V>,
}

impl<V: TypedValue> SparseMap<V> {
    /// Upstream's `new SparseMap(length)` — the default `Array` value store.
    ///
    /// # Errors
    ///
    /// [`crate::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE`] when `length`
    /// exceeds what a 32-bit pointer array can index, which is where upstream
    /// throws.
    pub fn array(length: usize) -> Result<Self, &'static str> {
        // Validate before allocating anything, not after: upstream reaches its
        // `throw` inside `getPointerArray` before `new Values(length)` runs, so
        // an over-large length must not allocate a value store on the way to
        // the same error. It is also the difference between an `Err` and a
        // 34 GB allocation abort.
        get_pointer_array(length as f64)?;

        Self::new(length, Values::Array(vec![None; length]))
    }

    /// Upstream's `new SparseMap(Uint8Array, length)`, and the wider variants.
    ///
    /// # Errors
    ///
    /// As [`SparseMap::array`]. Note that the *value* width is chosen by the
    /// caller and the *index* width by `length`, exactly as upstream: the two
    /// are independent, and `new SparseMap(Uint8Array, 1000)` gives 16-bit
    /// indices over 8-bit values.
    pub fn typed(length: usize, values: PointerWidth) -> Result<Self, &'static str> {
        get_pointer_array(length as f64)?;

        Self::new(length, Values::Typed(PointerVec::zeroed(values, length)))
    }

    /// Build a map with an already-allocated value store.
    ///
    /// # Errors
    ///
    /// As [`SparseMap::array`].
    ///
    /// # Panics
    ///
    /// A `length` that passes validation but is too large to allocate aborts
    /// through the global allocator; stable Rust has no fallible `Vec`
    /// allocation. Same treatment as
    /// [`SparseSet::new`](crate::structures::sparse_set::SparseSet::new).
    pub fn new(length: usize, vals: Values<V>) -> Result<Self, &'static str> {
        // One width for `dense` and `sparse` both, exactly as upstream's
        // single `getPointerArray(length)` call. `vals` is unrelated to it.
        let width = get_pointer_array(length as f64)?;

        Ok(Self {
            length,
            size: 0,
            dense: PointerVec::zeroed(width, length),
            sparse: PointerVec::zeroed(width, length),
            vals,
        })
    }

    /// Entries currently in the map.
    ///
    /// Can exceed [`SparseMap::length`] after an out-of-range
    /// [`set`](SparseMap::set); the mechanism is `SparseSet`'s BUG-SPARSE-SET-1 unchanged.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Capacity the map was built with — upstream's `length` property.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Members in insertion order, as the backing array holds them.
    pub fn dense(&self) -> &PointerVec {
        &self.dense
    }

    /// Member-to-slot index, upstream's `sparse` property.
    pub fn sparse(&self) -> &PointerVec {
        &self.sparse
    }

    /// The value store, upstream's `vals` property.
    ///
    /// Public because it is public upstream, and because the differential
    /// fuzzer compares it slot for slot: `delete` deliberately leaves it
    /// untouched (BUG-SPARSE-MAP-1), and only reading it directly distinguishes "the port
    /// reproduced the stale value" from "the port happened to agree on `get`".
    pub fn vals(&self) -> &Values<V> {
        &self.vals
    }

    /// Empty the map in O(1). Nothing is cleared but `size`.
    pub fn clear(&mut self) {
        self.size = 0;
    }

    /// Whether `member` has a value in the map.
    pub fn has(&self, member: usize) -> bool {
        // `undefined` past the end of `sparse`, and `undefined < this.size` is
        // false, so an out-of-range member is reported absent.
        let Some(index) = self.sparse.try_get(member) else {
            return false;
        };

        (index as usize) < self.size && self.stored_at(index as usize) == Some(member)
    }

    /// The value associated with `member`, or `None` for JS `undefined`.
    ///
    /// Two distinct routes to `None`, both upstream's: the member is absent, or
    /// the member is present but its `vals` slot is past the end of the value
    /// store — which a typed store reaches as soon as `size > length`.
    pub fn get(&self, member: usize) -> Option<V> {
        let index = self.sparse.try_get(member)? as usize;

        if index < self.size && self.stored_at(index) == Some(member) {
            return self.vals.slot(index);
        }

        None
    }

    /// Associate `value` with `member`, returning whether the member is new.
    ///
    /// Upstream returns `this` for chaining and exposes the answer only through
    /// `size`; the bridge drops this bool so the JS surface matches.
    ///
    /// Out of range this corrupts the map rather than failing, inheriting BUG-SPARSE-SET-1
    /// from `SparseSet` intact — with one addition of its own: the `vals` store
    /// still receives the value, and for the `Array` form it *grows* to hold it
    /// while `dense` cannot.
    pub fn set(&mut self, member: usize, value: V) -> bool {
        if let Some(index) = self.sparse.try_get(member) {
            let index = index as usize;

            if index < self.size && self.stored_at(index) == Some(member) {
                // The in-place update. `index < size` is not enough on its own
                // to know the slot exists in `vals` — a typed store past its
                // end drops this write, exactly as upstream does.
                self.vals.store(index, value);

                return false;
            }
        }

        self.dense.try_set(self.size, member as u32);
        self.sparse.try_set(member, self.size as u32);
        self.vals.store(self.size, value);
        self.size += 1;

        true
    }

    /// Remove `member`, returning whether it was there.
    ///
    /// **Does not move the value.** That is BUG-SPARSE-MAP-1 and it is upstream's, not a
    /// simplification here — see the module docs.
    pub fn delete(&mut self, member: usize) -> bool {
        let Some(slot) = self.sparse.try_get(member) else {
            return false;
        };
        let slot = slot as usize;

        if slot >= self.size || self.stored_at(slot) != Some(member) {
            return false;
        }

        // The `SparseSet` swap, including the asymmetry of BUG-SPARSE-SET-3 once `size` has
        // run past `length`: the `dense` store still lands (a `NaN` element
        // store is `0`) while `sparse[undefined]` becomes a string-keyed
        // expando that leaves the array alone.
        let last = self.dense.try_get(self.size - 1);

        self.dense.try_set(slot, last.unwrap_or(0));

        if let Some(last) = last {
            self.sparse.try_set(last as usize, slot as u32);
        }

        // And `vals` is untouched. Deliberately. BUG-SPARSE-MAP-1.
        self.size -= 1;

        true
    }

    /// A cursor over the members, in `dense` order — upstream's `keys()`.
    ///
    /// Freezes `size` and reads `dense` live. Yields
    /// [`Step::Gap`](crate::cursor::Step::Gap) once `size` has been pushed past
    /// `length`, because `dense` has no slot to supply.
    pub fn keys(&self) -> Cursor<'_, Self> {
        Cursor::projected(self, Projection::Keys)
    }

    /// A cursor over the values — upstream's `values()`.
    ///
    /// Gaps where the *value store* has no slot, which for an `Array` store is
    /// a different set of ordinals from [`keys`](SparseMap::keys): the array
    /// grew and `dense` did not.
    pub fn values(&self) -> Cursor<'_, Self> {
        Cursor::projected(self, Projection::Values)
    }

    /// A cursor over `[key, value]` pairs — upstream's `entries()`, and the
    /// method its `Symbol.iterator` is aliased to.
    ///
    /// Never gaps: upstream builds the pair and yields the array, so a missing
    /// half is `undefined` *inside* a yielded value rather than a yielded
    /// `undefined`.
    pub fn entries(&self) -> Cursor<'_, Self> {
        Cursor::new(self)
    }

    /// `this.dense[slot]`, widened back to a member.
    fn stored_at(&self, slot: usize) -> Option<usize> {
        self.dense.try_get(slot).map(|member| member as usize)
    }
}

/// The three walks, all freezing `size` and reading both arrays live.
impl<V: TypedValue> Sequence for SparseMap<V> {
    type Item = Projected<V>;
    /// Which walk. `entries` is the default because that is what upstream
    /// aliases `Symbol.iterator` to.
    type Frozen = Projection;

    fn freeze(&self) -> (Projection, usize) {
        // `var size = this.size` in all three upstream closures.
        (Projection::Entries, self.size)
    }

    fn slot(&self, frozen: &Projection, ordinal: usize) -> Option<Projected<V>> {
        match frozen {
            Projection::Keys => self.dense.try_get(ordinal).map(Projected::Key),
            Projection::Values => self.vals.slot(ordinal).map(Projected::Value),
            Projection::Entries => Some(Projected::Entry(
                self.dense.try_get(ordinal),
                self.vals.slot(ordinal),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{CursorState, Step};
    use crate::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE;

    fn keys<V: TypedValue>(map: &SparseMap<V>) -> Vec<u32> {
        map.keys().filter_map(Projected::key).collect()
    }

    fn values<V: TypedValue>(map: &SparseMap<V>) -> Vec<V> {
        map.values().filter_map(Projected::value).collect()
    }

    fn entries<V: TypedValue>(map: &SparseMap<V>) -> Vec<(Option<u32>, Option<V>)> {
        map.entries().filter_map(Projected::entry).collect()
    }

    /// 1:1 port of all nine upstream `it` blocks, as a baseline.
    #[test]
    fn reproduces_the_upstream_suite() {
        let mut map = SparseMap::<f64>::array(10).unwrap();
        map.set(3, 14.0);
        map.set(4, 22.0);
        map.set(3, 35.0);
        assert_eq!(map.size(), 2);
        assert_eq!(map.length(), 10);

        assert!(map.has(3));
        assert!(!map.has(1));
        assert!(!map.has(12));
        assert_eq!(map.get(3), Some(35.0));
        assert_eq!(map.get(4), Some(22.0));
        assert_eq!(map.get(12), None);

        // …and the same again with a value array constructor.
        let mut typed = SparseMap::<f64>::typed(10, PointerWidth::U8).unwrap();
        typed.set(3, 14.0);
        typed.set(4, 22.0);
        typed.set(3, 35.0);
        assert!(typed.has(3));
        assert!(!typed.has(1));
        assert!(!typed.has(12));
        assert_eq!(typed.get(3), Some(35.0));
        assert_eq!(typed.get(4), Some(22.0));
        assert_eq!(typed.get(12), None);

        let mut map = SparseMap::<f64>::array(10).unwrap();
        map.set(3, 14.0);
        map.delete(3);
        map.delete(4);
        assert_eq!(map.size(), 0);
        assert!(!map.has(3) && !map.has(4));
        assert_eq!(map.get(3), None);
        assert_eq!(map.get(4), None);
        map.set(2, 35.0);
        assert_eq!(map.size(), 1);
        assert_eq!(map.get(2), Some(35.0));
        map.set(3, 28.0);
        assert_eq!(map.size(), 2);
        assert_eq!(map.get(3), Some(28.0));

        let mut map = SparseMap::<f64>::array(10).unwrap();
        for member in 0..6 {
            map.set(member, member as f64 + 1.0);
        }
        assert_eq!(map.size(), 6);
        assert_eq!(map.get(3), Some(4.0));
        map.clear();
        assert_eq!(map.size(), 0);
        assert!(!map.has(3));
        assert_eq!(map.get(3), None);

        let mut map = SparseMap::<f64>::array(10).unwrap();
        map.set(3, 13.0);
        map.set(6, 22.0);
        map.set(9, 8.0);
        assert_eq!(keys(&map), vec![3, 6, 9]);
        assert_eq!(values(&map), vec![13.0, 22.0, 8.0]);
        assert_eq!(
            entries(&map),
            vec![
                (Some(3), Some(13.0)),
                (Some(6), Some(22.0)),
                (Some(9), Some(8.0)),
            ]
        );
    }

    /// **BUG-SPARSE-MAP-1.** `delete` swaps the last *member* into the hole and leaves the
    /// last *value* where it was, so the moved member inherits the deleted
    /// member's value.
    ///
    /// Verified against Node 24.18.1: `set(3,'a') set(4,'b') set(5,'c')` then
    /// `delete(3)` leaves `dense = [5,4,5]`, `vals = ['a','b','c']` and
    /// `get(5) === 'a'`.
    ///
    /// Upstream's suite cannot see this: it only ever deletes from a
    /// one-element map, where the swap is a self-assignment.
    #[test]
    fn delete_moves_the_key_but_not_the_value() {
        let mut map = SparseMap::<u32>::array(10).unwrap();

        map.set(3, 100);
        map.set(4, 200);
        map.set(5, 300);

        assert!(map.delete(3));

        assert_eq!(map.size(), 2);
        assert_eq!(map.dense().try_get(0), Some(5));
        // The value array is byte-for-byte what it was.
        assert_eq!(
            map.vals(),
            &Values::Array(vec![
                Some(100),
                Some(200),
                Some(300),
                None,
                None,
                None,
                None,
                None,
                None,
                None
            ])
        );
        // So member 5 now answers with member 3's value.
        assert_eq!(map.get(5), Some(100));
        assert_eq!(map.get(4), Some(200));
        assert_eq!(
            entries(&map),
            vec![(Some(5), Some(100)), (Some(4), Some(200))]
        );
    }

    /// The same defect through a typed value store, to show it is the swap and
    /// not the `Array`. Node gives `vals = [11,22,33,…]` and `get(5) === 11`.
    #[test]
    fn the_stale_value_is_not_an_artefact_of_the_array_store() {
        let mut map = SparseMap::<u32>::typed(10, PointerWidth::U8).unwrap();

        map.set(3, 11);
        map.set(4, 22);
        map.set(5, 33);
        map.delete(3);

        assert_eq!(map.get(5), Some(11));
        assert_eq!(map.vals().slot(2), Some(33));
    }

    /// Deleting the *last* entry is the self-swap, and there the missing value
    /// move happens to be invisible — which is exactly the case upstream tests.
    #[test]
    fn deleting_the_last_entry_hides_the_defect() {
        let mut map = SparseMap::<u32>::array(5).unwrap();

        map.set(0, 10);
        map.set(1, 11);

        assert!(map.delete(1));
        assert_eq!(entries(&map), vec![(Some(0), Some(10))]);
        assert!(map.delete(0));
        assert_eq!(map.size(), 0);
        assert!(!map.delete(0));
    }

    /// A re-`set` of a present member overwrites in place: no new slot, no
    /// change to either index array.
    #[test]
    fn setting_a_present_member_overwrites_in_place() {
        let mut map = SparseMap::<u32>::array(10).unwrap();

        assert!(map.set(3, 1));
        let before = map.clone();

        assert!(!map.set(3, 2));
        assert_eq!(map.dense(), before.dense());
        assert_eq!(map.sparse(), before.sparse());
        assert_eq!(map.size(), 1);
        assert_eq!(map.get(3), Some(2));
    }

    /// `clear` is O(1) and leaves live-looking debris in all three arrays;
    /// re-using the map afterwards must still work.
    #[test]
    fn clear_leaves_stale_entries_that_stay_unreachable() {
        let mut map = SparseMap::<u32>::array(5).unwrap();

        map.set(2, 20);
        map.set(4, 40);
        map.clear();

        assert_eq!(map.size(), 0);
        assert!(!map.has(2) && !map.has(4));
        assert_eq!(map.get(2), None);
        // The debris is still there, in both dense and vals.
        assert_eq!(map.dense().try_get(0), Some(2));
        assert_eq!(map.vals().slot(0), Some(20));

        map.set(4, 41);
        assert_eq!(map.size(), 1);
        assert_eq!(map.get(4), Some(41));
        assert!(!map.has(2));
        assert_eq!(entries(&map), vec![(Some(4), Some(41))]);
    }

    /// `has`, `get` and `delete` are all safe out of range; only `set` is not.
    #[test]
    fn reads_out_of_range_report_absence() {
        let mut map = SparseMap::<u32>::array(10).unwrap();

        map.set(3, 1);

        assert!(!map.has(10) && !map.has(300) && !map.has(usize::MAX));
        assert_eq!(map.get(10), None);
        assert_eq!(map.get(300), None);
        assert!(!map.delete(10) && !map.delete(300));
        assert_eq!(map.size(), 1);
    }

    /// BUG-SPARSE-SET-1, inherited from `SparseSet` intact — plus the value store, which
    /// still receives the write.
    ///
    /// Verified against Node: `new SparseMap(10)` then `set(300, 7)` gives
    /// `size === 1`, `dense === [44, 0, …]`, `sparse` untouched,
    /// `vals === [7, <9 holes>]`, and `has(300) === has(44) === false`.
    #[test]
    fn an_out_of_range_set_corrupts_the_map_exactly_as_upstream_does() {
        let mut map = SparseMap::<u32>::array(10).unwrap();

        assert!(map.set(300, 7));

        assert_eq!(map.size(), 1);
        assert_eq!(map.dense().try_get(0), Some(300 % 256));
        assert_eq!(map.sparse(), &PointerVec::zeroed(PointerWidth::U8, 10));
        // The value landed even though the key did not: slot 0 was in range.
        assert_eq!(map.vals().slot(0), Some(7));
        assert!(!map.has(300) && !map.has(44));
        assert_eq!(map.get(300), None);
        // Iterable, though, because iteration reads the arrays directly.
        assert_eq!(entries(&map), vec![(Some(44), Some(7))]);
    }

    /// The `Array` store **grows** past the map's length while `dense` cannot,
    /// so `keys()` gaps where `values()` still has real data.
    ///
    /// Verified against Node: `new SparseMap(2)`, `set(100…103)` gives
    /// `dense = [100, 101]`, `vals = [1, 2, 3, 4]` with `vals.length === 4`,
    /// `keys → [100, 101, undefined, undefined]`, `values → [1, 2, 3, 4]`,
    /// `entries → [[100,1], [101,2], [undefined,3], [undefined,4]]`.
    #[test]
    fn an_array_value_store_outgrows_the_map_it_belongs_to() {
        let mut map = SparseMap::<u32>::array(2).unwrap();

        for (member, value) in [(100, 1), (101, 2), (102, 3), (103, 4)] {
            map.set(member, value);
        }

        assert_eq!((map.size(), map.length()), (4, 2));
        assert_eq!(map.dense().len(), 2);
        assert_eq!(map.vals().len(), 4);

        let mut walk = map.keys();
        assert_eq!(walk.step(), Step::Item(Projected::Key(100)));
        assert_eq!(walk.step(), Step::Item(Projected::Key(101)));
        assert_eq!(walk.step(), Step::Gap);
        assert_eq!(walk.step(), Step::Gap);
        assert_eq!(walk.step(), Step::Done);

        // No gaps at all on the value side: the array grew to fit.
        assert_eq!(values(&map), vec![1, 2, 3, 4]);
        assert_eq!(
            entries(&map),
            vec![
                (Some(100), Some(1)),
                (Some(101), Some(2)),
                (None, Some(3)),
                (None, Some(4)),
            ]
        );
    }

    /// A typed value store cannot grow, so the write is dropped and both sides
    /// gap together. Node: `dense = [100,101]`, `vals = [1,2]`,
    /// `[...map] → [[100,1], [101,2], [undefined,undefined]]`.
    #[test]
    fn a_typed_value_store_drops_the_write_and_gaps_with_the_keys() {
        let mut map = SparseMap::<u32>::typed(2, PointerWidth::U8).unwrap();

        for (member, value) in [(100, 1), (101, 2), (102, 3)] {
            map.set(member, value);
        }

        assert_eq!(map.size(), 3);
        assert_eq!(map.vals().len(), 2);
        assert_eq!(map.vals(), &Values::Typed(PointerVec::U8(vec![1, 2])));

        assert_eq!(keys(&map), vec![100, 101]);
        assert_eq!(values(&map), vec![1, 2]);
        assert_eq!(
            entries(&map),
            vec![(Some(100), Some(1)), (Some(101), Some(2)), (None, None)]
        );
    }

    /// Values narrow to the store's width on the way in, exactly as a JS typed
    /// array element store does.
    #[test]
    fn typed_values_truncate_at_their_own_width() {
        let mut narrow = SparseMap::<u32>::typed(4, PointerWidth::U8).unwrap();
        narrow.set(0, 300);
        assert_eq!(narrow.get(0), Some(300 % 256));

        let mut wide = SparseMap::<u32>::typed(4, PointerWidth::U16).unwrap();
        wide.set(0, 70_000);
        assert_eq!(wide.get(0), Some(70_000 % 65_536));

        // The value width is chosen independently of the index width.
        let mixed = SparseMap::<u32>::typed(1_000, PointerWidth::U8).unwrap();
        assert_eq!(mixed.dense().width(), PointerWidth::U16);
        assert_eq!(
            mixed.vals(),
            &Values::Typed(PointerVec::zeroed(PointerWidth::U8, 1_000))
        );

        // An Array store keeps what it is given.
        let mut plain = SparseMap::<u32>::array(4).unwrap();
        plain.set(0, 300);
        assert_eq!(plain.get(0), Some(300));
    }

    /// BUG-SPARSE-SET-3, on this module's arrays. Node gives `dense = [0, 0, 2]`,
    /// `sparse = [0, 1, 2]`, `sparse.undefined = 1`, and `vals` unchanged.
    #[test]
    fn a_delete_past_capacity_writes_dense_but_not_sparse() {
        let mut map = SparseMap::<u32>::array(3).unwrap();

        for (member, value) in [(0, 10), (1, 11), (2, 12), (99, 99)] {
            map.set(member, value);
        }

        assert_eq!(map.size(), 4);
        assert!(map.delete(1));

        assert_eq!(map.size(), 3);
        assert_eq!(map.dense(), &PointerVec::U8(vec![0, 0, 2]));
        // Not `[1, 1, 2]`, which is what writing `sparse[0]` would give.
        assert_eq!(map.sparse(), &PointerVec::U8(vec![0, 1, 2]));
        assert_eq!(
            map.vals(),
            &Values::Array(vec![Some(10), Some(11), Some(12), Some(99)])
        );
    }

    /// DIV-STACK-1 / DIV-STACK-2 seen from Rust: each cursor is exhausted once, but the map
    /// can be walked again — and the three walks are independent.
    #[test]
    fn cursors_do_not_restart_but_the_map_can_be_walked_again() {
        let mut map = SparseMap::<u32>::array(10).unwrap();

        map.set(3, 30);
        map.set(6, 60);

        let mut cursor = map.keys();
        assert_eq!(cursor.by_ref().count(), 2);
        assert_eq!(cursor.count(), 0);

        assert_eq!(keys(&map), vec![3, 6]);
        assert_eq!(keys(&map), vec![3, 6]);
        assert_eq!(values(&map), vec![30, 60]);
        assert_eq!(entries(&map).len(), 2);
    }

    /// DIV-PROJ-10 on this module's data: a `delete` between two steps is visible,
    /// because both arrays are read lazily — and BUG-SPARSE-MAP-1 makes the visible result
    /// a *mismatched pair*, which is the sharpest possible demonstration of it.
    ///
    /// Measured against Node with the equivalent JS.
    #[test]
    fn a_delete_during_iteration_is_visible_and_desynchronises_the_pair() {
        let mut map = SparseMap::<u32>::array(10).unwrap();

        for (member, value) in [(1, 10), (2, 20), (3, 30)] {
            map.set(member, value);
        }

        let mut state = CursorState::open(&map);

        assert_eq!(
            state.step(&map),
            Step::Item(Projected::Entry(Some(1), Some(10)))
        );

        map.delete(2);

        // 3 was swapped into slot 1; its value was not. The frozen size keeps
        // the walk going past the map's new end, so slot 2 is still read.
        assert_eq!(
            state.step(&map),
            Step::Item(Projected::Entry(Some(3), Some(20)))
        );
        assert_eq!(
            state.step(&map),
            Step::Item(Projected::Entry(Some(3), Some(30)))
        );
        assert_eq!(state.step(&map), Step::Done);
    }

    /// Growth is not visible: `size` is frozen at creation.
    #[test]
    fn a_set_during_iteration_is_not_visible_to_the_cursor() {
        let mut map = SparseMap::<u32>::array(10).unwrap();

        map.set(1, 10);

        let mut state = CursorState::open(&map);

        map.set(2, 20);
        map.set(3, 30);

        assert_eq!(
            state.step(&map),
            Step::Item(Projected::Entry(Some(1), Some(10)))
        );
        assert_eq!(state.step(&map), Step::Done);
    }

    /// Gap 9's analogue: upstream's only tested length is 10, so the 16- and
    /// 32-bit index branches are never reached through this module.
    #[test]
    fn picks_one_pointer_width_for_both_index_arrays() {
        for (length, expected) in [
            (0usize, PointerWidth::U8),
            (256, PointerWidth::U8),
            (257, PointerWidth::U16),
            (65_536, PointerWidth::U16),
            (65_537, PointerWidth::U32),
        ] {
            let map = SparseMap::<u32>::array(length).unwrap();

            assert_eq!(map.dense().width(), expected, "length {length}");
            assert_eq!(map.sparse().width(), expected, "length {length}");
            assert_eq!(map.vals().len(), length);
        }
    }

    #[test]
    fn rejects_a_length_no_pointer_array_can_index() {
        assert_eq!(
            SparseMap::<u32>::array(4_294_967_297).unwrap_err(),
            POINTER_ARRAY_TOO_LARGE
        );
        assert_eq!(
            SparseMap::<u32>::typed(4_294_967_297, PointerWidth::U8).unwrap_err(),
            POINTER_ARRAY_TOO_LARGE
        );
    }

    /// The degenerate length. Every member is out of range, so nothing is ever
    /// findable — but the `Array` value store still grows one slot per `set`.
    /// Node: `size === 1`, `vals === [9]`, `[...map] → [[undefined, 9]]`.
    #[test]
    fn a_zero_length_map_finds_nothing_but_still_accumulates_values() {
        let mut map = SparseMap::<u32>::array(0).unwrap();

        assert!(!map.has(0));
        assert_eq!(map.get(0), None);
        assert!(!map.delete(0));

        map.set(0, 9);

        assert_eq!(map.size(), 1);
        assert!(!map.has(0));
        assert_eq!(map.vals(), &Values::Array(vec![Some(9)]));
        assert_eq!(map.keys().step(), Step::Gap);
        assert_eq!(entries(&map), vec![(None, Some(9))]);

        // A typed store of length zero drops the write instead.
        let mut typed = SparseMap::<u32>::typed(0, PointerWidth::U8).unwrap();
        typed.set(0, 9);
        assert_eq!(typed.size(), 1);
        assert!(typed.vals().is_empty());
        assert_eq!(entries(&typed), vec![(None, None)]);
    }

    /// Filling to capacity: every slot used, no truncation anywhere, and `size`
    /// lands exactly on `length`.
    #[test]
    fn fills_to_capacity_without_running_off_the_end() {
        let mut map = SparseMap::<u32>::array(300).unwrap();

        for member in 0..300 {
            assert!(map.set(member, member as u32 * 2));
        }

        assert_eq!(map.size(), 300);
        assert_eq!(map.dense().width(), PointerWidth::U16);
        assert_eq!(map.get(299), Some(598));
        assert_eq!(keys(&map), (0..300u32).collect::<Vec<_>>());
        assert_eq!(values(&map), (0..300u32).map(|m| m * 2).collect::<Vec<_>>());
    }

    /// The projection accessors, so a mis-projected step is a `None` rather
    /// than a silently plausible value.
    #[test]
    fn projections_do_not_answer_for_each_other() {
        assert_eq!(Projected::<u32>::Key(1).key(), Some(1));
        assert_eq!(Projected::<u32>::Key(1).value(), None);
        assert_eq!(Projected::<u32>::Key(1).entry(), None);
        assert_eq!(Projected::Value(2u32).value(), Some(2));
        assert_eq!(Projected::Value(2u32).key(), None);
        assert_eq!(
            Projected::Entry(Some(1), Some(2u32)).entry(),
            Some((Some(1), Some(2)))
        );
        assert_eq!(Projected::Entry(Some(1), Some(2u32)).key(), None);
    }
}

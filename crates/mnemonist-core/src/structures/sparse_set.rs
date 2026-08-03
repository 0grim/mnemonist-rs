//! Port of upstream `sparse-set.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! A sparse set over the integers `0..length`, in the classic two-array form
//! ([research.swtch.com/sparse](https://research.swtch.com/sparse)): `dense`
//! holds the members in insertion order, `sparse` maps a member to its slot in
//! `dense`, and membership is `sparse[m] < size && dense[sparse[m]] == m`.
//! Nothing needs clearing, so `clear` is `size = 0`.
//!
//! # Out-of-range members are the interesting part
//!
//! Upstream never validates `member`, and the three methods react differently
//! because JavaScript's `undefined` compares false against everything:
//!
//! | call, with `member >= length` | upstream | why |
//! |---|---|---|
//! | `has(m)` | `false` | `undefined < size` is false |
//! | `delete(m)` | `false` | `undefined >= size` is *also* false, then `dense[undefined] !== m` is true |
//! | `add(m)` | **corrupts the set** | neither guard fires, so it writes and bumps `size` anyway |
//!
//! That last row is reproduced here rather than rejected, and it is worth
//! spelling out because two upstream bugs stack in it:
//!
//! 1. `this.dense[this.size] = member` is a **truncating** typed-array store.
//!    On a set of length 10 (`Uint8Array`), `add(300)` stores `44`.
//! 2. `this.sparse[member] = this.size` is an out-of-range store, which JS
//!    silently drops. So the member is unfindable: `has` says `false` for both
//!    `300` and `44` immediately after.
//! 3. `this.size++` happens regardless, so **`size` can exceed `length`** —
//!    and then `values()` freezes a length past the end of `dense` and yields
//!    genuine `undefined` values. See [`SparseSet::values`].
//!
//! This differs from the approach taken in
//! [`crate::structures::static_disjoint_set`], where out-of-range indices
//! raise at the bridge instead. The difference is not a change of mind: there,
//! upstream propagates `NaN` through arithmetic and no honest Rust
//! reproduction exists. Here every step is a well-defined read, store or
//! dropped store, so the faithful port *is* expressible, and reproducing it
//! bug-for-bug is both cheaper and more useful than guarding it.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::structures::sparse_set::SparseSet;
//!
//! let mut set = SparseSet::new(10).unwrap();
//! set.add(3);
//! set.add(6);
//! set.add(3);
//!
//! assert_eq!(set.size(), 2);
//! assert!(set.has(3));
//! assert!(set.delete(3));
//! assert!(!set.delete(3));
//! assert_eq!(set.values().collect::<Vec<_>>(), vec![6]);
//! ```

use crate::cursor::{Cursor, Sequence};
use crate::utils::typed_arrays::{get_pointer_array, PointerVec};

/// A sparse set over the members `0..length`.
#[derive(Debug, Clone)]
pub struct SparseSet {
    length: usize,
    size: usize,
    dense: PointerVec,
    sparse: PointerVec,
}

impl SparseSet {
    /// Build an empty set able to hold the members `0..length`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE`] when
    /// `length` exceeds what a 32-bit pointer array can index, which is where
    /// upstream throws.
    ///
    /// # Panics
    ///
    /// A `length` that passes validation but is too large to allocate aborts
    /// through the global allocator, exactly as in
    /// [`crate::structures::static_disjoint_set::StaticDisjointSet::new`] and
    /// for the same reason: stable Rust has no fallible `Vec` allocation.
    pub fn new(length: usize) -> Result<Self, &'static str> {
        // One width for both arrays, unlike StaticDisjointSet: upstream picks
        // `getPointerArray(length)` once and uses it for `dense` and `sparse`
        // alike. `dense` holds members and `sparse` holds slots, and both are
        // bounded by `length`, so one width genuinely serves.
        let width = get_pointer_array(length as f64)?;

        Ok(Self {
            length,
            size: 0,
            dense: PointerVec::zeroed(width, length),
            sparse: PointerVec::zeroed(width, length),
        })
    }

    /// Number of members currently in the set.
    ///
    /// Can exceed [`SparseSet::length`] after an out-of-range
    /// [`add`](SparseSet::add); see the module docs.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Capacity the set was built with — upstream's `length` property.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Members in insertion order, as the backing array holds them.
    ///
    /// Exposed because it is a public property upstream and because the
    /// differential fuzzer compares it slot for slot: agreeing on membership
    /// while disagreeing on the internal layout would mean the swap-with-last
    /// in [`delete`](SparseSet::delete) had drifted, and only iteration order
    /// would eventually show it.
    pub fn dense(&self) -> &PointerVec {
        &self.dense
    }

    /// Member-to-slot index, upstream's `sparse` property.
    pub fn sparse(&self) -> &PointerVec {
        &self.sparse
    }

    /// Empty the set in O(1).
    ///
    /// Neither array is touched — stale entries are unreachable because every
    /// lookup is gated on `< size` first. That is the whole point of the
    /// structure, and it means [`clear`](SparseSet::clear) leaves observable
    /// debris in [`dense`](SparseSet::dense), exactly as upstream does.
    pub fn clear(&mut self) {
        self.size = 0;
    }

    /// Whether `member` is in the set.
    pub fn has(&self, member: usize) -> bool {
        // `var index = this.sparse[member];` — `undefined` past the end, and
        // `undefined < this.size` is false, so an out-of-range member is
        // simply reported absent.
        let Some(index) = self.sparse.try_get(member) else {
            return false;
        };

        (index as usize) < self.size && self.stored_at(index as usize) == Some(member)
    }

    /// Add `member` to the set. Idempotent for members already present.
    ///
    /// Returns whether the member was newly inserted. Upstream returns `this`
    /// for chaining and exposes the answer only through `size`; the bridge
    /// drops this bool so the JS surface matches.
    ///
    /// Out of range, this corrupts the set rather than failing — deliberately.
    /// See the module docs.
    pub fn add(&mut self, member: usize) -> bool {
        if self.has(member) {
            return false;
        }

        // `this.dense[this.size] = member` — truncating in range, silently
        // dropped past the end. Both matter: the first is how `add(300)`
        // becomes a stored `44` on an 8-bit set, the second is how `size` gets
        // to run past `length`.
        self.dense.try_set(self.size, member as u32);
        self.sparse.try_set(member, self.size as u32);
        self.size += 1;

        true
    }

    /// Remove `member`, returning whether it was there.
    pub fn delete(&mut self, member: usize) -> bool {
        // Upstream's guard is `index >= this.size || this.dense[index] !== member`.
        // With `index` undefined *both* halves are false-ish in the way that
        // matters: `undefined >= size` is false, so evaluation continues to
        // `this.dense[undefined]`, which is also `undefined` and compares
        // unequal to any member. The net effect is `false`, but by the second
        // clause rather than the first.
        let Some(slot) = self.sparse.try_get(member) else {
            return false;
        };
        let slot = slot as usize;

        if slot >= self.size || self.stored_at(slot) != Some(member) {
            return false;
        }

        // Swap the last member into the hole:
        //
        //   index = this.dense[this.size - 1];
        //   this.dense[this.sparse[member]] = index;
        //   this.sparse[index]             = this.sparse[member];
        //
        // `index` is `undefined` once `size` has run past `length`, and the
        // two stores then behave *differently*, which is the trap:
        let last = self.dense.try_get(self.size - 1);

        // Storing `undefined` into a typed array stores 0 — `ToNumber` gives
        // `NaN`, and a `NaN` element store is 0. So this write still lands.
        self.dense.try_set(slot, last.unwrap_or(0));

        // But `this.sparse[undefined]` is a *string-keyed* property on the
        // typed-array object, not element 0. It creates an expando that no
        // method ever reads, and it leaves `sparse` untouched. Writing
        // `sparse[0]` here instead would be a silent divergence — verified
        // against Node: `new SparseSet(3)`, add 0/1/2/99, `delete(1)` leaves
        // `sparse` as `[0, 1, 2]` and sets `sparse.undefined = 1`.
        if let Some(last) = last {
            self.sparse.try_set(last as usize, slot as u32);
        }

        self.size -= 1;

        true
    }

    /// A cursor over the set's members, in `dense` order.
    ///
    /// Freezes `size` — upstream's `var size = this.size` — and reads `dense`
    /// live, which is the hybrid capture of [`crate::cursor`]. Two consequences
    /// that no upstream test observes:
    ///
    /// * A [`delete`](SparseSet::delete) during iteration moves the last
    ///   member into the hole, and a cursor already past that slot will yield
    ///   the moved member a **second** time while never yielding the removed
    ///   one. The frozen `size` also means it keeps walking past the set's new
    ///   end.
    /// * If `size` has been pushed past `length` by an out-of-range
    ///   [`add`](SparseSet::add), the walk runs off the end of `dense` and
    ///   produces [`crate::cursor::Step::Gap`] — JS `{done: false, value:
    ///   undefined}`. This is the only module in the port so far where the
    ///   shrink window of DESIGN.md 3.7 is reachable at all, and it is
    ///   reachable through the *public* API.
    pub fn values(&self) -> Cursor<'_, Self> {
        Cursor::new(self)
    }

    /// `this.dense[slot]`, widened back to a member.
    fn stored_at(&self, slot: usize) -> Option<usize> {
        self.dense.try_get(slot).map(|member| member as usize)
    }
}

/// The `values()` walk: `size` frozen, `dense` read live.
impl Sequence for SparseSet {
    type Item = u32;
    /// Nothing beyond the length: the ordinal is the index into `dense`.
    type Frozen = ();

    fn freeze(&self) -> ((), usize) {
        // `var size = this.size`, not `this.length` and not `dense.length`.
        // Which of the three upstream happens to capture differs per module
        // (DIV-PROJ-19), so it is read off the source file every time rather than
        // normalised.
        ((), self.size)
    }

    fn slot(&self, _frozen: &(), ordinal: usize) -> Option<u32> {
        self.dense.try_get(ordinal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::Step;
    use crate::utils::typed_arrays::{PointerWidth, POINTER_ARRAY_TOO_LARGE};

    fn members(set: &SparseSet) -> Vec<u32> {
        set.values().collect()
    }

    /// 1:1 port of the whole upstream `it` block set, as a baseline.
    #[test]
    fn reproduces_the_upstream_suite() {
        let mut set = SparseSet::new(10).unwrap();

        set.add(3);
        set.add(4);
        set.add(3);
        assert_eq!(set.size(), 2);
        assert_eq!(set.length(), 10);

        let mut set = SparseSet::new(10).unwrap();
        set.add(3);
        assert!(set.has(3));
        assert!(!set.has(1));

        let mut set = SparseSet::new(10).unwrap();
        set.add(3);
        assert!(set.delete(3));
        assert!(!set.delete(4));
        assert_eq!(set.size(), 0);

        let mut set = SparseSet::new(10).unwrap();
        for member in 0..6 {
            set.add(member);
        }
        set.clear();
        assert_eq!(set.size(), 0);
        assert!(!set.has(1));

        let mut set = SparseSet::new(10).unwrap();
        set.add(3);
        set.add(6);
        set.add(9);
        assert_eq!(members(&set), vec![3, 6, 9]);
    }

    /// Gap 1: `add` reports insertion, and the duplicate really is a no-op on
    /// the backing arrays rather than merely on `size`.
    #[test]
    fn a_duplicate_add_changes_nothing_at_all() {
        let mut set = SparseSet::new(10).unwrap();

        assert!(set.add(3));
        let before = set.clone();

        assert!(!set.add(3));
        assert_eq!(set.dense(), before.dense());
        assert_eq!(set.sparse(), before.sparse());
        assert_eq!(set.size(), 1);
    }

    /// Gap 2: `delete` is the swap-with-last, and the layout it leaves is
    /// observable through iteration order. Upstream asserts only `size`.
    #[test]
    fn delete_swaps_the_last_member_into_the_hole() {
        let mut set = SparseSet::new(10).unwrap();

        for member in [1, 2, 3, 4] {
            set.add(member);
        }

        assert!(set.delete(2));
        // 4 moved into slot 1, so iteration order is no longer insertion order.
        assert_eq!(members(&set), vec![1, 4, 3]);
        assert!(set.has(4) && set.has(3) && set.has(1) && !set.has(2));

        // And the moved member's index was fixed up, so deleting it still works.
        assert!(set.delete(4));
        assert_eq!(members(&set), vec![1, 3]);
    }

    /// Gap 3: deleting the *last* member takes the branch where the swap is a
    /// self-assignment. Upstream never deletes anything but a fresh singleton.
    #[test]
    fn deleting_the_last_member_is_a_self_swap() {
        let mut set = SparseSet::new(5).unwrap();

        set.add(0);
        set.add(1);

        assert!(set.delete(1));
        assert_eq!(members(&set), vec![0]);
        assert!(set.delete(0));
        assert_eq!(members(&set), Vec::<u32>::new());
        assert!(!set.delete(0));
    }

    /// Gap 4: `clear` leaves debris, and re-adding after it must still work.
    /// The structure's O(1) clear is only correct because every read is gated
    /// on `size` first.
    #[test]
    fn clear_leaves_stale_entries_that_stay_unreachable() {
        let mut set = SparseSet::new(5).unwrap();

        set.add(2);
        set.add(4);
        set.clear();

        assert_eq!(set.size(), 0);
        assert!(!set.has(2) && !set.has(4));
        // The debris is still there. Upstream behaves identically; asserting it
        // pins the O(1) clear against a future "tidy-up".
        assert_eq!(set.dense().try_get(0), Some(2));

        set.add(4);
        assert_eq!(set.size(), 1);
        assert!(set.has(4) && !set.has(2));
        assert_eq!(members(&set), vec![4]);
    }

    /// Gap 5: `has` and `delete` are safe out of range; only `add` is not.
    #[test]
    fn reads_out_of_range_report_absence() {
        let mut set = SparseSet::new(10).unwrap();

        set.add(3);

        assert!(!set.has(10));
        assert!(!set.has(300));
        assert!(!set.has(usize::MAX));
        assert!(!set.delete(10));
        assert!(!set.delete(300));
        assert_eq!(set.size(), 1);
    }

    /// Gap 6: the compound upstream defect. An out-of-range `add` truncates
    /// the stored member, drops the index write, and bumps `size` anyway.
    ///
    /// Verified against real Node: `new SparseSet(10)` then `add(300)` gives
    /// `size = 1`, `dense = [44, 0, ...]`, `sparse` untouched, and
    /// `has(300) === has(44) === false`.
    #[test]
    fn an_out_of_range_add_corrupts_the_set_exactly_as_upstream_does() {
        let mut set = SparseSet::new(10).unwrap();

        assert!(set.add(300));

        assert_eq!(set.size(), 1);
        // 300 truncated by the Uint8Array store.
        assert_eq!(set.dense().try_get(0), Some(300 % 256));
        assert_eq!(set.sparse(), &PointerVec::zeroed(PointerWidth::U8, 10));
        // Unfindable under either name: `sparse[300]` was never written, and
        // `sparse[44]` is 0 but `dense[0]` is 44 — which would match, except
        // that reaching that comparison requires `sparse[44]`, and 44 is
        // itself past the end.
        assert!(!set.has(300));
        assert!(!set.has(44));
        // But it *is* iterable, because iteration reads `dense` directly.
        assert_eq!(members(&set), vec![44]);
    }

    /// ToUint32 and a narrowing store compose to the same answer in both
    /// languages: JS stores `-1` into a `Uint8Array` as `255`, and the bridge's
    /// `u32` coercion of `-1` is `u32::MAX`, which narrows to `255` too.
    #[test]
    fn negative_members_arrive_as_their_two_s_complement_and_truncate_alike() {
        let mut set = SparseSet::new(10).unwrap();

        set.add(u32::MAX as usize);

        assert_eq!(set.dense().try_get(0), Some(255));
        assert_eq!(members(&set), vec![255]);
    }

    /// Gap 7: `size` running past `length`, which is what makes the cursor's
    /// gap branch reachable through the public API.
    #[test]
    fn size_can_exceed_length_and_then_iteration_hits_the_gap() {
        let mut set = SparseSet::new(2).unwrap();

        for member in [100, 101, 102, 103] {
            set.add(member);
        }

        assert_eq!(set.size(), 4);
        assert_eq!(set.length(), 2);

        let mut cursor = set.values();

        assert_eq!(cursor.frozen_len(), 4);
        assert_eq!(cursor.step(), Step::Item(100));
        assert_eq!(cursor.step(), Step::Item(101));
        // Ordinals 2 and 3 are inside the frozen size but past `dense`.
        // Upstream yields `{done: false, value: undefined}` here — measured.
        assert_eq!(cursor.step(), Step::Gap);
        assert_eq!(cursor.step(), Step::Gap);
        assert_eq!(cursor.step(), Step::Done);
    }

    /// Deleting once `size` has run past `length` is where upstream's two
    /// swap stores stop behaving alike: the `dense` store lands (as 0), the
    /// `sparse` store becomes a string-keyed expando and leaves the array
    /// alone. Pinned against Node, which gives `dense = [0, 0, 2]` and
    /// `sparse = [0, 1, 2]` for exactly this sequence.
    #[test]
    fn a_delete_past_capacity_writes_dense_but_not_sparse() {
        let mut set = SparseSet::new(3).unwrap();

        for member in [0, 1, 2, 99] {
            set.add(member);
        }

        assert_eq!(set.size(), 4);
        assert!(set.delete(1));

        assert_eq!(set.size(), 3);
        assert_eq!(set.dense(), &PointerVec::U8(vec![0, 0, 2]));
        // Not `[1, 1, 2]`, which is what writing `sparse[0]` would give.
        assert_eq!(set.sparse(), &PointerVec::U8(vec![0, 1, 2]));
    }

    /// Gap 8: DIV-STACK-1. The cursor is not restartable, while the set is
    /// re-iterable — the two-level `Symbol.iterator` of DIV-STACK-2, seen from Rust.
    #[test]
    fn cursors_do_not_restart_but_the_set_can_be_walked_again() {
        let mut set = SparseSet::new(10).unwrap();

        set.add(3);
        set.add(6);

        let mut cursor = set.values();
        assert_eq!(cursor.by_ref().collect::<Vec<_>>(), vec![3, 6]);
        assert_eq!(cursor.collect::<Vec<_>>(), Vec::<u32>::new());

        assert_eq!(members(&set), vec![3, 6]);
        assert_eq!(members(&set), vec![3, 6]);
    }

    /// Gap 9: DIV-PROJ-10, on this module's own data. A `delete` between two steps of
    /// a live cursor is visible, because `dense` is read lazily — and the
    /// frozen `size` keeps the walk going past the set's new end.
    ///
    /// Measured against Node: after `delete(1)` on `{1,2,3}` mid-walk, the
    /// remaining steps are `2` then `3`, and `dense` is `[3, 2, 3]`.
    #[test]
    fn a_delete_during_iteration_is_visible_to_the_cursor() {
        let mut set = SparseSet::new(10).unwrap();

        for member in [1, 2, 3] {
            set.add(member);
        }

        let mut state = crate::cursor::CursorState::open(&set);

        assert_eq!(state.step(&set), Step::Item(1));
        set.delete(1);

        assert_eq!(set.size(), 2);
        // Still three steps, because `size` was frozen at 3.
        assert_eq!(state.step(&set), Step::Item(2));
        assert_eq!(state.step(&set), Step::Item(3));
        assert_eq!(state.step(&set), Step::Done);
    }

    /// The nastier half of the same behaviour: delete the member the cursor is
    /// *about* to reach, and the swap makes the last member appear twice.
    #[test]
    fn a_delete_ahead_of_the_cursor_can_yield_a_member_twice() {
        let mut set = SparseSet::new(10).unwrap();

        for member in [1, 2, 3] {
            set.add(member);
        }

        let mut state = crate::cursor::CursorState::open(&set);

        assert_eq!(state.step(&set), Step::Item(1));
        // 3 is swapped into slot 1, which the cursor has not passed yet.
        set.delete(2);

        assert_eq!(state.step(&set), Step::Item(3));
        assert_eq!(state.step(&set), Step::Item(3));
        assert_eq!(state.step(&set), Step::Done);
    }

    /// Growth is not visible: `size` is frozen, so members added mid-walk are
    /// never reached.
    #[test]
    fn an_add_during_iteration_is_not_visible_to_the_cursor() {
        let mut set = SparseSet::new(10).unwrap();

        set.add(1);

        let mut state = crate::cursor::CursorState::open(&set);

        set.add(2);
        set.add(3);

        assert_eq!(state.step(&set), Step::Item(1));
        assert_eq!(state.step(&set), Step::Done);
    }

    /// Gap 10: the width machinery. Upstream's only tested length is 10, so
    /// the 16- and 32-bit branches are never reached through this module.
    #[test]
    fn picks_one_pointer_width_for_both_arrays() {
        for (length, expected) in [
            (0usize, PointerWidth::U8),
            (256, PointerWidth::U8),
            (257, PointerWidth::U16),
            (65_536, PointerWidth::U16),
            (65_537, PointerWidth::U32),
        ] {
            let set = SparseSet::new(length).unwrap();

            assert_eq!(set.dense().width(), expected, "length {length}");
            assert_eq!(set.sparse().width(), expected, "length {length}");
            assert_eq!(set.dense().len(), length);
        }
    }

    #[test]
    fn rejects_a_length_no_pointer_array_can_index() {
        assert_eq!(
            SparseSet::new(4_294_967_297).unwrap_err(),
            POINTER_ARRAY_TOO_LARGE
        );
    }

    /// Gap 11: the degenerate lengths. `new SparseSet(0)` is legal upstream and
    /// every member is out of range, so every `add` corrupts and nothing is
    /// ever findable.
    #[test]
    fn a_zero_length_set_accepts_nothing_and_finds_nothing() {
        let mut set = SparseSet::new(0).unwrap();

        assert!(!set.has(0));
        assert!(!set.delete(0));

        set.add(0);

        assert_eq!(set.size(), 1);
        assert!(!set.has(0));
        // Every step is a gap: `size` is 1 and `dense` is empty.
        assert_eq!(set.values().step(), Step::Gap);
        assert_eq!(members(&set), Vec::<u32>::new());
    }

    #[test]
    fn a_one_member_set_behaves() {
        let mut set = SparseSet::new(1).unwrap();

        assert!(set.add(0));
        assert!(!set.add(0));
        assert!(set.has(0));
        assert_eq!(members(&set), vec![0]);
        assert!(set.delete(0));
        assert_eq!(set.size(), 0);
    }

    /// Filling to capacity: every slot used, no truncation, and `size` lands
    /// exactly on `length`.
    #[test]
    fn fills_to_capacity_without_running_off_the_end() {
        let mut set = SparseSet::new(300).unwrap();

        for member in 0..300 {
            assert!(set.add(member));
        }

        assert_eq!(set.size(), 300);
        assert_eq!(set.dense().width(), PointerWidth::U16);
        assert_eq!(
            members(&set),
            (0..300).map(|m| m as u32).collect::<Vec<_>>()
        );
        assert!(set.has(299));
    }
}

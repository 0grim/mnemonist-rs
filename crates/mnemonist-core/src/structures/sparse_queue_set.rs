//! Port of upstream `sparse-queue-set.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! A sparse set whose `dense` array is a **ring**: `enqueue` appends at
//! `(start + size) % capacity` and `dequeue` takes from `start`, so the
//! structure is a FIFO queue with O(1) membership testing. `sparse` still maps
//! a member to its slot, but membership is no longer `slot < size` — the live
//! window wraps, so it is "is `slot` inside the window, and does `dense[slot]`
//! still hold this member".
//!
//! # The sentinel does not fit
//!
//! `dequeue` marks the departed member absent by writing the **capacity** into
//! `sparse`, as a value no live slot can have:
//!
//! ```js
//! this.sparse[member] = this.capacity;
//! ```
//!
//! `sparse` is `getPointerArray(capacity)` wide, and that function sizes for
//! the largest *index*, `capacity - 1`. So at `capacity === 256` the array is a
//! `Uint8Array` and the sentinel truncates to **0** — a perfectly ordinary
//! slot. The dequeued member then reads as present again as soon as slot 0 is
//! back inside the window, and `enqueue` refuses to re-admit it. Same at
//! `capacity === 65536`, one width up. Verified on Node 24.18.1; see BUG-SPARSE-QUEUE-SET-1.
//!
//! # `enqueue` never checks whether the ring is full
//!
//! Nothing bounds `size` by `capacity`. In range that is unreachable — a queue
//! holding every member of `0..capacity` rejects every further `enqueue` as a
//! duplicate — but one out-of-range member is enough, because `sparse[member]`
//! is then `undefined` and the duplicate check cannot fire. The write lands on
//! a **live slot**, silently evicting whoever was there, and `size` runs past
//! `capacity`. See BUG-SPARSE-QUEUE-SET-2.
//!
//! # `capacity === 0` divides by zero
//!
//! `(this.start + this.size) % this.capacity` is `NaN`, so both stores are
//! dropped, `size` still increments, `start` climbs forever because
//! `start === capacity` is never true, and iteration yields `undefined` per
//! phantom member. That is the [`Step::Gap`](crate::cursor::Step::Gap) window
//! of `docs/DECISIONS.md`'s iteration section, reached here in two calls. See BUG-SPARSE-QUEUE-SET-3.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::structures::sparse_queue_set::SparseQueueSet;
//!
//! let mut queue = SparseQueueSet::new(4).unwrap();
//! queue.enqueue(2);
//! queue.enqueue(3);
//! queue.enqueue(2);
//!
//! assert_eq!(queue.size(), 2);
//! assert!(queue.has(2));
//! assert_eq!(queue.dequeue(), Some(2));
//! assert!(!queue.has(2));
//! assert_eq!(queue.values().collect::<Vec<_>>(), vec![3]);
//! ```

use crate::cursor::{Cursor, Sequence};
use crate::utils::typed_arrays::{get_pointer_array, PointerVec};

/// A FIFO queue over the members `0..capacity`, with O(1) membership.
#[derive(Debug, Clone)]
pub struct SparseQueueSet {
    capacity: usize,
    /// Index of the front of the ring. Bounded by `capacity` — except at
    /// `capacity == 0`, where upstream's wrap check never fires and it grows
    /// without limit.
    start: usize,
    size: usize,
    dense: PointerVec,
    sparse: PointerVec,
}

impl SparseQueueSet {
    /// Build an empty queue able to hold the members `0..capacity`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE`] when
    /// `capacity` exceeds what a 32-bit pointer array can index, which is where
    /// upstream throws.
    ///
    /// # Panics
    ///
    /// A `capacity` that passes validation but is too large to allocate aborts
    /// through the global allocator; stable Rust has no fallible `Vec`
    /// allocation.
    pub fn new(capacity: usize) -> Result<Self, &'static str> {
        let width = get_pointer_array(capacity as f64)?;

        Ok(Self {
            capacity,
            start: 0,
            size: 0,
            dense: PointerVec::zeroed(width, capacity),
            sparse: PointerVec::zeroed(width, capacity),
        })
    }

    /// Members currently queued.
    ///
    /// Can exceed [`capacity`](SparseQueueSet::capacity) after an out-of-range
    /// [`enqueue`](SparseQueueSet::enqueue); see BUG-SPARSE-QUEUE-SET-2 in the module docs.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Capacity the queue was built with.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Index of the front of the ring — upstream's `start` property.
    ///
    /// Public because it is public upstream and because it is the only way to
    /// see the ring's rotation from outside: two queues holding the same
    /// members in the same order can have different `start`s, and the
    /// differential fuzzer compares it after every op for exactly that reason.
    pub fn start(&self) -> usize {
        self.start
    }

    /// The ring buffer, upstream's `dense` property.
    pub fn dense(&self) -> &PointerVec {
        &self.dense
    }

    /// Member-to-slot index, upstream's `sparse` property.
    pub fn sparse(&self) -> &PointerVec {
        &self.sparse
    }

    /// Empty the queue in O(1). Only `start` and `size` are touched.
    pub fn clear(&mut self) {
        self.start = 0;
        self.size = 0;
    }

    /// Whether `member` is currently queued.
    pub fn has(&self, member: usize) -> bool {
        if self.size == 0 {
            return false;
        }

        // `undefined` past the end of `sparse`, and every comparison against
        // `undefined` is false, so `in_window` is false and an out-of-range
        // member is reported absent.
        let Some(slot) = self.sparse.try_get(member) else {
            return false;
        };
        let slot = slot as usize;

        self.in_window(slot) && self.dense.try_get(slot) == Some(member as u32)
    }

    /// Append `member` to the back of the queue. Idempotent for members already
    /// queued.
    ///
    /// Returns whether the member was newly enqueued. Upstream returns `this`
    /// for chaining and exposes the answer only through `size`; the bridge
    /// drops this bool so the JS surface matches.
    ///
    /// Out of range this evicts whatever occupies the target slot and pushes
    /// `size` past `capacity` — deliberately. See BUG-SPARSE-QUEUE-SET-2.
    pub fn enqueue(&mut self, member: usize) -> bool {
        // Upstream guards the duplicate check on `size !== 0` rather than
        // letting `in_window` decide, which matters: with `size == 0` the
        // window is empty and the check would be false anyway, so the guard is
        // redundant *and* faithful to reproduce, since it costs nothing.
        if self.size != 0 && self.has(member) {
            return false;
        }

        // `(this.start + this.size) % this.capacity`. NaN at capacity 0, which
        // JS turns into two dropped stores rather than a throw.
        match self.slot_after_end() {
            None => {
                self.size += 1;
            }
            Some(slot) => {
                // A truncating store in range and a dropped one past the end,
                // exactly as in `SparseSet::add` — but here the slot is inside
                // the ring by construction, so what the out-of-range member
                // corrupts is the *contents* of a live slot rather than the
                // array's bounds.
                self.dense.try_set(slot, member as u32);
                self.sparse.try_set(member, slot as u32);
                self.size += 1;
            }
        }

        true
    }

    /// Remove and return the member at the front, or `None` when empty.
    ///
    /// Upstream returns `undefined` for an empty queue, and also for a queue of
    /// `capacity === 0`, where `dense[start]` is `undefined` too.
    pub fn dequeue(&mut self) -> Option<u32> {
        if self.size == 0 {
            return None;
        }

        let slot = self.start;

        self.size -= 1;
        self.start += 1;

        // `if (this.start === this.capacity) this.start = 0;` — an equality
        // test, not a modulo. At capacity 0 it never fires and `start` grows
        // without bound, which is observable through the `start` property.
        if self.start == self.capacity {
            self.start = 0;
        }

        let member = self.dense.try_get(slot);

        // `this.sparse[member] = this.capacity` — the absence sentinel.
        //
        // Two ways this fails to do what it looks like, both upstream's:
        //
        // * `member` is `undefined` at capacity 0, and `sparse[undefined]` is a
        //   string-keyed expando rather than element 0 — the BUG-SPARSE-SET-3 asymmetry
        //   again. Nothing reads it, and `sparse` is left alone.
        // * the sentinel itself is `capacity`, which does not fit a
        //   `getPointerArray(capacity)` element at 256 or 65536 and truncates
        //   to 0. That is BUG-SPARSE-QUEUE-SET-1, and `try_set` reproduces it by narrowing.
        if let Some(member) = member {
            self.sparse.try_set(member as usize, self.capacity as u32);
        }

        member
    }

    /// A cursor over the queued members, front to back.
    ///
    /// Freezes `capacity`, `size` **and** `start` — upstream captures all three
    /// into the closure — and reads `dense` live. So a
    /// [`dequeue`](SparseQueueSet::dequeue) during iteration does not move the
    /// walk, and an [`enqueue`](SparseQueueSet::enqueue) that overwrites a slot
    /// the cursor has not passed yet *is* visible.
    pub fn values(&self) -> Cursor<'_, Self> {
        Cursor::new(self)
    }

    /// Whether `slot` is inside the live window, reproducing upstream's
    /// expression **including its precedence**.
    ///
    /// ```js
    /// index < this.capacity &&
    /// (index >= this.start && index < this.start + this.size)
    /// ||
    /// (index < ((this.start + this.size) % this.capacity))
    /// ```
    ///
    /// `&&` binds tighter than `||`, so this is one three-term conjunction
    /// **or** one comparison — not the `capacity` guard distributed over both
    /// arms. The guard is in fact redundant, because
    /// `(start + size) % capacity < capacity` always holds, but reproducing the
    /// shape rather than a simplification of it is the point: the two halves
    /// are the unwrapped and wrapped parts of the ring, and at
    /// `size > capacity` they overlap rather than partition.
    fn in_window(&self, slot: usize) -> bool {
        let end = self.start + self.size;

        if slot < self.capacity && slot >= self.start && slot < end {
            return true;
        }

        // `x % 0` is `NaN` in JS and every comparison against it is false.
        match self.capacity {
            0 => false,
            capacity => slot < end % capacity,
        }
    }

    /// `(start + size) % capacity`, or `None` where JS produces `NaN`.
    fn slot_after_end(&self) -> Option<usize> {
        match self.capacity {
            0 => None,
            capacity => Some((self.start + self.size) % capacity),
        }
    }
}

/// The `values()` walk: `capacity`, `size` and `start` frozen, `dense` live.
impl Sequence for SparseQueueSet {
    type Item = u32;
    /// `(capacity, start)` — `c` and the initial `i` of the upstream closure.
    /// The frozen length is `size`, which the cursor holds itself.
    type Frozen = (usize, usize);

    fn freeze(&self) -> ((usize, usize), usize) {
        ((self.capacity, self.start), self.size)
    }

    fn slot(&self, frozen: &(usize, usize), ordinal: usize) -> Option<u32> {
        let (capacity, start) = *frozen;

        // Upstream advances `i` and resets it on `i === c`, which for a
        // `start` inside the ring is `(start + ordinal) % capacity`. At
        // capacity 0 the reset never fires, so `i` simply keeps climbing —
        // off the end of an empty `dense`, one gap per step.
        let index = match capacity {
            0 => start + ordinal,
            capacity => (start + ordinal) % capacity,
        };

        self.dense.try_get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{CursorState, Step};
    use crate::utils::typed_arrays::{PointerWidth, POINTER_ARRAY_TOO_LARGE};

    fn members(queue: &SparseQueueSet) -> Vec<u32> {
        queue.values().collect()
    }

    /// 1:1 port of the whole upstream `it` block set, as a baseline —
    /// including the wrap-around block, which runs its body 13 times.
    #[test]
    fn reproduces_the_upstream_suite() {
        let mut queue = SparseQueueSet::new(10).unwrap();
        queue.enqueue(3);
        queue.enqueue(4);
        queue.enqueue(3);
        assert_eq!(queue.size(), 2);
        assert_eq!(queue.capacity(), 10);

        let mut queue = SparseQueueSet::new(10).unwrap();
        queue.enqueue(3);
        assert!(queue.has(3));
        assert!(!queue.has(1));

        let mut queue = SparseQueueSet::new(10).unwrap();
        for member in 0..6 {
            queue.enqueue(member);
        }
        queue.clear();
        assert_eq!(queue.size(), 0);
        assert!(!queue.has(1));

        let mut queue = SparseQueueSet::new(4).unwrap();
        for member in [2, 3, 0, 1] {
            queue.enqueue(member);
        }
        for (expected, remaining) in [(2, 3), (3, 2), (0, 1), (1, 0)] {
            assert_eq!(queue.dequeue(), Some(expected));
            assert_eq!(queue.size(), remaining);
        }

        // "should not break when wrapping around": 13 full cycles.
        let mut queue = SparseQueueSet::new(4).unwrap();
        let values = [2u32, 3, 1];

        for _ in 0..13 {
            for (index, member) in values.iter().enumerate() {
                assert!(!queue.has(*member as usize));
                queue.enqueue(*member as usize);
                assert_eq!(queue.size(), index + 1);
            }

            assert_eq!(members(&queue), values);

            for (index, member) in values.iter().enumerate() {
                assert!(queue.has(*member as usize));
                assert_eq!(queue.dequeue(), Some(*member));
                assert_eq!(queue.size(), values.len() - index - 1);
            }
        }

        for member in [0, 1, 2, 3, 2, 3, 0, 1] {
            queue.enqueue(member);
        }
        assert_eq!(members(&queue), vec![0, 1, 2, 3]);

        let mut queue = SparseQueueSet::new(10).unwrap();
        queue.enqueue(3);
        queue.enqueue(6);
        queue.enqueue(9);
        assert_eq!(members(&queue), vec![3, 6, 9]);
    }

    /// **BUG-SPARSE-QUEUE-SET-1.** `dequeue`'s absence sentinel is `capacity`, and `sparse` is
    /// sized to hold indices `0..capacity` — so at exactly `capacity == 256`
    /// the sentinel truncates to `0`, an ordinary slot.
    ///
    /// Verified against Node 24.18.1, and against `capacity == 255` as a
    /// control, where the sentinel fits and the queue behaves.
    #[test]
    fn the_dequeue_sentinel_truncates_at_the_pointer_width_boundary() {
        let mut queue = SparseQueueSet::new(256).unwrap();

        assert_eq!(queue.dense().width(), PointerWidth::U8);

        queue.enqueue(5);
        assert_eq!(queue.dequeue(), Some(5));
        // Should be 256; a Uint8Array cannot hold it.
        assert_eq!(queue.sparse().try_get(5), Some(0));

        // While the queue is empty the damage is hidden, because `has` short
        // circuits on `size == 0`.
        assert!(!queue.has(5));

        queue.enqueue(7);

        // Now slot 0 is inside the window again — and `dense[0]` is still 5.
        assert!(
            queue.has(5),
            "BUG-SPARSE-QUEUE-SET-1: a dequeued member reads as present"
        );
        assert_eq!(members(&queue), vec![7]);

        // Worse than a wrong answer: `enqueue` believes it too, so 5 can never
        // be re-admitted.
        queue.enqueue(5);
        assert_eq!(queue.size(), 1);
        assert_eq!(members(&queue), vec![7]);
    }

    /// The control, one below the boundary: the sentinel fits and nothing goes
    /// wrong. Same at 254, 65_535 — the defect is exactly at the powers.
    #[test]
    fn one_below_the_boundary_the_sentinel_fits() {
        let mut queue = SparseQueueSet::new(255).unwrap();

        queue.enqueue(5);
        queue.dequeue();
        assert_eq!(queue.sparse().try_get(5), Some(255));

        queue.enqueue(7);
        assert!(!queue.has(5));

        queue.enqueue(5);
        assert_eq!(members(&queue), vec![7, 5]);
    }

    /// And again one width up, at `capacity == 65536`, where `sparse` is a
    /// `Uint16Array` and 65536 truncates to 0. Verified against Node.
    #[test]
    fn the_sentinel_truncates_at_the_second_boundary_too() {
        let mut queue = SparseQueueSet::new(65_536).unwrap();

        assert_eq!(queue.sparse().width(), PointerWidth::U16);

        queue.enqueue(5);
        queue.dequeue();
        assert_eq!(queue.sparse().try_get(5), Some(0));

        queue.enqueue(7);
        assert!(queue.has(5));
    }

    /// **BUG-SPARSE-QUEUE-SET-2.** `enqueue` never checks whether the ring is full, so one
    /// out-of-range member evicts a live one and pushes `size` past `capacity`.
    ///
    /// Verified against Node: `dense` becomes `[100, 1, 2, 3]`, `size` 5 with
    /// capacity 4, `has(0)` is false, and the walk yields five members with
    /// `100` twice.
    #[test]
    fn an_out_of_range_enqueue_evicts_a_live_member() {
        let mut queue = SparseQueueSet::new(4).unwrap();

        for member in 0..4 {
            queue.enqueue(member);
        }

        assert_eq!(members(&queue), vec![0, 1, 2, 3]);

        queue.enqueue(100);

        assert_eq!(queue.size(), 5);
        assert_eq!(queue.capacity(), 4);
        assert_eq!(queue.dense(), &PointerVec::U8(vec![100, 1, 2, 3]));
        // Member 0 was in the queue and is now simply gone.
        assert!(!queue.has(0));
        // And 100 is not findable either: its `sparse` write was dropped.
        assert!(!queue.has(100));
        // The walk runs `size` steps around a `capacity`-slot ring.
        assert_eq!(members(&queue), vec![100, 1, 2, 3, 100]);
        assert_eq!(queue.dequeue(), Some(100));
        assert_eq!(queue.start(), 1);
    }

    /// The `dense` store still truncates, as everywhere else in this family.
    /// Node: `new SparseQueueSet(10)` then `enqueue(300)` stores `44`.
    #[test]
    fn an_out_of_range_member_truncates_into_the_ring() {
        let mut queue = SparseQueueSet::new(10).unwrap();

        queue.enqueue(300);

        assert_eq!(queue.dense().try_get(0), Some(300 % 256));
        assert_eq!(queue.sparse(), &PointerVec::zeroed(PointerWidth::U8, 10));
        assert!(!queue.has(300) && !queue.has(44));
        assert_eq!(members(&queue), vec![44]);
        // It dequeues under its truncated name.
        assert_eq!(queue.dequeue(), Some(44));
    }

    /// **BUG-SPARSE-QUEUE-SET-3.** `capacity == 0` makes `(start + size) % capacity` `NaN`, so
    /// both stores are dropped, `size` still climbs, `start` never wraps, and
    /// iteration is all gaps.
    ///
    /// Verified against Node: `Array.from(q)` is `[undefined]` after one
    /// `enqueue`, `dequeue()` is `undefined`, `sparse.undefined` is set to `0`
    /// as an expando, and `start` is `1` — never reset, because
    /// `1 === 0` is false.
    #[test]
    fn a_zero_capacity_queue_counts_phantoms() {
        let mut queue = SparseQueueSet::new(0).unwrap();

        assert!(!queue.has(0));

        queue.enqueue(0);

        assert_eq!(queue.size(), 1);
        assert!(queue.dense().is_empty());
        assert!(!queue.has(0));
        // One phantom member, one `{done: false, value: undefined}`.
        assert_eq!(queue.values().step(), Step::Gap);
        assert_eq!(members(&queue), Vec::<u32>::new());

        assert_eq!(queue.dequeue(), None);
        assert_eq!(queue.size(), 0);
        // `start` climbed and was never reset: `start === capacity` is `1 === 0`.
        assert_eq!(queue.start(), 1);

        queue.enqueue(1);
        queue.enqueue(2);
        assert_eq!(queue.size(), 2);

        let mut walk = queue.values();
        assert_eq!(walk.step(), Step::Gap);
        assert_eq!(walk.step(), Step::Gap);
        assert_eq!(walk.step(), Step::Done);
    }

    /// `dequeue` on an empty queue is `undefined` and changes nothing — the one
    /// branch upstream's suite never takes, since it always dequeues exactly as
    /// many members as it enqueued.
    #[test]
    fn dequeuing_an_empty_queue_changes_nothing() {
        let mut queue = SparseQueueSet::new(4).unwrap();

        assert_eq!(queue.dequeue(), None);
        assert_eq!((queue.start(), queue.size()), (0, 0));

        queue.enqueue(1);
        queue.dequeue();

        assert_eq!(queue.dequeue(), None);
        assert_eq!((queue.start(), queue.size()), (1, 0));
    }

    /// A capacity-one ring: `start` wraps on every single dequeue, and the
    /// duplicate check has exactly one slot to look at.
    #[test]
    fn a_one_slot_ring_wraps_on_every_dequeue() {
        let mut queue = SparseQueueSet::new(1).unwrap();

        assert!(queue.enqueue(0));
        assert!(!queue.enqueue(0));
        assert!(queue.has(0));
        assert_eq!(members(&queue), vec![0]);

        assert_eq!(queue.dequeue(), Some(0));
        assert_eq!((queue.start(), queue.size()), (0, 0));
        // Sentinel is 1, which fits a Uint8Array, so 0 really is absent.
        assert_eq!(queue.sparse().try_get(0), Some(1));
        assert!(!queue.has(0));

        assert!(queue.enqueue(0));
        assert_eq!(members(&queue), vec![0]);
    }

    /// `clear` resets `start` as well as `size` — the only structural
    /// difference from `SparseSet::clear`, and one the upstream suite checks
    /// only on an unrotated queue.
    #[test]
    fn clear_resets_the_rotation_as_well_as_the_size() {
        let mut queue = SparseQueueSet::new(4).unwrap();

        for member in [0, 1, 2] {
            queue.enqueue(member);
        }
        queue.dequeue();
        queue.dequeue();
        assert_eq!(queue.start(), 2);

        queue.clear();

        assert_eq!((queue.start(), queue.size()), (0, 0));
        assert!(!queue.has(2));
        // The debris is still in `dense`, and stays unreachable.
        assert_eq!(queue.dense().try_get(2), Some(2));

        queue.enqueue(3);
        assert_eq!(members(&queue), vec![3]);
        assert!(queue.has(3) && !queue.has(2));
    }

    /// A wrapped window is the case the `||` in `in_window` exists for: the
    /// live slots are `{2, 3, 0}` and the two halves of the expression cover
    /// them separately.
    #[test]
    fn membership_holds_across_a_wrapped_window() {
        let mut queue = SparseQueueSet::new(4).unwrap();

        for member in [0, 1, 2, 3] {
            queue.enqueue(member);
        }
        queue.dequeue();
        queue.dequeue();
        queue.enqueue(0);

        // start 2, size 3: slots 2, 3 and 0.
        assert_eq!((queue.start(), queue.size()), (2, 3));
        assert!(queue.has(2) && queue.has(3) && queue.has(0));
        assert!(!queue.has(1));
        assert_eq!(members(&queue), vec![2, 3, 0]);
    }

    /// DIV-STACK-1 / DIV-STACK-2 from Rust: the cursor is exhausted once, the queue is not.
    #[test]
    fn cursors_do_not_restart_but_the_queue_can_be_walked_again() {
        let mut queue = SparseQueueSet::new(10).unwrap();

        queue.enqueue(3);
        queue.enqueue(6);

        let mut cursor = queue.values();
        assert_eq!(cursor.by_ref().collect::<Vec<_>>(), vec![3, 6]);
        assert_eq!(cursor.collect::<Vec<_>>(), Vec::<u32>::new());

        assert_eq!(members(&queue), vec![3, 6]);
        assert_eq!(members(&queue), vec![3, 6]);
    }

    /// DIV-PROJ-10: `start` is frozen, so a `dequeue` mid-walk does **not** move the
    /// cursor — it keeps yielding the member it already passed the front of.
    ///
    /// Measured against Node: after `dequeue()` between the first and second
    /// step of a walk over `{0,1,2}`, the remaining steps are `1` then `2`.
    #[test]
    fn a_dequeue_during_iteration_does_not_move_the_walk() {
        let mut queue = SparseQueueSet::new(4).unwrap();

        for member in [0, 1, 2] {
            queue.enqueue(member);
        }

        let mut state = CursorState::open(&queue);

        assert_eq!(state.step(&queue), Step::Item(0));
        queue.dequeue();

        assert_eq!((queue.start(), queue.size()), (1, 2));
        // Still three steps from the frozen start, not two from the new one.
        assert_eq!(state.step(&queue), Step::Item(1));
        assert_eq!(state.step(&queue), Step::Item(2));
        assert_eq!(state.step(&queue), Step::Done);
    }

    /// The other half of DIV-PROJ-10: `dense` is read live, so an `enqueue` that
    /// overwrites a slot the cursor has not reached yet **is** visible.
    #[test]
    fn an_enqueue_that_overwrites_an_unread_slot_is_visible() {
        let mut queue = SparseQueueSet::new(3).unwrap();

        for member in [0, 1, 2] {
            queue.enqueue(member);
        }

        let mut state = CursorState::open(&queue);

        assert_eq!(state.step(&queue), Step::Item(0));

        // Out of range, so no duplicate check fires; lands on slot 0, which the
        // cursor has already passed, and pushes `size` to 4 — invisible to this
        // walk, whose length was frozen at 3.
        queue.enqueue(99);
        // A second one lands on slot 1, which the cursor has NOT passed.
        queue.enqueue(98);

        assert_eq!(state.step(&queue), Step::Item(98));
        assert_eq!(state.step(&queue), Step::Item(2));
        assert_eq!(state.step(&queue), Step::Done);
    }

    /// The width machinery. Upstream's only tested capacities are 4 and 10, so
    /// the 16- and 32-bit branches are never reached through this module.
    #[test]
    fn picks_one_pointer_width_for_both_arrays() {
        for (capacity, expected) in [
            (0usize, PointerWidth::U8),
            (256, PointerWidth::U8),
            (257, PointerWidth::U16),
            (65_536, PointerWidth::U16),
            (65_537, PointerWidth::U32),
        ] {
            let queue = SparseQueueSet::new(capacity).unwrap();

            assert_eq!(queue.dense().width(), expected, "capacity {capacity}");
            assert_eq!(queue.sparse().width(), expected, "capacity {capacity}");
            assert_eq!(queue.dense().len(), capacity);
        }
    }

    #[test]
    fn rejects_a_capacity_no_pointer_array_can_index() {
        assert_eq!(
            SparseQueueSet::new(4_294_967_297).unwrap_err(),
            POINTER_ARRAY_TOO_LARGE
        );
    }

    /// Filling the ring exactly, then draining it exactly, with no truncation
    /// and no eviction anywhere.
    #[test]
    fn fills_and_drains_a_full_ring() {
        let mut queue = SparseQueueSet::new(300).unwrap();

        for member in 0..300 {
            assert!(queue.enqueue(member));
        }

        assert_eq!(queue.size(), 300);
        assert_eq!(queue.dense().width(), PointerWidth::U16);
        assert_eq!(members(&queue), (0..300u32).collect::<Vec<_>>());
        assert!(queue.has(299));

        for member in 0..300u32 {
            assert_eq!(queue.dequeue(), Some(member));
        }

        assert_eq!((queue.size(), queue.start()), (0, 0));
        assert!(!queue.has(0));
    }
}

//! Port of upstream `circular-buffer.js` (140 LOC).
//!
//! A `FixedDeque` that **overwrites** instead of refusing. Upstream says so
//! literally:
//!
//! ```js
//! function paste(name) {
//!   CircularBuffer.prototype[name] = FixedDeque.prototype[name];
//! }
//!
//! Object.keys(FixedDeque.prototype).forEach(paste);
//! Object.getOwnPropertySymbols(FixedDeque.prototype).forEach(paste);
//! ```
//!
//! — the whole prototype is copied, and then exactly two methods are replaced.
//! `clear`, `pop`, `shift`, `peekFirst`, `peekLast`, `get`, `forEach`,
//! `toArray`, `values`, `entries`, `inspect` and `Symbol.iterator` are the
//! *same functions*, not reimplementations. This module mirrors that: it holds
//! a [`FixedDeque`] and delegates everything except `push` and `unshift`, so
//! there is one ring implementation and one place a wrap can be wrong.
//!
//! It also means **every defect of `FixedDeque` is a defect here**, by
//! construction and not by coincidence — including BUG-CIRCULAR-BUFFER-1, `get` being bounded by
//! the capacity rather than by the size.
//!
//! # The two overridden methods
//!
//! ```js
//! CircularBuffer.prototype.push = function (item) {
//!   var index = this.start + this.size;
//!   if (index >= this.capacity) index -= this.capacity;
//!   this.items[index] = item;
//!
//!   if (this.size === this.capacity) {      // overwriting
//!     index++;
//!     if (index >= this.capacity) this.start = 0;
//!     else this.start = index;
//!     return this.size;                     // unchanged
//!   }
//!
//!   return ++this.size;
//! };
//! ```
//!
//! Two things to keep. The store happens **before** the fullness test, so a
//! full buffer overwrites the slot the oldest element is in and only then
//! advances `start` past it. And the return value is the size *unchanged* when
//! overwriting, which is the only externally visible signal that anything was
//! dropped — there is none other, and nothing upstream tests it.
//!
//! `unshift` is the mirror image and is slightly odd in the source: it assigns
//! `this.start = index` in both branches of its `if`, once inside and once
//! after. Same effect; reproduced as one assignment.
//!
//! Measured on Node 24.18.1, `new CircularBuffer(Array, 3)`:
//!
//! ```text
//! push(1) -> 1  start 0  items [1, <2 empty>]
//! push(2) -> 2  start 0  items [1, 2, <1 empty>]
//! push(3) -> 3  start 0  items [1, 2, 3]
//! push(4) -> 3  start 1  items [4, 2, 3]
//! push(5) -> 3  start 2  items [4, 5, 3]
//! ```
//!
//! # What upstream's `from` does NOT do
//!
//! `CircularBuffer.from` is the same fourteen lines as the other two, so its
//! array-like branch copies by index and assigns `size` — it does **not** push,
//! and therefore does not overwrite. An oversized iterable leaves a buffer with
//! `size > capacity`, exactly as it does for a `FixedDeque`; the overwriting
//! behaviour that is this class's entire purpose is bypassed. And its other
//! branch is BUG-UTILS-ITERABLES-2 and cannot run at all.

use std::fmt;

use crate::cursor::{Cursor, Sequence};
use crate::structures::backing::Backing;
use crate::structures::fixed_deque::{DequeCapture, FixedDeque};

/// `arguments.length < 2`, verbatim.
pub const MISSING_ARGUMENTS: &str =
    "mnemonist/circular-buffer: expecting an Array class and a capacity.";

/// `typeof capacity !== 'number' || capacity <= 0`, verbatim.
pub const BAD_CAPACITY: &str = "mnemonist/circular-buffer: `capacity` should be a positive number.";

/// `CircularBuffer.from` with no capacity and an unguessable iterable, verbatim.
pub const CANNOT_GUESS_LENGTH: &str = "mnemonist/circular-buffer.from: could not guess iterable \
     length. Please provide desired capacity as last argument.";

/// What upstream throws, with the same text.
///
/// Only one variant, because `push` and `unshift` cannot fail here — that is
/// the whole difference from [`FixedDeque`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `capacity <= 0`.
    Capacity,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(BAD_CAPACITY)
    }
}

impl std::error::Error for Error {}

/// A fixed-capacity ring that overwrites its oldest element rather than
/// refusing a new one.
pub struct CircularBuffer<T> {
    /// The pasted prototype, as a field. Everything but `push` and `unshift`
    /// delegates straight through.
    inner: FixedDeque<T>,
}

impl<T: Clone> CircularBuffer<T> {
    /// `new CircularBuffer(ArrayClass, capacity)`.
    pub fn new(backing: Backing<T>, capacity: usize) -> Result<Self, Error> {
        FixedDeque::new(backing, capacity)
            .map(|inner| Self { inner })
            .map_err(|_| Error::Capacity)
    }

    /// `CircularBuffer.from(arrayLike, ArrayClass, capacity)` — copies by
    /// index, so it does **not** overwrite. See the module docs.
    pub fn from_array_like(
        backing: Backing<T>,
        capacity: usize,
        values: impl IntoIterator<Item = T>,
    ) -> Result<Self, Error> {
        FixedDeque::from_array_like(backing, capacity, values)
            .map(|inner| Self { inner })
            .map_err(|_| Error::Capacity)
    }

    /// `#.push` — append, overwriting the oldest element when full.
    ///
    /// Returns the new size, which is the size **unchanged** when overwriting.
    pub fn push(&mut self, item: T) -> usize {
        let index = self.inner.slot_for_push();

        // Before the fullness test, as upstream does it: a full buffer writes
        // over the slot the oldest element occupies, and only then steps past
        // it.
        self.inner.store(index, item);

        if self.inner.size() == self.inner.capacity() {
            let next = index + 1;

            self.inner.set_start(if next >= self.inner.capacity() {
                0
            } else {
                next
            });

            return self.inner.size();
        }

        self.inner.set_size(self.inner.size() + 1);

        self.inner.size()
    }

    /// `#.unshift` — prepend, overwriting the newest element when full.
    pub fn unshift(&mut self, item: T) -> usize {
        let index = self.inner.slot_for_unshift();

        self.inner.store(index, item);
        // Upstream assigns `this.start = index` once inside the `if` and once
        // after it. Same effect, written once.
        self.inner.set_start(index);

        if self.inner.size() == self.inner.capacity() {
            return self.inner.size();
        }

        self.inner.set_size(self.inner.size() + 1);

        self.inner.size()
    }

    // ---- pasted from FixedDeque.prototype, unchanged ----------------------

    /// `#.clear` — empties the buffer, resetting `start` and `size` without
    /// shrinking the backing storage.
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// `#.pop` — removes and returns the element at the back, or `None`
    /// (upstream `undefined`) when empty.
    pub fn pop(&mut self) -> Option<T> {
        self.inner.pop()
    }

    /// `#.shift` — removes and returns the element at the front, or `None`
    /// (upstream `undefined`) when empty.
    pub fn shift(&mut self) -> Option<T> {
        self.inner.shift()
    }

    /// `#.peekFirst` — the front element without removing it.
    pub fn peek_first(&self) -> Option<T> {
        self.inner.peek_first()
    }

    /// `#.peekLast` — the back element without removing it.
    pub fn peek_last(&self) -> Option<T> {
        self.inner.peek_last()
    }

    /// Bounded by the capacity, not by the size — BUG-CIRCULAR-BUFFER-1, inherited literally.
    pub fn get(&self, index: usize) -> Option<T> {
        self.inner.get(index)
    }

    /// `#.toArray` — the live elements front to back. `None` marks a slot the
    /// ring considers occupied but which holds no value; see
    /// [`FixedDeque::to_array`].
    pub fn to_array(&self) -> Vec<Option<T>> {
        self.inner.to_array()
    }

    /// The raw backing array, in physical order rather than ring order. This
    /// is upstream's `this.items`, exposed so the differential fuzzer can
    /// compare representations and not just observable behaviour.
    pub fn items(&self) -> &[Option<T>] {
        self.inner.items()
    }

    /// `#.size` — the number of live elements, never above the capacity.
    pub fn size(&self) -> usize {
        self.inner.size()
    }

    /// `#.capacity` — the fixed ring length fixed at construction.
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// `#.start` — the physical index the front element occupies.
    pub fn start(&self) -> usize {
        self.inner.start()
    }

    /// `this.items[i]` — see [`FixedDeque::slot_at`].
    pub fn slot_at(&self, index: usize) -> Option<T> {
        self.inner.slot_at(index)
    }

    /// Whether the buffer holds no elements — `size === 0`.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// `#.values` — a fresh, non-restartable cursor, front to back.
    pub fn values(&self) -> Cursor<'_, Self> {
        Cursor::new(self)
    }
}

/// Delegated, because `CircularBuffer.prototype.values` **is**
/// `FixedDeque.prototype.values` — the same function object, pasted.
impl<T: Clone> Sequence for CircularBuffer<T> {
    type Item = T;
    type Frozen = DequeCapture;

    fn freeze(&self) -> (DequeCapture, usize) {
        self.inner.freeze()
    }

    fn slot(&self, frozen: &DequeCapture, ordinal: usize) -> Option<T> {
        self.inner.slot(frozen, ordinal)
    }
}

impl<T: fmt::Debug> fmt::Debug for CircularBuffer<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CircularBuffer")
            .field("inner", &self.inner)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{CursorState, Step};

    fn plain(capacity: usize) -> CircularBuffer<i32> {
        CircularBuffer::new(Backing::Holes, capacity).expect("capacity is positive")
    }

    fn typed(capacity: usize) -> CircularBuffer<i32> {
        CircularBuffer::new(Backing::Filled(0), capacity).expect("capacity is positive")
    }

    fn present(buffer: &CircularBuffer<i32>) -> Vec<i32> {
        buffer.to_array().into_iter().flatten().collect()
    }

    /// The upstream suite, ported 1:1. `test/circular-buffer.js`, 16 `it`
    /// blocks — the best test-to-source ratio in the library at 2.4:1.
    #[test]
    fn reproduces_the_upstream_suite() {
        assert_eq!(
            CircularBuffer::<i32>::new(Backing::Holes, 0).unwrap_err(),
            Error::Capacity
        );

        let mut buffer = plain(10);
        buffer.push(1);
        assert_eq!((buffer.size(), buffer.capacity()), (1, 10));

        // `should be possible to wrap buffer around when pushing.`
        let mut buffer = plain(3);
        for value in 1..=4 {
            buffer.push(value);
        }
        assert_eq!(present(&buffer), vec![2, 3, 4]);
        assert_eq!(buffer.size(), 3);
        buffer.push(5);
        assert_eq!(present(&buffer), vec![3, 4, 5]);
        buffer.push(6);
        assert_eq!(present(&buffer), vec![4, 5, 6]);
        buffer.push(7);
        buffer.push(8);
        assert_eq!(present(&buffer), vec![6, 7, 8]);
        assert_eq!(buffer.size(), 3);

        // `…when unshifting.`
        let mut buffer = plain(3);
        for value in 1..=4 {
            buffer.unshift(value);
        }
        assert_eq!(present(&buffer), vec![4, 3, 2]);
        buffer.unshift(5);
        assert_eq!(present(&buffer), vec![5, 4, 3]);
        buffer.unshift(6);
        assert_eq!(present(&buffer), vec![6, 5, 4]);
        buffer.unshift(7);
        buffer.unshift(8);
        assert_eq!(present(&buffer), vec![8, 7, 6]);
        assert_eq!(buffer.size(), 3);

        let mut buffer = plain(2);
        buffer.push(2);
        buffer.push(3);
        buffer.clear();
        assert_eq!(buffer.size(), 0);
        assert_eq!(present(&buffer), Vec::<i32>::new());

        let mut buffer = plain(3);
        assert_eq!(buffer.peek_first(), None);
        assert_eq!(buffer.peek_last(), None);
        buffer.push(1);
        assert_eq!(
            (buffer.peek_first(), buffer.peek_last()),
            (Some(1), Some(1))
        );
        buffer.push(2);
        buffer.push(3);
        assert_eq!(
            (buffer.peek_first(), buffer.peek_last()),
            (Some(1), Some(3))
        );
        assert_eq!(buffer.get(0), Some(1));
        assert_eq!(buffer.get(1), Some(2));
        assert_eq!(buffer.get(2), Some(3));
        assert_eq!(buffer.get(3), None);

        // `peekLast should not be subject to one-off errors (#223).`
        let mut buffer: CircularBuffer<bool> =
            CircularBuffer::new(Backing::Holes, 2).expect("capacity is positive");
        buffer.push(true);
        buffer.push(true);
        buffer.push(true);
        buffer.push(false);
        buffer.push(true);
        assert_eq!(
            buffer.to_array(),
            vec![Some(false), Some(true)],
            "regression #223"
        );
        assert_eq!(buffer.peek_first(), Some(false));
        assert_eq!(buffer.peek_last(), Some(true));
        assert_eq!(buffer.get(1), Some(true));

        // `should be possible to pop the buffer.`
        let mut buffer = plain(3);
        for value in 1..=3 {
            buffer.push(value);
        }
        assert_eq!(buffer.pop(), Some(3));
        assert_eq!(buffer.pop(), Some(2));
        assert_eq!(buffer.pop(), Some(1));
        assert_eq!(buffer.pop(), None);
        assert_eq!(buffer.size(), 0);
        buffer.push(4);
        assert_eq!((buffer.size(), buffer.peek_last()), (1, Some(4)));

        let mut second = plain(6);
        for value in 1..=3 {
            second.push(value);
        }
        for value in 4..=6 {
            second.unshift(value);
        }
        assert_eq!(second.pop(), Some(3));
        assert_eq!(second.size(), 5);

        // `should be possible to shift the buffer.`
        let mut buffer = typed(3);
        for value in 1..=3 {
            buffer.push(value);
        }
        assert_eq!(buffer.shift(), Some(1));
        assert_eq!(buffer.shift(), Some(2));
        assert_eq!(buffer.shift(), Some(3));
        assert_eq!(buffer.size(), 0);
        buffer.push(4);
        buffer.push(5);
        assert_eq!(buffer.size(), 2);
        assert_eq!(buffer.pop(), Some(5));
        assert_eq!(buffer.shift(), Some(4));

        // `should be possible to unshift the buffer.`
        let mut buffer = typed(6);
        for value in 10..=12 {
            buffer.push(value);
        }
        assert_eq!(buffer.unshift(13), 4);
        assert_eq!(buffer.unshift(14), 5);
        assert_eq!(buffer.unshift(15), 6);
        assert_eq!((buffer.size(), buffer.start()), (6, 3));
        assert_eq!(buffer.pop(), Some(12));
        assert_eq!(buffer.shift(), Some(15));

        // `should be consistent over time.`
        let mut buffer = typed(3);
        buffer.push(1);
        buffer.push(2);
        buffer.pop();
        assert_eq!(present(&buffer), vec![1]);
        buffer.push(3);
        buffer.push(4);
        assert_eq!(present(&buffer), vec![1, 3, 4]);
        buffer.shift();
        buffer.shift();
        assert_eq!(present(&buffer), vec![4]);
        buffer.pop();
        assert_eq!(present(&buffer), Vec::<i32>::new());
        buffer.push(5);
        buffer.push(6);
        assert_eq!(present(&buffer), vec![5, 6]);
        buffer.shift();
        assert_eq!(present(&buffer), vec![6]);

        // The three `from` blocks.
        let buffer = CircularBuffer::from_array_like(Backing::Holes, 3, [1, 2, 3]).unwrap();
        assert_eq!(present(&buffer), vec![1, 2, 3]);

        let buffer = CircularBuffer::from_array_like(Backing::Filled(0), 45, [1, 2, 3]).unwrap();
        assert_eq!(buffer.values().collect::<Vec<_>>(), vec![1, 2, 3]);

        let buffer = CircularBuffer::from_array_like(Backing::Filled(0), 5, [1, 2, 3]).unwrap();
        assert_eq!((buffer.size(), buffer.capacity()), (3, 5));
        let entries: Vec<(usize, i32)> = buffer.values().enumerate().collect();
        assert_eq!(entries, vec![(0, 1), (1, 2), (2, 3)]);

        // `should handle tricky situations.`
        let mut buffer = typed(6);
        for value in 1..=3 {
            buffer.push(value);
        }
        assert_eq!(buffer.unshift(4), 4);
        assert_eq!(buffer.unshift(5), 5);
        assert_eq!(buffer.peek_first(), Some(5));
        assert_eq!(buffer.peek_last(), Some(3));
        assert_eq!(buffer.get(1), Some(4));
        assert_eq!((buffer.size(), buffer.start()), (5, 4));
        assert_eq!(buffer.pop(), Some(3));
        assert_eq!(buffer.shift(), Some(5));
        assert_eq!(buffer.unshift(5), 4);
        assert_eq!(buffer.peek_first(), Some(5));
    }

    /// The one externally visible signal that a push dropped something: the
    /// return value stops advancing. Nothing upstream asserts it.
    ///
    /// Pinned against Node 24.18.1: `push` returns 1, 2, 3, 3, 3 on a
    /// capacity-3 buffer, with `start` at 0, 0, 0, 1, 2.
    #[test]
    fn a_push_that_overwrites_returns_the_unchanged_size() {
        let mut buffer = plain(3);

        assert_eq!(
            (1..=5).map(|value| buffer.push(value)).collect::<Vec<_>>(),
            vec![1, 2, 3, 3, 3]
        );
        assert_eq!(buffer.start(), 2);
        assert_eq!(buffer.items(), [Some(4), Some(5), Some(3)]);
        assert_eq!(present(&buffer), vec![3, 4, 5]);
    }

    /// The same for `unshift`, whose `start` walks the other way.
    ///
    /// Pinned against Node 24.18.1: 1, 2, 3, 3 with `start` 2, 1, 0, 2.
    #[test]
    fn an_unshift_that_overwrites_returns_the_unchanged_size() {
        let mut buffer = plain(3);
        let sizes: Vec<usize> = (1..=4).map(|value| buffer.unshift(value)).collect();

        assert_eq!(sizes, vec![1, 2, 3, 3]);
        assert_eq!(buffer.start(), 2);
        assert_eq!(buffer.items(), [Some(3), Some(2), Some(4)]);
        assert_eq!(present(&buffer), vec![4, 3, 2]);
    }

    /// `push` writes the slot **before** testing for fullness, so the element
    /// it overwrites is the one `start` is sitting on — and `start` then steps
    /// past it. Getting the order the other way round drops the wrong element.
    #[test]
    fn a_full_push_overwrites_the_slot_start_is_on() {
        let mut buffer = plain(3);
        for value in 1..=3 {
            buffer.push(value);
        }

        assert_eq!(buffer.start(), 0);
        buffer.push(4);

        // Slot 0 held the oldest element, 1. It now holds 4, and `start` has
        // moved to 1.
        assert_eq!(buffer.items(), [Some(4), Some(2), Some(3)]);
        assert_eq!(buffer.start(), 1);
        assert_eq!(buffer.peek_first(), Some(2));
    }

    /// Mixing `push` and `unshift` on a full buffer: each overwrites the other
    /// end's element.
    ///
    /// Pinned against Node 24.18.1: push 1..4 then unshift 9 on a capacity-3
    /// buffer gives `start 0`, `items [9, 2, 3]`, `toArray [9, 2, 3]`.
    #[test]
    fn push_and_unshift_overwrite_opposite_ends() {
        let mut buffer = plain(3);
        for value in 1..=4 {
            buffer.push(value);
        }
        buffer.unshift(9);

        assert_eq!((buffer.start(), buffer.size()), (0, 3));
        assert_eq!(buffer.items(), [Some(9), Some(2), Some(3)]);
        assert_eq!(present(&buffer), vec![9, 2, 3]);
    }

    /// BUG-CIRCULAR-BUFFER-1 is inherited literally, because `get` is the same function object
    /// upstream. Same transcript as the `FixedDeque` test.
    #[test]
    fn get_is_bounded_by_the_capacity_here_too() {
        let mut buffer = plain(3);
        buffer.push(1);
        buffer.push(2);
        buffer.pop();

        assert_eq!(buffer.size(), 1);
        assert_eq!(buffer.get(1), Some(2));
        assert_eq!(buffer.get(3), None);
    }

    /// `from` does **not** overwrite: it copies by index and assigns `size`,
    /// so an oversized iterable leaves `size > capacity` and the walk goes
    /// round the ring more than once — the same state a `FixedDeque` reaches,
    /// on the class whose whole purpose is to prevent it.
    ///
    /// Pinned against Node 24.18.1:
    /// `CircularBuffer.from([1,2,3], Array, 2)` → `size 3`, `start 0`,
    /// `items [1,2,3]`, `toArray() [1, 2, 1]`.
    #[test]
    fn from_bypasses_the_overwriting_that_this_class_exists_for() {
        let buffer = CircularBuffer::from_array_like(Backing::Holes, 2, [1, 2, 3]).unwrap();

        assert_eq!(
            (buffer.size(), buffer.capacity(), buffer.start()),
            (3, 2, 0)
        );
        assert_eq!(buffer.items(), [Some(1), Some(2), Some(3)]);
        assert_eq!(present(&buffer), vec![1, 2, 1]);
    }

    /// A buffer that has wrapped many times still walks front to back.
    #[test]
    fn many_wraps_still_walk_in_order() {
        let mut buffer = plain(4);
        for value in 1..=13 {
            buffer.push(value);
        }

        assert_eq!(buffer.size(), 4);
        assert_eq!(present(&buffer), vec![10, 11, 12, 13]);
        assert_eq!(buffer.peek_first(), Some(10));
        assert_eq!(buffer.peek_last(), Some(13));
        assert_eq!(buffer.values().collect::<Vec<_>>(), vec![10, 11, 12, 13]);
    }

    /// A capacity-one buffer is the degenerate ring: every push replaces the
    /// single element and `start` never moves off zero.
    #[test]
    fn a_capacity_of_one_replaces_in_place() {
        let mut buffer = plain(1);

        assert_eq!(buffer.push(1), 1);
        assert_eq!(buffer.push(2), 1);
        assert_eq!(buffer.push(3), 1);
        assert_eq!(buffer.start(), 0);
        assert_eq!(present(&buffer), vec![3]);
        assert_eq!(buffer.unshift(4), 1);
        assert_eq!(present(&buffer), vec![4]);
    }

    /// DIV-PROJ-10: a `push` that overwrites *behind* an open cursor is visible to it,
    /// because elements are read live while the geometry is frozen. This is the
    /// sharpest form of the hybrid capture in the whole wave — the cursor can
    /// yield an element that was not in the buffer when the walk started.
    #[test]
    fn an_overwriting_push_is_visible_to_an_open_cursor() {
        let mut buffer = plain(3);
        for value in 1..=3 {
            buffer.push(value);
        }

        let mut state = CursorState::open(&buffer);
        assert_eq!(state.step(&buffer), Step::Item(1));

        // Overwrites slot 0 — already consumed — and slot 1, which the cursor
        // has not reached.
        buffer.push(4);
        buffer.push(5);

        assert_eq!(state.step(&buffer), Step::Item(5));
        assert_eq!(state.step(&buffer), Step::Item(3));
        assert_eq!(state.step(&buffer), Step::Done);
    }

    /// DIV-STACK-1 / DIV-STACK-2 again, on this class: pasted `values` is still a factory,
    /// and the cursor it makes is still not restartable.
    #[test]
    fn cursors_do_not_restart_but_the_buffer_can_be_walked_again() {
        let mut buffer = plain(3);
        for value in 1..=5 {
            buffer.push(value);
        }

        let mut cursor = buffer.values();
        assert_eq!(cursor.next(), Some(3));
        assert_eq!(cursor.by_ref().collect::<Vec<_>>(), vec![4, 5]);
        assert_eq!(cursor.collect::<Vec<_>>(), Vec::<i32>::new());

        assert_eq!(buffer.values().collect::<Vec<_>>(), vec![3, 4, 5]);
    }

    #[test]
    fn error_text_is_upstreams() {
        assert_eq!(Error::Capacity.to_string(), BAD_CAPACITY);
        assert!(MISSING_ARGUMENTS.contains("Array class"));
        assert!(CANNOT_GUESS_LENGTH.contains("could not guess iterable length"));
        assert!(BAD_CAPACITY.starts_with("mnemonist/circular-buffer:"));
    }

    #[test]
    fn debug_reports_the_geometry() {
        assert!(format!("{:?}", plain(2)).starts_with("CircularBuffer {"));
    }
}

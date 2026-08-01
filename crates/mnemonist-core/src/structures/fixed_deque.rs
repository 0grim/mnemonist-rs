//! Port of upstream `fixed-deque.js` (357 LOC).
//!
//! A double-ended queue of fixed capacity, laid out as a ring over a
//! pre-allocated array. `start` is where the first element lives; the element
//! at logical position `j` is at `items[(start + j) mod capacity]`.
//!
//! Everything below is upstream's, and only the first is visible in
//! `test/fixed-deque.js`.
//!
//! # 1. The wrap is a conditional subtraction, not a modulo
//!
//! ```js
//! var index = this.start + this.size;
//! if (index >= this.capacity) index -= this.capacity;
//! ```
//!
//! That is the same answer as `% capacity` **only while `start + size` stays
//! below `2 * capacity`**, which every public path maintains — except one.
//! `from` assigns `size` from the iterable without checking it against the
//! capacity (see item 4 of [`crate::structures::fixed_stack`]'s docs, which is
//! the same code), so `size` can exceed `capacity` and a single subtraction
//! then leaves the index still out of range. Upstream reads the raw slot and
//! gets whatever is there; so does this port, which is why every one of these
//! sites is written as one conditional subtraction rather than as `%`.
//!
//! `values()` and `forEach` are the exception: they *step* the index one
//! position at a time and wrap on equality, which really is `% capacity` for
//! any ordinal. Those two are written that way.
//!
//! # 2. `get` is bounded by the capacity, not by the size (NOTES B-62)
//!
//! ```js
//! FixedDeque.prototype.get = function (index) {
//!   if (this.size === 0 || index >= this.capacity) return;
//!   ...
//! };
//! ```
//!
//! Every other reader guards on `this.size`. `get` guards on the *capacity*, so
//! for `size <= index < capacity` it returns whatever is sitting in the slot —
//! debris a `pop` or a `shift` left behind, or the class zero, or `undefined`.
//! Measured on Node 24.18.1:
//!
//! ```js
//! var d = new FixedDeque(Array, 3); d.push(1); d.push(2); d.pop();
//! d.size;    // 1
//! d.get(1);  // 2   <- popped, still returned
//! d.get(3);  // undefined, because 3 >= capacity
//! ```
//!
//! The original suite asks only for `get(0..2)` on a full capacity-3 deque and
//! `get(3)` on the same, so the guard it exercises is the one that happens to
//! be right there.
//!
//! # 3. `pop`, `shift` and `clear` leave the elements in place
//!
//! `pop` moves `size`, `shift` moves `start` and `size`, `clear` sets both to
//! zero. None of them writes to the array, so the elements stay reachable
//! through `items` and through `get` (item 2). A port that cleared the slot
//! would diverge on both.
//!
//! # 4. `toArray` has a fast path whose result is a *slice*
//!
//! ```js
//! var offset = this.start + this.size;
//! if (offset < this.capacity) return this.items.slice(this.start, offset);
//! ```
//!
//! Same elements as the slow path in every reachable case — `offset < capacity`
//! implies `offset <= items.length` for both backings — so the port implements
//! one walk. The two differ only in what an absent slot becomes: `slice`
//! preserves a hole, while the slow path's `array[j] = undefined` creates an
//! own property. The port produces the fast path's answer; see the divergence
//! table in `docs/modules/fixed-deque.md`.

use std::fmt;

use crate::cursor::{Cursor, Sequence};
use crate::structures::backing::Backing;

/// `arguments.length < 2`, verbatim.
pub const MISSING_ARGUMENTS: &str =
    "mnemonist/fixed-deque: expecting an Array class and a capacity.";

/// `typeof capacity !== 'number' || capacity <= 0`, verbatim.
pub const BAD_CAPACITY: &str = "mnemonist/fixed-deque: `capacity` should be a positive number.";

/// `FixedDeque.from` with no capacity and an unguessable iterable, verbatim.
pub const CANNOT_GUESS_LENGTH: &str = "mnemonist/fixed-deque.from: could not guess iterable \
     length. Please provide desired capacity as last argument.";

/// What upstream throws, with the same text.
///
/// `push` and `unshift` word their message differently — `deque.push:` against
/// `deque.unshift:` — so the end is part of the value, not a formatting detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `capacity <= 0`. The non-numeric half is a JavaScript-only notion and is
    /// checked at the bridge.
    Capacity,
    /// A full deque, named by the method that refused.
    Full(End, usize),
}

/// Which end of the deque an operation addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// `#.push`.
    Back,
    /// `#.unshift`.
    Front,
}

impl End {
    fn method(self) -> &'static str {
        match self {
            Self::Back => "push",
            Self::Front => "unshift",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capacity => formatter.write_str(BAD_CAPACITY),
            Self::Full(end, capacity) => write!(
                formatter,
                "mnemonist/fixed-deque.{}: deque capacity ({capacity}) exceeded!",
                end.method()
            ),
        }
    }
}

impl std::error::Error for Error {}

/// A double-ended queue of fixed capacity.
pub struct FixedDeque<T> {
    capacity: usize,
    backing: Backing<T>,
    items: Vec<Option<T>>,
    start: usize,
    size: usize,
}

/// What `values()`, `entries()` and `forEach` all freeze, which is the same
/// three quantities in all three:
///
/// ```js
/// var items = this.items, c = this.capacity, l = this.size, i = this.start;
/// ```
///
/// `l` travels separately as the cursor's frozen length; `items` is read live
/// through `&self`, which this structure — unlike `Stack` — never rebinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DequeCapture {
    capacity: usize,
    start: usize,
}

impl<T: Clone> FixedDeque<T> {
    /// `new FixedDeque(ArrayClass, capacity)`.
    pub fn new(backing: Backing<T>, capacity: usize) -> Result<Self, Error> {
        if capacity == 0 {
            return Err(Error::Capacity);
        }

        let items = backing.allocate(capacity);

        Ok(Self {
            capacity,
            backing,
            items,
            start: 0,
            size: 0,
        })
    }

    /// `FixedDeque.from(arrayLike, ArrayClass, capacity)` — the only branch of
    /// upstream's `from` that is reachable (NOTES B-60).
    ///
    /// Note what it does **not** do: reset `start` (it is already zero from
    /// `clear`), check the iterable against the capacity, or count. `size` is
    /// assigned from the iterable's length, so an oversized one leaves a deque
    /// whose logical length exceeds its ring — after which `values()` walks the
    /// ring more than once and repeats elements. Measured on Node 24.18.1:
    /// `FixedDeque.from([1,2,3,4], Array, 2).toArray()` is `[1, 2, 1, 2]`.
    pub fn from_array_like(
        backing: Backing<T>,
        capacity: usize,
        values: impl IntoIterator<Item = T>,
    ) -> Result<Self, Error> {
        let mut deque = Self::new(backing, capacity)?;
        let mut length = 0;

        for (index, value) in values.into_iter().enumerate() {
            deque.backing.store(&mut deque.items, index, value);
            length = index + 1;
        }

        deque.size = length;

        Ok(deque)
    }

    /// `#.clear` — resets the geometry and leaves the array alone.
    pub fn clear(&mut self) {
        self.start = 0;
        self.size = 0;
    }

    /// `#.push` — append, returning the new size.
    pub fn push(&mut self, item: T) -> Result<usize, Error> {
        if self.size == self.capacity {
            return Err(Error::Full(End::Back, self.capacity));
        }

        let index = wrap_once(self.start + self.size, self.capacity);

        self.backing.store(&mut self.items, index, item);
        self.size += 1;

        Ok(self.size)
    }

    /// `#.unshift` — prepend, returning the new size.
    pub fn unshift(&mut self, item: T) -> Result<usize, Error> {
        if self.size == self.capacity {
            return Err(Error::Full(End::Front, self.capacity));
        }

        let index = self.previous_start();

        self.backing.store(&mut self.items, index, item);
        self.start = index;
        self.size += 1;

        Ok(self.size)
    }

    /// `#.pop` — remove and return the last element, `undefined` when empty.
    pub fn pop(&mut self) -> Option<T> {
        if self.size == 0 {
            return None;
        }

        self.size -= 1;

        let index = wrap_once(self.start + self.size, self.capacity);

        self.items.get(index).cloned().flatten()
    }

    /// `#.shift` — remove and return the first element, `undefined` when empty.
    pub fn shift(&mut self) -> Option<T> {
        if self.size == 0 {
            return None;
        }

        let index = self.start;

        self.size -= 1;
        self.start += 1;

        // `if (this.start === this.capacity) this.start = 0` — equality, not
        // `>=`, which is the same thing given `start < capacity` on entry.
        if self.start == self.capacity {
            self.start = 0;
        }

        self.items.get(index).cloned().flatten()
    }

    /// `#.peekFirst` — `undefined` when empty.
    pub fn peek_first(&self) -> Option<T> {
        if self.size == 0 {
            return None;
        }

        self.items.get(self.start).cloned().flatten()
    }

    /// `#.peekLast` — `undefined` when empty.
    pub fn peek_last(&self) -> Option<T> {
        if self.size == 0 {
            return None;
        }

        let index = wrap_once(self.start + self.size - 1, self.capacity);

        self.items.get(index).cloned().flatten()
    }

    /// `#.get` — **bounded by the capacity, not by the size** (B-62).
    ///
    /// See item 2 in the module docs: for `size <= index < capacity` this
    /// returns whatever is in the slot, which is debris rather than an element.
    /// Reproduced, not guarded.
    pub fn get(&self, index: usize) -> Option<T> {
        if self.size == 0 || index >= self.capacity {
            return None;
        }

        let index = wrap_once(self.start + index, self.capacity);

        self.items.get(index).cloned().flatten()
    }

    /// `#.toArray` — front to back, `size` entries.
    ///
    /// `None` is `undefined`, which a caller building a real `Array` renders as
    /// a hole and a caller building a typed array renders as that class's zero.
    pub fn to_array(&self) -> Vec<Option<T>> {
        let (frozen, length) = self.freeze();

        (0..length).map(|j| self.slot(&frozen, j)).collect()
    }

    /// The backing array itself, in physical order, holes included.
    ///
    /// `items` is a public property upstream; the differential fuzzer compares
    /// it slot for slot after every operation, which is what makes the debris
    /// left by `pop`/`shift`/`clear` checkable directly.
    pub fn items(&self) -> &[Option<T>] {
        &self.items
    }

    /// `this.size`.
    pub fn size(&self) -> usize {
        self.size
    }

    /// `this.capacity`.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// `this.start`, which the original test file asserts on directly.
    pub fn start(&self) -> usize {
        self.start
    }

    /// Whether the deque holds nothing. Not upstream; upstream writes
    /// `size === 0`.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// `#.values` — a fresh, non-restartable cursor, front to back.
    pub fn values(&self) -> Cursor<'_, Self> {
        Cursor::new(self)
    }

    /// Where `unshift` would write: `start - 1`, or the last slot when `start`
    /// is zero.
    fn previous_start(&self) -> usize {
        if self.start == 0 {
            // `this.capacity - 1`. The capacity is at least one, so this cannot
            // underflow.
            return self.capacity - 1;
        }

        self.start - 1
    }
}

/// Upstream's `if (index >= capacity) index -= capacity` — **one** subtraction.
///
/// Not `%`. The two agree while `index < 2 * capacity`, which every path but an
/// oversized `from` maintains; where they disagree, upstream keeps the
/// out-of-range index and reads whatever is there, and so does this.
fn wrap_once(index: usize, capacity: usize) -> usize {
    if index >= capacity {
        return index - capacity;
    }

    index
}

/// Front to back, over `this.size` elements frozen at creation.
///
/// The walk steps one position and wraps on equality, which really is
/// `(start + ordinal) mod capacity` for any ordinal — including ordinals past
/// the capacity, where the ring is walked more than once. That is upstream's
/// loop, and it is what makes an oversized `from` produce repeats rather than
/// an out-of-range read.
impl<T: Clone> Sequence for FixedDeque<T> {
    type Item = T;
    type Frozen = DequeCapture;

    fn freeze(&self) -> (DequeCapture, usize) {
        (
            DequeCapture {
                capacity: self.capacity,
                start: self.start,
            },
            self.size,
        )
    }

    fn slot(&self, frozen: &DequeCapture, ordinal: usize) -> Option<T> {
        let index = (frozen.start + ordinal) % frozen.capacity;

        self.items.get(index).cloned().flatten()
    }
}

impl<T: fmt::Debug> fmt::Debug for FixedDeque<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FixedDeque")
            .field("capacity", &self.capacity)
            .field("start", &self.start)
            .field("size", &self.size)
            .field("items", &self.items)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{CursorState, Step};

    fn plain(capacity: usize) -> FixedDeque<i32> {
        FixedDeque::new(Backing::Holes, capacity).expect("capacity is positive")
    }

    fn typed(capacity: usize) -> FixedDeque<i32> {
        FixedDeque::new(Backing::Filled(0), capacity).expect("capacity is positive")
    }

    fn present(deque: &FixedDeque<i32>) -> Vec<i32> {
        deque.to_array().into_iter().flatten().collect()
    }

    /// The upstream suite, ported 1:1, as the baseline the rest builds on.
    /// `test/fixed-deque.js`, 14 `it` blocks.
    #[test]
    fn reproduces_the_upstream_suite() {
        assert_eq!(
            FixedDeque::<i32>::new(Backing::Holes, 0).unwrap_err(),
            Error::Capacity
        );

        let mut deque = plain(10);
        deque.push(1).unwrap();
        assert_eq!((deque.size(), deque.capacity()), (1, 10));

        let mut one = plain(1);
        one.push(1).unwrap();
        assert_eq!(one.push(2), Err(Error::Full(End::Back, 1)));
        assert_eq!(one.unshift(2), Err(Error::Full(End::Front, 1)));

        let mut deque = plain(2);
        deque.push(2).unwrap();
        deque.push(3).unwrap();
        deque.clear();
        assert_eq!(deque.size(), 0);
        assert_eq!(present(&deque), Vec::<i32>::new());

        // `should be possible to peek.`
        let mut deque = plain(3);
        assert_eq!(deque.peek_first(), None);
        assert_eq!(deque.peek_last(), None);
        deque.push(1).unwrap();
        assert_eq!((deque.peek_first(), deque.peek_last()), (Some(1), Some(1)));
        deque.push(2).unwrap();
        deque.push(3).unwrap();
        assert_eq!((deque.peek_first(), deque.peek_last()), (Some(1), Some(3)));
        assert_eq!(deque.get(0), Some(1));
        assert_eq!(deque.get(1), Some(2));
        assert_eq!(deque.get(2), Some(3));
        assert_eq!(deque.get(3), None);

        // `should be possible to pop the deque.`
        let mut deque = plain(3);
        for value in 1..=3 {
            deque.push(value).unwrap();
        }
        assert_eq!(deque.pop(), Some(3));
        assert_eq!(deque.pop(), Some(2));
        assert_eq!(deque.pop(), Some(1));
        assert_eq!(deque.pop(), None);
        assert_eq!(deque.size(), 0);
        deque.push(4).unwrap();
        assert_eq!((deque.size(), deque.peek_last()), (1, Some(4)));

        let mut second = plain(6);
        for value in 1..=3 {
            second.push(value).unwrap();
        }
        for value in 4..=6 {
            second.unshift(value).unwrap();
        }
        assert_eq!(second.pop(), Some(3));
        assert_eq!(second.size(), 5);

        // `should be possible to shift the deque.`
        let mut deque = typed(3);
        for value in 1..=3 {
            deque.push(value).unwrap();
        }
        assert_eq!(deque.shift(), Some(1));
        assert_eq!(deque.shift(), Some(2));
        assert_eq!(deque.shift(), Some(3));
        assert_eq!(deque.size(), 0);
        deque.push(4).unwrap();
        deque.push(5).unwrap();
        assert_eq!(deque.size(), 2);
        assert_eq!(deque.pop(), Some(5));
        assert_eq!(deque.shift(), Some(4));

        // `should be possible to unshift the deque.`
        let mut deque = typed(6);
        for value in 10..=12 {
            deque.push(value).unwrap();
        }
        assert_eq!(deque.unshift(13), Ok(4));
        assert_eq!(deque.unshift(14), Ok(5));
        assert_eq!(deque.unshift(15), Ok(6));
        assert_eq!((deque.size(), deque.start()), (6, 3));
        assert_eq!(deque.pop(), Some(12));
        assert_eq!(deque.shift(), Some(15));

        // `should be consistent over time.`
        let mut deque = typed(3);
        deque.push(1).unwrap();
        deque.push(2).unwrap();
        deque.pop();
        assert_eq!(present(&deque), vec![1]);
        deque.push(3).unwrap();
        deque.push(4).unwrap();
        assert_eq!(present(&deque), vec![1, 3, 4]);
        deque.shift();
        deque.shift();
        assert_eq!(present(&deque), vec![4]);
        deque.pop();
        assert_eq!(present(&deque), Vec::<i32>::new());
        deque.push(5).unwrap();
        deque.push(6).unwrap();
        assert_eq!(present(&deque), vec![5, 6]);
        deque.shift();
        assert_eq!(present(&deque), vec![6]);

        // The three `from` blocks.
        let deque = FixedDeque::from_array_like(Backing::Holes, 3, [1, 2, 3]).unwrap();
        assert_eq!(present(&deque), vec![1, 2, 3]);

        let deque = FixedDeque::from_array_like(Backing::Filled(0), 45, [1, 2, 3]).unwrap();
        assert_eq!(deque.values().collect::<Vec<_>>(), vec![1, 2, 3]);

        let deque = FixedDeque::from_array_like(Backing::Filled(0), 5, [1, 2, 3]).unwrap();
        assert_eq!((deque.size(), deque.capacity()), (3, 5));
        let entries: Vec<(usize, i32)> = deque.values().enumerate().collect();
        assert_eq!(entries, vec![(0, 1), (1, 2), (2, 3)]);

        // `should handle tricky situations.`
        let mut deque = typed(6);
        for value in 1..=3 {
            deque.push(value).unwrap();
        }
        assert_eq!(deque.unshift(4), Ok(4));
        assert_eq!(deque.unshift(5), Ok(5));
        assert_eq!(deque.peek_first(), Some(5));
        assert_eq!(deque.peek_last(), Some(3));
        assert_eq!(deque.get(1), Some(4));
        assert_eq!((deque.size(), deque.start()), (5, 4));
        assert_eq!(deque.pop(), Some(3));
        assert_eq!(deque.shift(), Some(5));
        assert_eq!(deque.unshift(5), Ok(4));
        assert_eq!(deque.peek_first(), Some(5));
    }

    /// B-62: `get`'s guard is the capacity, so it hands back debris for every
    /// index between `size` and `capacity`.
    ///
    /// Pinned against Node 24.18.1: `new FixedDeque(Array, 3)`, push 1 and 2,
    /// `pop()` — then `size === 1` while `get(1) === 2`.
    #[test]
    fn get_is_bounded_by_the_capacity_and_returns_debris_below_it() {
        let mut deque = plain(3);
        deque.push(1).unwrap();
        deque.push(2).unwrap();
        deque.pop();

        assert_eq!(deque.size(), 1);
        assert_eq!(deque.get(0), Some(1));
        // Popped, and still returned.
        assert_eq!(deque.get(1), Some(2));
        // A slot nothing ever wrote: `undefined` from an `Array`…
        assert_eq!(deque.get(2), None);
        // …and past the capacity, the one guard that does fire.
        assert_eq!(deque.get(3), None);

        // The same deque over a typed class returns the class zero rather than
        // `undefined`, which is the other half of the same defect.
        let mut deque = typed(3);
        deque.push(1).unwrap();
        deque.pop();
        assert_eq!(deque.size(), 0);
        // `size === 0` is the *first* clause of the guard, so an empty deque is
        // the one case `get` gets right.
        assert_eq!(deque.get(0), None);

        let mut deque = typed(3);
        deque.push(1).unwrap();
        deque.push(2).unwrap();
        deque.pop();
        assert_eq!(deque.get(2), Some(0));
    }

    /// …and after a `shift`, the debris `get` returns is the *wrapped* slot,
    /// not simply a stale tail. Pinned against Node: start 1, size 2,
    /// `get(2) === 1`.
    #[test]
    fn get_past_the_size_wraps_around_to_the_shifted_element() {
        let mut deque = plain(3);
        for value in 1..=3 {
            deque.push(value).unwrap();
        }
        deque.shift();

        assert_eq!((deque.start(), deque.size()), (1, 2));
        assert_eq!(deque.get(2), Some(1));
        assert_eq!(deque.peek_last(), Some(3));
    }

    /// `pop`, `shift` and `clear` write nothing to the array.
    #[test]
    fn removals_leave_the_elements_in_place() {
        let mut deque = plain(3);
        for value in 1..=3 {
            deque.push(value).unwrap();
        }

        deque.pop();
        deque.shift();
        assert_eq!(deque.items(), [Some(1), Some(2), Some(3)]);
        assert_eq!((deque.start(), deque.size()), (1, 1));

        deque.clear();
        assert_eq!((deque.start(), deque.size()), (0, 0));
        assert_eq!(deque.items(), [Some(1), Some(2), Some(3)]);
    }

    /// A refused `push` or `unshift` changes nothing, and the two messages name
    /// different methods.
    #[test]
    fn a_refused_insert_leaves_the_deque_untouched_and_names_its_method() {
        let mut deque = plain(2);
        deque.push(1).unwrap();
        deque.unshift(2).unwrap();

        assert_eq!(deque.push(3), Err(Error::Full(End::Back, 2)));
        assert_eq!(deque.unshift(3), Err(Error::Full(End::Front, 2)));
        assert_eq!((deque.start(), deque.size()), (1, 2));

        assert_eq!(
            Error::Full(End::Back, 2).to_string(),
            "mnemonist/fixed-deque.push: deque capacity (2) exceeded!"
        );
        assert_eq!(
            Error::Full(End::Front, 2).to_string(),
            "mnemonist/fixed-deque.unshift: deque capacity (2) exceeded!"
        );
    }

    /// Item 1 and item 4 together: an oversized `from` leaves `size` past the
    /// capacity, and the walk then goes round the ring more than once.
    ///
    /// Pinned against Node 24.18.1:
    /// `FixedDeque.from([1,2,3,4], Array, 2)` → `size 4`, `items [1,2,3,4]`,
    /// `toArray() [1,2,1,2]`, `pop() === 2`.
    #[test]
    fn an_oversized_from_walks_the_ring_more_than_once() {
        let mut deque = FixedDeque::from_array_like(Backing::Holes, 2, [1, 2, 3, 4]).unwrap();

        assert_eq!((deque.size(), deque.capacity()), (4, 2));
        assert_eq!(deque.items(), [Some(1), Some(2), Some(3), Some(4)]);
        assert_eq!(present(&deque), vec![1, 2, 1, 2]);

        // `pop` uses ONE conditional subtraction: `start + size` is 3, which is
        // still 1 after subtracting the capacity, so it reads `items[1]`.
        assert_eq!(deque.pop(), Some(2));
        assert_eq!(deque.size(), 3);
    }

    /// The same call against a typed class truncates instead, and every read of
    /// the missing tail is `undefined`.
    #[test]
    fn an_oversized_from_is_truncated_by_a_typed_class() {
        let deque = FixedDeque::from_array_like(Backing::Filled(0), 2, [1, 2, 3, 4]).unwrap();

        assert_eq!((deque.size(), deque.items().len()), (4, 2));
        assert_eq!(deque.items(), [Some(1), Some(2)]);
        assert_eq!(present(&deque), vec![1, 2, 1, 2]);
    }

    /// D-06 / D-07: the cursor is stateful and does not restart, but the deque
    /// hands out a fresh one every time.
    #[test]
    fn cursors_do_not_restart_but_the_deque_can_be_walked_again() {
        let deque = FixedDeque::from_array_like(Backing::Holes, 3, [1, 2, 3]).unwrap();

        let mut cursor = deque.values();
        assert_eq!(cursor.next(), Some(1));
        assert_eq!(cursor.by_ref().collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(cursor.collect::<Vec<_>>(), Vec::<i32>::new());

        assert_eq!(deque.values().collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    /// D-08, the frozen half: `values()` captures `start`, `capacity` and the
    /// size, so a later `push` is invisible.
    #[test]
    fn a_push_during_iteration_is_not_visible_to_the_cursor() {
        let mut deque = plain(5);
        deque.push(1).unwrap();
        deque.push(2).unwrap();

        let mut state = CursorState::open(&deque);
        assert_eq!(state.step(&deque), Step::Item(1));
        deque.push(9).unwrap();

        assert_eq!(state.step(&deque), Step::Item(2));
        assert_eq!(state.step(&deque), Step::Done);
    }

    /// …and the frozen `start` is the sharper half: a `shift` mid-walk moves
    /// the deque's own start but not the cursor's, so the cursor keeps
    /// yielding from where the deque used to begin.
    #[test]
    fn a_shift_during_iteration_does_not_move_the_cursor() {
        let mut deque = plain(4);
        for value in 1..=3 {
            deque.push(value).unwrap();
        }

        let mut state = CursorState::open(&deque);
        assert_eq!(state.step(&deque), Step::Item(1));

        deque.shift();
        assert_eq!(deque.start(), 1);

        // Ordinal 1 is still `items[(0 + 1) % 4]`, because `start` was frozen
        // at 0 — the shifted element is skipped rather than re-yielded.
        assert_eq!(state.step(&deque), Step::Item(2));
        assert_eq!(state.step(&deque), Step::Item(3));
        assert_eq!(state.step(&deque), Step::Done);
    }

    /// D-08, the live half: an element written ahead of the cursor is seen.
    #[test]
    fn an_overwrite_ahead_of_the_cursor_is_visible() {
        let mut deque = plain(3);
        for value in 1..=3 {
            deque.push(value).unwrap();
        }

        let mut state = CursorState::open(&deque);
        assert_eq!(state.step(&deque), Step::Item(1));

        deque.pop();
        deque.pop();
        deque.push(99).unwrap();

        assert_eq!(state.step(&deque), Step::Item(99));
        assert_eq!(state.step(&deque), Step::Item(3));
    }

    /// A wrapped deque walks correctly, which is the one thing the ring is for.
    #[test]
    fn a_wrapped_deque_walks_front_to_back() {
        let mut deque = plain(4);
        for value in 1..=4 {
            deque.push(value).unwrap();
        }
        deque.shift();
        deque.shift();
        deque.push(5).unwrap();
        deque.push(6).unwrap();

        assert_eq!(deque.start(), 2);
        assert_eq!(present(&deque), vec![3, 4, 5, 6]);
        assert_eq!(deque.values().collect::<Vec<_>>(), vec![3, 4, 5, 6]);
        assert_eq!(deque.peek_first(), Some(3));
        assert_eq!(deque.peek_last(), Some(6));
    }

    /// `unshift` from an empty deque with `start == 0` writes the LAST slot,
    /// which is the wrap the upstream suite only reaches through a full one.
    #[test]
    fn unshift_from_the_zero_start_wraps_to_the_last_slot() {
        let mut deque = plain(3);

        assert_eq!(deque.unshift(7), Ok(1));
        assert_eq!(deque.start(), 2);
        assert_eq!(deque.items(), [None, None, Some(7)]);
        assert_eq!(present(&deque), vec![7]);
        assert_eq!(deque.peek_first(), Some(7));
        assert_eq!(deque.peek_last(), Some(7));
    }

    /// The degenerate ends.
    #[test]
    fn a_capacity_of_one_and_an_empty_deque_both_behave() {
        let mut one = plain(1);
        assert_eq!(one.push(7), Ok(1));
        assert_eq!(one.push(8), Err(Error::Full(End::Back, 1)));
        assert_eq!(one.peek_first(), Some(7));
        assert_eq!(one.peek_last(), Some(7));
        assert_eq!(one.get(0), Some(7));
        assert_eq!(one.get(1), None);
        assert_eq!(one.shift(), Some(7));
        assert_eq!(one.start(), 0);

        let empty = plain(4);
        assert_eq!(empty.peek_first(), None);
        assert_eq!(empty.peek_last(), None);
        assert_eq!(empty.get(0), None);
        assert_eq!(present(&empty), Vec::<i32>::new());
        assert_eq!(empty.values().collect::<Vec<_>>(), Vec::<i32>::new());
        assert!(empty.is_empty());
    }

    #[test]
    fn from_array_like_accepts_any_iterator() {
        let empty =
            FixedDeque::from_array_like(Backing::Holes, 3, std::iter::empty::<i32>()).unwrap();
        assert_eq!(empty.size(), 0);

        let mapped =
            FixedDeque::from_array_like(Backing::Holes, 3, (1..=3).map(|n| n * 10)).unwrap();
        assert_eq!(present(&mapped), vec![10, 20, 30]);
    }

    #[test]
    fn error_text_is_upstreams() {
        assert_eq!(Error::Capacity.to_string(), BAD_CAPACITY);
        assert!(MISSING_ARGUMENTS.contains("Array class"));
        assert!(CANNOT_GUESS_LENGTH.contains("could not guess iterable length"));
    }

    #[test]
    fn debug_reports_the_geometry() {
        assert!(format!("{:?}", plain(2)).starts_with("FixedDeque {"));
    }
}

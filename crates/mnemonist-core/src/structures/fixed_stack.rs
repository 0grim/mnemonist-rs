//! Port of upstream `fixed-stack.js` (242 LOC).
//!
//! A LIFO stack over a pre-allocated array of fixed capacity. The data
//! structure is `Stack` with a bound; everything interesting about it is in the
//! *bound*, and in the fact that the array is allocated once at construction
//! and never resized.
//!
//! Four things a naive port gets wrong, all of them upstream's, and only one of
//! them visible in `test/fixed-stack.js`.
//!
//! # 1. `forEach` walks `items.length`, not `size` (NOTES B-61)
//!
//! ```js
//! FixedStack.prototype.forEach = function (callback, scope) {
//!   scope = arguments.length > 1 ? scope : this;
//!
//!   for (var i = 0, l = this.items.length; i < l; i++)
//!     callback.call(scope, this.items[l - i - 1], i, this);
//! };
//! ```
//!
//! `this.items.length` is the **capacity**. Every other method in the file is
//! written against `this.size`. So a stack of size 2 and capacity 5 invokes the
//! callback **five** times, the first three with the unused slots — `undefined`
//! from an `Array`, `0` from a `Uint8Array`. Measured on Node 24.18.1:
//!
//! ```js
//! var s = new FixedStack(Array, 5); s.push(1); s.push(2);
//! s.forEach(function (v, i) { ... });
//! // (undefined, 0) (undefined, 1) (undefined, 2) (2, 3) (1, 4)
//! ```
//!
//! The original test file cannot see this: its one `forEach` block builds a
//! capacity-3 stack and pushes three items, so `items.length === size` and the
//! defect is exactly invisible. [`FixedStack::items_len`] and
//! [`FixedStack::lifo_slot`] are what a caller reproducing it needs, and they
//! are deliberately separate from the cursor's read below.
//!
//! # 2. `values()` freezes `this.size`, and `forEach` freezes `items.length`
//!
//! Two walks over the same array, in the same direction, with two different
//! bounds. They coincide on a full stack and nowhere else. Keeping them
//! separate is the whole of (1); collapsing them into one "iterate the stack"
//! helper is how the defect gets tidied away.
//!
//! # 3. `clear()` does not clear
//!
//! ```js
//! FixedStack.prototype.clear = function () { this.size = 0; };
//! ```
//!
//! The array is untouched, so every popped or cleared element stays reachable
//! through `items` and can be read back by anything that indexes past `size` —
//! which upstream's own `forEach` does. `pop()` is the same: it decrements
//! `size` and leaves the element in place. Neither is a leak upstream *notices*,
//! but both are observable, and a port that zeroed the slot would diverge.
//!
//! # 4. `from` never checks the iterable against the capacity
//!
//! ```js
//! if (iterables.isArrayLike(iterable)) {
//!   for (i = 0, l = iterable.length; i < l; i++)
//!     stack.items[i] = iterable[i];
//!   stack.size = l;
//!   return stack;
//! }
//! ```
//!
//! `l` is the *iterable's* length, not the capacity, and `size` is assigned
//! rather than counted. So `FixedStack.from([1, 2, 3], Array, 2)` yields a
//! stack whose `size` is 3 and whose capacity is 2 — and, because a plain
//! `Array` grows on an out-of-range store while a typed array drops it, the
//! two classes end up in genuinely different states. See
//! [`crate::structures::backing`].
//!
//! The `else` branch — the one that would handle a `Set` or a generator —
//! **cannot run at all**: it calls `iterables.forEach`, and `utils/iterables.js`
//! exports no such function. That is NOTES B-60, and it is reproduced at the
//! bridge, where the `TypeError` it raises exists.

use std::fmt;

use crate::cursor::{Cursor, Sequence};
use crate::structures::backing::Backing;

/// `arguments.length < 2`, verbatim.
pub const MISSING_ARGUMENTS: &str =
    "mnemonist/fixed-stack: expecting an Array class and a capacity.";

/// `typeof capacity !== 'number' || capacity <= 0`, verbatim.
pub const BAD_CAPACITY: &str = "mnemonist/fixed-stack: `capacity` should be a positive number.";

/// `FixedStack.from` with no capacity and an unguessable iterable, verbatim.
pub const CANNOT_GUESS_LENGTH: &str = "mnemonist/fixed-stack.from: could not guess iterable \
     length. Please provide desired capacity as last argument.";

/// What upstream throws, with the same text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `capacity <= 0`. The non-numeric half of upstream's test is a
    /// JavaScript-only notion and is checked at the bridge.
    Capacity,
    /// `#.push` on a full stack.
    Full(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capacity => formatter.write_str(BAD_CAPACITY),
            Self::Full(capacity) => write!(
                formatter,
                "mnemonist/fixed-stack.push: stack capacity ({capacity}) exceeded!"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// A LIFO stack of fixed capacity.
pub struct FixedStack<T> {
    /// `this.capacity`, which after construction is only ever *compared*
    /// against — never re-derived from `items.length`, because the two can
    /// diverge. See item 4 in the module docs.
    capacity: usize,
    backing: Backing<T>,
    /// `this.items`. `None` is JavaScript `undefined`: an `Array` hole, or a
    /// read past the end of a typed array.
    items: Vec<Option<T>>,
    size: usize,
}

impl<T: Clone> FixedStack<T> {
    /// `new FixedStack(ArrayClass, capacity)`.
    pub fn new(backing: Backing<T>, capacity: usize) -> Result<Self, Error> {
        // `capacity <= 0`. Zero is the only half of that a `usize` can express;
        // the negative and non-numeric halves are the bridge's.
        if capacity == 0 {
            return Err(Error::Capacity);
        }

        let items = backing.allocate(capacity);

        Ok(Self {
            capacity,
            backing,
            items,
            size: 0,
        })
    }

    /// `FixedStack.from(arrayLike, ArrayClass, capacity)` — the only branch of
    /// upstream's `from` that is reachable (see item 4 and NOTES B-60).
    ///
    /// Takes an [`IntoIterator`] rather than a slice, which is D-03: core takes
    /// the natural Rust shape and the JS-value coercion lives at the boundary.
    /// The *semantics* are upstream's index-by-index store, so an iterable
    /// longer than `capacity` behaves exactly as it does there — growing a
    /// `Holes` backing and being truncated by a `Filled` one — and `size`
    /// becomes the iterable's length either way.
    pub fn from_array_like(
        backing: Backing<T>,
        capacity: usize,
        values: impl IntoIterator<Item = T>,
    ) -> Result<Self, Error> {
        let mut stack = Self::new(backing, capacity)?;
        let mut length = 0;

        for (index, value) in values.into_iter().enumerate() {
            stack.backing.store(&mut stack.items, index, value);
            length = index + 1;
        }

        // `stack.size = l` — assigned, not counted, and never compared against
        // the capacity.
        stack.size = length;

        Ok(stack)
    }

    /// `#.clear` — sets `size` to zero and **leaves the array alone**.
    pub fn clear(&mut self) {
        self.size = 0;
    }

    /// `#.push` — returns the new size, or the capacity error.
    pub fn push(&mut self, item: T) -> Result<usize, Error> {
        if self.size == self.capacity {
            return Err(Error::Full(self.capacity));
        }

        let index = self.size;

        self.backing.store(&mut self.items, index, item);
        self.size += 1;

        Ok(self.size)
    }

    /// `#.pop` — `undefined` on an empty stack, and also `undefined` when the
    /// slot below `size` was never written.
    ///
    /// The second case is not hypothetical: `from` can leave `size` past the
    /// end of a typed array's storage, and upstream then returns `undefined`
    /// exactly as it does for an empty stack. Both are `None` here because both
    /// are the same JS value.
    pub fn pop(&mut self) -> Option<T> {
        if self.size == 0 {
            return None;
        }

        self.size -= 1;

        self.items.get(self.size).cloned().flatten()
    }

    /// `#.peek` — `this.items[this.size - 1]`, which upstream does not guard.
    ///
    /// On an empty stack it evaluates `items[-1]`, i.e. `undefined`;
    /// [`checked_sub`](usize::checked_sub) is the same answer without the
    /// negative index.
    pub fn peek(&self) -> Option<T> {
        let index = self.size.checked_sub(1)?;

        self.items.get(index).cloned().flatten()
    }

    /// `#.toArray` — newest first, one entry per `size`.
    ///
    /// `None` is `undefined`, which a caller building a real `Array` renders as
    /// an own `undefined` property and a caller building a typed array renders
    /// as that class's zero. Upstream writes `array[i] = this.items[l - i]`
    /// into a fresh `new this.ArrayClass(this.size)`, so both follow from
    /// leaving the slot unwritten.
    pub fn to_array(&self) -> Vec<Option<T>> {
        (0..self.size)
            .map(|index| self.items.get(self.size - 1 - index).cloned().flatten())
            .collect()
    }

    /// The backing array itself, oldest first, holes included.
    ///
    /// `items` is a **public property** upstream. The differential fuzzer
    /// observes it after every operation, which is what makes the debris left
    /// by `pop` and `clear` — and the growth of an over-filled `Array` —
    /// checkable directly rather than only through a later read.
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

    /// Whether the stack holds nothing. Not upstream; upstream writes
    /// `size === 0`.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// `this.items.length` — the bound `forEach` walks, and **not** `size`.
    ///
    /// Separate from [`size`](FixedStack::size) because upstream keeps them
    /// separate and they disagree on every stack that is not exactly full.
    /// See item 1 in the module docs.
    pub fn items_len(&self) -> usize {
        self.items.len()
    }

    /// One element of `forEach`'s walk: `this.items[l - i - 1]`, read **live**.
    ///
    /// Only `l` is frozen upstream; `this.items` is re-read on every iteration,
    /// so a callback that mutates the stack is visible to the reads that
    /// follow. [`Sequence::slot`], which serves `values()`, uses a different
    /// bound — that difference is upstream's, not a simplification here.
    pub fn lifo_slot(&self, frozen_len: usize, ordinal: usize) -> Option<T> {
        let index = frozen_len.checked_sub(ordinal + 1)?;

        self.items.get(index).cloned().flatten()
    }

    /// `#.values` — a fresh, non-restartable cursor, newest first.
    pub fn values(&self) -> Cursor<'_, Self> {
        Cursor::new(self)
    }
}

/// Newest first, over `this.size` elements frozen at creation.
///
/// ```js
/// FixedStack.prototype.values = function () {
///   var items = this.items, l = this.size, i = 0;
///   return new Iterator(function () {
///     if (i >= l) return {done: true};
///     var value = items[l - i - 1];
///     i++;
///     return {value: value, done: false};
///   });
/// };
/// ```
///
/// `l` is the frozen size; `items` is the array *object*, which this structure
/// — unlike `Stack` — never rebinds, so reading it back through `&self` is the
/// same array upstream captured.
impl<T: Clone> Sequence for FixedStack<T> {
    type Item = T;
    /// `l`, needed by `slot` to turn a step counter into an index.
    type Frozen = usize;

    fn freeze(&self) -> (usize, usize) {
        (self.size, self.size)
    }

    fn slot(&self, frozen: &usize, ordinal: usize) -> Option<T> {
        let index = frozen.checked_sub(ordinal + 1)?;

        self.items.get(index).cloned().flatten()
    }
}

impl<T: fmt::Debug> fmt::Debug for FixedStack<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FixedStack")
            .field("capacity", &self.capacity)
            .field("size", &self.size)
            .field("items", &self.items)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{CursorState, Step};

    fn plain(capacity: usize) -> FixedStack<i32> {
        FixedStack::new(Backing::Holes, capacity).expect("capacity is positive")
    }

    fn typed(capacity: usize) -> FixedStack<i32> {
        FixedStack::new(Backing::Filled(0), capacity).expect("capacity is positive")
    }

    fn filled(stack: &FixedStack<i32>) -> Vec<Option<i32>> {
        stack.to_array()
    }

    /// The upstream suite, ported 1:1, as the baseline the rest builds on.
    /// `test/fixed-stack.js`, 11 `it` blocks.
    #[test]
    fn reproduces_the_upstream_suite() {
        assert_eq!(
            FixedStack::<i32>::new(Backing::Holes, 0).unwrap_err(),
            Error::Capacity
        );

        let mut stack = plain(10);
        assert_eq!(stack.push(1), Ok(1));
        assert_eq!((stack.size(), stack.capacity()), (1, 10));

        let mut one = plain(1);
        assert_eq!(one.push(1), Ok(1));
        assert_eq!(one.push(2), Err(Error::Full(1)));

        let mut stack = plain(2);
        stack.push(2).unwrap();
        stack.push(3).unwrap();
        stack.clear();
        assert_eq!(stack.size(), 0);
        assert_eq!(filled(&stack), Vec::<Option<i32>>::new());

        let mut stack = plain(2);
        assert_eq!(stack.peek(), None);
        stack.push(1).unwrap();
        assert_eq!(stack.peek(), Some(1));
        stack.push(2).unwrap();
        assert_eq!(stack.peek(), Some(2));

        let mut stack = plain(3);
        for value in 1..=3 {
            stack.push(value).unwrap();
        }
        assert_eq!(stack.pop(), Some(3));
        assert_eq!(stack.pop(), Some(2));
        assert_eq!(stack.pop(), Some(1));
        assert_eq!(stack.pop(), None);

        // `should be possible to convert the stack to an array.`
        let mut stack = typed(3);
        for value in 1..=3 {
            stack.push(value).unwrap();
        }
        assert_eq!(filled(&stack), vec![Some(3), Some(2), Some(1)]);

        // `…from an arbitrary iterable.` and the two iterator blocks.
        let stack = FixedStack::from_array_like(Backing::Holes, 3, [1, 2, 3]).unwrap();
        assert_eq!(filled(&stack), vec![Some(3), Some(2), Some(1)]);

        let stack = FixedStack::from_array_like(Backing::Filled(0), 45, [1, 2, 3]).unwrap();
        assert_eq!(stack.values().collect::<Vec<_>>(), vec![3, 2, 1]);

        let stack = FixedStack::from_array_like(Backing::Filled(0), 5, [1, 2, 3]).unwrap();
        assert_eq!((stack.size(), stack.capacity()), (3, 5));
        let entries: Vec<(usize, i32)> = stack.values().enumerate().collect();
        assert_eq!(entries, vec![(0, 3), (1, 2), (2, 1)]);
    }

    /// B-61, the whole point of the module. `forEach`'s bound is the array's
    /// length, so an under-full stack hands the callback its unused slots —
    /// `undefined` from an `Array`, the class zero from a typed array — before
    /// any real element.
    ///
    /// Pinned against Node 24.18.1; see the module docs for the transcript.
    #[test]
    fn for_each_walks_the_capacity_and_not_the_size() {
        let mut stack = plain(5);
        stack.push(1).unwrap();
        stack.push(2).unwrap();

        assert_eq!(stack.items_len(), 5);
        assert_eq!(stack.size(), 2);

        let bound = stack.items_len();
        let seen: Vec<Option<i32>> = (0..bound).map(|i| stack.lifo_slot(bound, i)).collect();

        assert_eq!(seen, vec![None, None, None, Some(2), Some(1)]);

        // The same stack over a typed class: the unused slots read as its zero,
        // not as `undefined`.
        let mut stack = typed(5);
        stack.push(1).unwrap();
        stack.push(2).unwrap();

        let bound = stack.items_len();
        let seen: Vec<Option<i32>> = (0..bound).map(|i| stack.lifo_slot(bound, i)).collect();

        assert_eq!(seen, vec![Some(0), Some(0), Some(0), Some(2), Some(1)]);
    }

    /// …and this is why the original suite cannot see it: at `size == capacity`
    /// the two bounds agree, and that is the only shape its `forEach` block
    /// builds.
    #[test]
    fn for_each_agrees_with_values_only_on_a_full_stack() {
        let mut stack = plain(3);
        for value in 1..=3 {
            stack.push(value).unwrap();
        }

        let bound = stack.items_len();
        let by_for_each: Vec<Option<i32>> = (0..bound).map(|i| stack.lifo_slot(bound, i)).collect();
        let by_values: Vec<Option<i32>> = stack.values().map(Some).collect();

        assert_eq!(by_for_each, by_values);

        stack.pop();

        let bound = stack.items_len();
        let by_for_each: Vec<Option<i32>> = (0..bound).map(|i| stack.lifo_slot(bound, i)).collect();
        let by_values: Vec<Option<i32>> = stack.values().map(Some).collect();

        assert_ne!(by_for_each, by_values);
        assert_eq!(by_for_each, vec![Some(3), Some(2), Some(1)]);
        assert_eq!(by_values, vec![Some(2), Some(1)]);
    }

    /// `clear` and `pop` both leave the element in the array. Nothing upstream
    /// asserts it, and `forEach` reads it back.
    #[test]
    fn clear_and_pop_leave_the_elements_in_place() {
        let mut stack = plain(3);
        for value in 1..=3 {
            stack.push(value).unwrap();
        }

        stack.pop();
        assert_eq!(stack.items(), [Some(1), Some(2), Some(3)]);

        stack.clear();
        assert_eq!(stack.size(), 0);
        assert_eq!(stack.items(), [Some(1), Some(2), Some(3)]);

        // …and the debris is reachable through the one method that indexes past
        // `size`.
        let bound = stack.items_len();
        let seen: Vec<Option<i32>> = (0..bound).map(|i| stack.lifo_slot(bound, i)).collect();
        assert_eq!(seen, vec![Some(3), Some(2), Some(1)]);
    }

    /// A push after a clear overwrites slot 0 rather than appending, because
    /// `push` indexes by `size`.
    #[test]
    fn a_push_after_clear_reuses_the_array_from_the_bottom() {
        let mut stack = plain(3);
        stack.push(1).unwrap();
        stack.push(2).unwrap();
        stack.clear();
        stack.push(9).unwrap();

        assert_eq!(stack.items(), [Some(9), Some(2), None]);
        assert_eq!(filled(&stack), vec![Some(9)]);
    }

    /// `push` on a full stack throws and changes **nothing** — the guard is
    /// before the store, so the array and the size are both untouched.
    #[test]
    fn a_refused_push_leaves_the_stack_untouched() {
        let mut stack = plain(2);
        stack.push(1).unwrap();
        stack.push(2).unwrap();

        assert_eq!(stack.push(3), Err(Error::Full(2)));
        assert_eq!(stack.size(), 2);
        assert_eq!(stack.items(), [Some(1), Some(2)]);
        assert_eq!(
            stack.push(3).unwrap_err().to_string(),
            "mnemonist/fixed-stack.push: stack capacity (2) exceeded!"
        );
    }

    /// Item 4: `from` assigns `size` from the *iterable*, so an oversized one
    /// grows a plain `Array` straight past the capacity it was given.
    ///
    /// Pinned against Node 24.18.1:
    /// `FixedStack.from([1,2,3], Array, 2)` → `size 3`, `items [1,2,3]`,
    /// `toArray() [3,2,1]`.
    #[test]
    fn from_an_oversized_array_like_overflows_a_plain_array() {
        let stack = FixedStack::from_array_like(Backing::Holes, 2, [1, 2, 3]).unwrap();

        assert_eq!((stack.size(), stack.capacity()), (3, 2));
        assert_eq!(stack.items(), [Some(1), Some(2), Some(3)]);
        assert_eq!(stack.items_len(), 3);
        assert_eq!(filled(&stack), vec![Some(3), Some(2), Some(1)]);
    }

    /// The same call against a typed class drops the overflow instead, which
    /// leaves `size` past the end of the storage — and every read of the
    /// missing slot is then `undefined`.
    ///
    /// Pinned against Node 24.18.1:
    /// `FixedStack.from([1,2,3], Uint8Array, 2)` → `size 3`,
    /// `items Uint8Array(2) [1,2]`, `values() [undefined, 2, 1]`,
    /// `toArray() Uint8Array(3) [0, 2, 1]`, `pop() undefined`.
    #[test]
    fn from_an_oversized_array_like_is_truncated_by_a_typed_class() {
        let mut stack = FixedStack::from_array_like(Backing::Filled(0), 2, [1, 2, 3]).unwrap();

        assert_eq!((stack.size(), stack.items_len()), (3, 2));
        assert_eq!(stack.items(), [Some(1), Some(2)]);
        // `toArray` leaves the missing slot unwritten, which in a fresh typed
        // array is the class zero — upstream's `Uint8Array(3) [0, 2, 1]`.
        assert_eq!(filled(&stack), vec![None, Some(2), Some(1)]);
        assert_eq!(stack.pop(), None);
        assert_eq!(stack.size(), 2);
    }

    /// The gap the truncation above opens is visible to the cursor as
    /// `Step::Gap` — `{done: false, value: undefined}` — not as an early end.
    /// Upstream: `[undefined, 2, 1]`.
    #[test]
    fn a_truncated_from_makes_the_cursor_yield_undefined() {
        let stack = FixedStack::from_array_like(Backing::Filled(0), 2, [1, 2, 3]).unwrap();
        let mut cursor = stack.values();

        assert_eq!(cursor.step(), Step::Gap);
        assert_eq!(cursor.step(), Step::Item(2));
        assert_eq!(cursor.step(), Step::Item(1));
        assert_eq!(cursor.step(), Step::Done);
    }

    /// D-06 / D-07: the cursor is stateful and does not restart, but the stack
    /// hands out a fresh one every time.
    #[test]
    fn cursors_do_not_restart_but_the_stack_can_be_walked_again() {
        let stack = FixedStack::from_array_like(Backing::Holes, 3, [1, 2, 3]).unwrap();

        let mut cursor = stack.values();
        assert_eq!(cursor.next(), Some(3));
        assert_eq!(cursor.by_ref().collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(cursor.collect::<Vec<_>>(), Vec::<i32>::new());

        assert_eq!(stack.values().collect::<Vec<_>>(), vec![3, 2, 1]);
    }

    /// D-08, the frozen half: `l` is `this.size` at creation, so a later `push`
    /// is invisible however much room there was.
    #[test]
    fn a_push_during_iteration_is_not_visible_to_the_cursor() {
        let mut stack = plain(5);
        stack.push(1).unwrap();
        stack.push(2).unwrap();

        let mut state = CursorState::open(&stack);

        assert_eq!(state.step(&stack), Step::Item(2));
        stack.push(9).unwrap();

        assert_eq!(state.step(&stack), Step::Item(1));
        assert_eq!(state.step(&stack), Step::Done);
    }

    /// D-08, the live half: `pop` moves `size` but not the array, so a cursor
    /// opened first still reads the popped element out of the slot it is
    /// sitting in. Nothing shortens, so there is no gap either.
    #[test]
    fn a_pop_during_iteration_still_yields_the_popped_element() {
        let mut stack = plain(3);
        for value in 1..=3 {
            stack.push(value).unwrap();
        }

        let mut state = CursorState::open(&stack);
        stack.pop();

        assert_eq!(state.step(&stack), Step::Item(3));
        assert_eq!(state.step(&stack), Step::Item(2));
        assert_eq!(state.step(&stack), Step::Item(1));
        assert_eq!(state.step(&stack), Step::Done);
    }

    /// …and a `push` after that `pop` **is** visible, because it overwrites the
    /// slot the cursor has not reached yet. Element writes are live; only the
    /// length is frozen.
    #[test]
    fn an_overwrite_ahead_of_the_cursor_is_visible() {
        let mut stack = plain(3);
        for value in 1..=3 {
            stack.push(value).unwrap();
        }

        let mut state = CursorState::open(&stack);
        assert_eq!(state.step(&stack), Step::Item(3));

        stack.pop();
        stack.pop();
        stack.push(99).unwrap();

        // Ordinal 1 is `items[1]`, which the push above just rewrote.
        assert_eq!(state.step(&stack), Step::Item(99));
        assert_eq!(state.step(&stack), Step::Item(1));
    }

    /// A `clear` mid-walk changes nothing at all: the cursor's bound is frozen
    /// and `clear` does not touch the array. Contrast `Stack`, where `clear`
    /// rebinds the array and detaches the cursor entirely.
    #[test]
    fn a_clear_during_iteration_is_invisible_because_clear_does_nothing_to_the_array() {
        let mut stack = plain(3);
        for value in 1..=3 {
            stack.push(value).unwrap();
        }

        let mut state = CursorState::open(&stack);
        stack.clear();

        assert_eq!(state.step(&stack), Step::Item(3));
        assert_eq!(state.step(&stack), Step::Item(2));
        assert_eq!(state.step(&stack), Step::Item(1));
        assert_eq!(state.step(&stack), Step::Done);
    }

    /// The degenerate ends: capacity one, and an empty stack.
    #[test]
    fn a_capacity_of_one_and_an_empty_stack_both_behave() {
        let mut one = plain(1);
        assert_eq!(one.push(7), Ok(1));
        assert_eq!(one.push(8), Err(Error::Full(1)));
        assert_eq!(one.peek(), Some(7));
        assert_eq!(one.pop(), Some(7));
        assert_eq!(one.pop(), None);

        let empty = plain(4);
        assert_eq!(empty.peek(), None);
        assert_eq!(filled(&empty), Vec::<Option<i32>>::new());
        assert_eq!(empty.values().collect::<Vec<_>>(), Vec::<i32>::new());
        assert!(empty.is_empty());
    }

    /// `from` on an empty iterable leaves an empty stack with the array
    /// allocated, and on a non-slice iterator behaves identically — D-03's
    /// claim that core takes any `IntoIterator`.
    #[test]
    fn from_array_like_accepts_any_iterator() {
        let empty =
            FixedStack::from_array_like(Backing::Holes, 3, std::iter::empty::<i32>()).unwrap();
        assert_eq!(empty.size(), 0);
        assert_eq!(empty.items_len(), 3);

        let mapped =
            FixedStack::from_array_like(Backing::Holes, 3, (1..=3).map(|n| n * 10)).unwrap();
        assert_eq!(mapped.to_array(), vec![Some(30), Some(20), Some(10)]);
    }

    /// Duplicates are values, not identities.
    #[test]
    fn duplicates_are_kept() {
        let stack = FixedStack::from_array_like(Backing::Holes, 3, [7, 7, 7]).unwrap();

        assert_eq!(stack.size(), 3);
        assert_eq!(stack.to_array(), vec![Some(7), Some(7), Some(7)]);
    }

    /// The two error strings, verbatim, so a bridge that reformats one is
    /// caught here rather than by a mocha regex.
    #[test]
    fn error_text_is_upstreams() {
        assert_eq!(Error::Capacity.to_string(), BAD_CAPACITY);
        assert_eq!(
            Error::Full(7).to_string(),
            "mnemonist/fixed-stack.push: stack capacity (7) exceeded!"
        );
        assert!(MISSING_ARGUMENTS.contains("Array class"));
        assert!(CANNOT_GUESS_LENGTH.contains("could not guess iterable length"));
    }

    #[test]
    fn debug_reports_the_geometry() {
        let stack = plain(2);

        assert!(format!("{stack:?}").starts_with("FixedStack {"));
    }
}

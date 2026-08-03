//! Port of upstream `stack.js` (210 LOC).
//!
//! A LIFO stack over a growable array. Trivial as a data structure, which is
//! exactly why it was chosen to host the `forEach` boundary coercion
//! : if the dispatch is wrong, it shows up here rather than
//! inside a module that needs four primitives at once.
//!
//! Three things a naive port gets wrong, all of them upstream's, none of them
//! visible in `test/stack.js`.
//!
//! # 1. `items` is a JavaScript array, and `clear()` **rebinds** it
//!
//! ```js
//! Stack.prototype.clear = function () {
//!   this.items = [];        // a NEW array, not items.length = 0
//!   this.size = 0;
//! };
//! ```
//!
//! `Stack.prototype.values` captures `var items = this.items` — the array
//! *object*. So a cursor opened before a `clear()` keeps walking the array it
//! captured and is completely unaffected by the clear, while `pop()`, which
//! mutates the same array in place, **is** visible to it. A `Vec<T>` cannot
//! tell those two apart: both would look like "the buffer got shorter".
//!
//! That is why the backing store is [`Rc<RefCell<Vec<T>>>`](std::rc::Rc) rather
//! than a plain `Vec`. It is not interior mutability for its own sake — the
//! mutators still take `&mut self` — it is the *reference* half of a JS array,
//! which is the only part of `Array` semantics this module actually depends on.
//!
//! # 2. `values()` freezes `items.length`, not `this.size` (DIV-PROJ-19, NOTES DIV-PROJ-9)
//!
//! Every other structure freezes `this.size`. `Stack` freezes the array's
//! length and then counts *down* through it (`items[l - i - 1]`). The two
//! coincide on every path the public API offers, so the inconsistency is
//! latent rather than active — but normalising it would be an unforced
//! assumption, so `size` and the array length are kept as separate quantities
//! here exactly as they are upstream.
//!
//! # 3. `toArray()` reverses, `forEach` reverses, the cursor reverses
//!
//! All three walk newest-first, and each does it by different arithmetic.
//! [`Sequence`] was built with a reversed walk in mind — its `ordinal` is a
//! step counter, not an index — so the reversal is nine lines here rather than
//! a second cursor type.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::cursor::{Cursor, Sequence};

/// The shared backing store: the Rust half of `this.items`.
///
/// Shared because a cursor captures the array *object*, and rebinding
/// `this.items` must leave that capture intact (see the module docs).
type Items<T> = Rc<RefCell<Vec<T>>>;

/// What `Stack.prototype.values` captures at construction time.
///
/// Both fields are upstream's, one line apart:
///
/// ```js
/// var items = this.items,      // the array OBJECT
///     l = items.length,        // its length, frozen
///     i = 0;
/// ```
pub struct StackCapture<T> {
    items: Items<T>,
    len: usize,
}

/// A LIFO stack.
///
/// `size` is tracked separately from the backing array's length because
/// upstream tracks it separately; see the module docs.
pub struct Stack<T> {
    items: Items<T>,
    size: usize,
}

impl<T> Stack<T> {
    /// `new Stack()`, which upstream implements as `this.clear()`.
    pub fn new() -> Self {
        Self {
            items: Rc::new(RefCell::new(Vec::new())),
            size: 0,
        }
    }

    /// `#.clear` — **rebinds** the backing array rather than emptying it.
    ///
    /// Any cursor already open keeps the array it captured and is unaffected.
    /// That is upstream's `this.items = []`, and reproducing it is the reason
    /// `Items` is refcounted.
    pub fn clear(&mut self) {
        self.items = Rc::new(RefCell::new(Vec::new()));
        self.size = 0;
    }

    /// `#.push` — returns the new size, as upstream's `return ++this.size`.
    pub fn push(&mut self, item: T) -> usize {
        self.items.borrow_mut().push(item);
        self.size += 1;

        self.size
    }

    /// `#.pop` — mutates the backing array **in place**, so an open cursor
    /// sees the hole.
    pub fn pop(&mut self) -> Option<T> {
        if self.size == 0 {
            return None;
        }

        self.size -= 1;

        self.items.borrow_mut().pop()
    }

    /// `#.size`.
    pub fn size(&self) -> usize {
        self.size
    }

    /// `this.items.length`, which is *not* the same quantity as
    /// [`size`](Stack::size) even though nothing in the public API can pull
    /// them apart. See the module docs and DIV-PROJ-19.
    pub fn items_len(&self) -> usize {
        self.items.borrow().len()
    }

    /// Whether the stack holds nothing. Not upstream; upstream callers write
    /// `size === 0`.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
}

impl<T: Clone> Stack<T> {
    /// `#.peek` — `this.items[this.size - 1]`, `undefined` when empty.
    ///
    /// Upstream does not guard: on an empty stack it evaluates `items[-1]`,
    /// which is `undefined` rather than an error. [`checked_sub`](usize) is
    /// the same answer without the negative index.
    pub fn peek(&self) -> Option<T> {
        let index = self.size.checked_sub(1)?;

        self.items.borrow().get(index).cloned()
    }

    /// `#.toArray` — newest first.
    ///
    /// Upstream sizes the result from `this.size` and reads
    /// `items[size - 1 - i]`. Those indices are all in range whenever
    /// `size == items.length`, which every public path maintains; the
    /// `filter_map` is what an out-of-range read would be, and it is
    /// unreachable rather than defensive.
    pub fn to_vec(&self) -> Vec<T> {
        let items = self.items.borrow();

        debug_assert_eq!(self.size, items.len(), "size and items.length diverged");

        (0..self.size)
            .filter_map(|i| items.get(self.size - i - 1).cloned())
            .collect()
    }

    /// The backing array itself, oldest first.
    ///
    /// `items` is a **public property** upstream, so this exposes exactly the
    /// surface a JS caller already has. The differential fuzzer observes it
    /// after every operation, which is what makes the array-rebinding of
    /// `clear()` checkable directly rather than only through its effect on an
    /// open cursor.
    pub fn items(&self) -> Vec<T> {
        self.items.borrow().clone()
    }

    /// One element of a **live** newest-first walk: `this.items[l - i - 1]`.
    ///
    /// This is `Stack.prototype.forEach`'s read, and it deliberately goes
    /// through `self.items` — the array bound *now* — rather than through a
    /// capture. Upstream freezes only the loop bound:
    ///
    /// ```js
    /// for (var i = 0, l = this.items.length; i < l; i++)
    ///   callback.call(scope, this.items[l - i - 1], i, this);
    /// ```
    ///
    /// so a callback that calls `clear()` makes every later read land in the
    /// *new* array. [`Sequence::slot`], which serves `values()`, reads the
    /// captured array instead. The difference between the two is upstream's.
    pub fn lifo_slot(&self, frozen_len: usize, ordinal: usize) -> Option<T> {
        let index = frozen_len.checked_sub(ordinal + 1)?;

        self.items.borrow().get(index).cloned()
    }

    /// `#.values` — a fresh, non-restartable cursor, newest first.
    pub fn values(&self) -> Cursor<'_, Self> {
        Cursor::new(self)
    }
}

/// The Rust-caller form of `Stack.from(iterable)`.
///
/// DIV-QUEUE-1: core accepts any [`IntoIterator`]; the five-branch coercion that turns
/// an arbitrary *JavaScript* value into one lives at the napi boundary, where
/// JS values exist. A Rust caller writing `Stack::from_iter(vec)` gets the
/// natural thing and never meets the dispatch.
impl<T> FromIterator<T> for Stack<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iterable: I) -> Self {
        let mut stack = Self::new();

        for item in iterable {
            stack.push(item);
        }

        stack
    }
}

impl<T> Default for Stack<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Newest first, over the array captured at cursor creation.
impl<T: Clone> Sequence for Stack<T> {
    type Item = T;
    type Frozen = StackCapture<T>;

    fn freeze(&self) -> (StackCapture<T>, usize) {
        let len = self.items.borrow().len();

        (
            StackCapture {
                items: Rc::clone(&self.items),
                len,
            },
            len,
        )
    }

    fn slot(&self, frozen: &StackCapture<T>, ordinal: usize) -> Option<T> {
        let index = frozen.len.checked_sub(ordinal + 1)?;

        frozen.items.borrow().get(index).cloned()
    }
}

impl<T: fmt::Debug> fmt::Debug for Stack<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Stack")
            .field("size", &self.size)
            .field("items", &self.items.borrow())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{CursorState, Step};

    /// The upstream suite, ported 1:1, as the baseline the rest builds on.
    /// `test/stack.js`, 11 `it` blocks.
    #[test]
    fn reproduces_the_upstream_suite() {
        let mut stack: Stack<i32> = Stack::new();
        stack.push(1);
        assert_eq!(stack.size(), 1);

        stack.push(2);
        stack.clear();
        assert_eq!(stack.size(), 0);
        assert_eq!(stack.to_vec(), Vec::<i32>::new());

        assert_eq!(stack.peek(), None);
        stack.push(1);
        assert_eq!(stack.peek(), Some(1));
        stack.push(2);
        assert_eq!(stack.peek(), Some(2));

        let mut stack: Stack<i32> = [1, 2, 3].into_iter().collect();
        assert_eq!(stack.pop(), Some(3));
        assert_eq!(stack.pop(), Some(2));
        assert_eq!(stack.pop(), Some(1));
        assert_eq!(stack.pop(), None);

        let stack: Stack<i32> = [1, 2, 3].into_iter().collect();
        assert_eq!(stack.to_vec(), vec![3, 2, 1]);
        assert_eq!(stack.values().collect::<Vec<_>>(), vec![3, 2, 1]);
    }

    /// `push` returns the new size. Upstream's `return ++this.size` is never
    /// asserted by the suite.
    #[test]
    fn push_returns_the_new_size() {
        let mut stack = Stack::new();

        assert_eq!(stack.push('a'), 1);
        assert_eq!(stack.push('b'), 2);
        assert_eq!(stack.push('c'), 3);
    }

    /// `pop` on an empty stack returns `undefined` **and leaves `size` at 0** —
    /// upstream returns early precisely so the decrement does not run.
    #[test]
    fn popping_an_empty_stack_does_not_move_the_size() {
        let mut stack: Stack<i32> = Stack::new();

        assert_eq!(stack.pop(), None);
        assert_eq!(stack.pop(), None);
        assert_eq!(stack.size(), 0);
        assert_eq!(stack.items_len(), 0);
    }

    /// DIV-PROJ-19 / DIV-PROJ-9: `values()` is defined against `items.length`, `toArray()`
    /// against `size`. They agree here, and this test is what would notice if
    /// a future change made them disagree.
    #[test]
    fn size_and_the_backing_length_track_each_other() {
        let mut stack = Stack::new();

        for item in 0..5 {
            stack.push(item);
        }
        assert_eq!((stack.size(), stack.items_len()), (5, 5));

        stack.pop();
        assert_eq!((stack.size(), stack.items_len()), (4, 4));

        stack.clear();
        assert_eq!((stack.size(), stack.items_len()), (0, 0));
    }

    /// DIV-STACK-1: a cursor is stateful, but the collection hands out a fresh one
    /// every time — the two halves of the `Symbol.iterator` split (DIV-STACK-2).
    #[test]
    fn cursors_do_not_restart_but_the_stack_can_be_walked_again() {
        let stack: Stack<i32> = [1, 2, 3].into_iter().collect();

        let mut cursor = stack.values();
        assert_eq!(cursor.next(), Some(3));
        assert_eq!(cursor.by_ref().collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(cursor.collect::<Vec<_>>(), Vec::<i32>::new());

        assert_eq!(stack.values().collect::<Vec<_>>(), vec![3, 2, 1]);
    }

    /// A `push` during iteration is invisible: `l` is frozen at creation.
    #[test]
    fn a_push_during_iteration_is_not_visible_to_the_cursor() {
        let mut stack: Stack<i32> = [1, 2].into_iter().collect();
        let mut state = CursorState::open(&stack);

        assert_eq!(state.step(&stack), Step::Item(2));
        stack.push(9);

        // Ordinal 1 is `items[2 - 1 - 1]` = `items[0]` = 1, and then the frozen
        // length of 2 ends the walk with the 9 never seen.
        assert_eq!(state.step(&stack), Step::Item(1));
        assert_eq!(state.step(&stack), Step::Done);
    }

    /// A `pop` during iteration **is** visible, because it shortens the very
    /// array the cursor captured. The hole is `Step::Gap` — `undefined` in JS.
    #[test]
    fn a_pop_during_iteration_opens_a_gap_at_the_top_of_the_walk() {
        let mut stack: Stack<i32> = [1, 2, 3].into_iter().collect();
        let mut state = CursorState::open(&stack);

        stack.pop();

        // Ordinal 0 wants `items[2]`, which the pop removed.
        assert_eq!(state.step(&stack), Step::Gap);
        assert_eq!(state.step(&stack), Step::Item(2));
        assert_eq!(state.step(&stack), Step::Item(1));
        assert_eq!(state.step(&stack), Step::Done);
    }

    /// The whole reason the backing store is refcounted: `clear()` rebinds
    /// `this.items`, so a cursor opened first keeps yielding the **old**
    /// contents. A `Vec<T>` would have reported three gaps here, and no
    /// upstream test would have noticed.
    #[test]
    fn clear_rebinds_the_array_and_leaves_an_open_cursor_untouched() {
        let mut stack: Stack<i32> = [1, 2, 3].into_iter().collect();
        let mut state = CursorState::open(&stack);

        assert_eq!(state.step(&stack), Step::Item(3));
        stack.clear();

        assert_eq!(stack.size(), 0);
        assert_eq!(state.step(&stack), Step::Item(2));
        assert_eq!(state.step(&stack), Step::Item(1));
        assert_eq!(state.step(&stack), Step::Done);
    }

    /// …and pushes after that `clear` go to the new array, so they stay
    /// invisible to the detached cursor no matter how many there are.
    #[test]
    fn a_cursor_detached_by_clear_never_sees_the_new_array() {
        let mut stack: Stack<i32> = [1, 2].into_iter().collect();
        let mut state = CursorState::open(&stack);

        stack.clear();
        for item in 100..110 {
            stack.push(item);
        }

        assert_eq!(state.step(&stack), Step::Item(2));
        assert_eq!(state.step(&stack), Step::Item(1));
        assert_eq!(state.step(&stack), Step::Done);
        assert_eq!(stack.to_vec(), (100..110).rev().collect::<Vec<_>>());
    }

    /// `forEach` reads live, `values()` reads the capture. Same stack, same
    /// moment, different answers — and both are upstream.
    #[test]
    fn for_each_reads_the_live_array_where_the_cursor_reads_the_capture() {
        let mut stack: Stack<i32> = [1, 2, 3].into_iter().collect();
        let mut state = CursorState::open(&stack);

        stack.clear();

        // `lifo_slot` is what `forEach` calls: the array bound now is empty.
        assert_eq!(stack.lifo_slot(3, 0), None);
        // The cursor's capture still holds all three.
        assert_eq!(state.step(&stack), Step::Item(3));
    }

    /// `peek` never mutates and never consumes, however often it is called.
    #[test]
    fn peek_is_a_pure_read() {
        let mut stack = Stack::new();
        stack.push("only");

        assert_eq!(stack.peek(), Some("only"));
        assert_eq!(stack.peek(), Some("only"));
        assert_eq!(stack.size(), 1);
    }

    /// An empty stack is an immediately-done walk and an empty array, with no
    /// special-casing anywhere.
    #[test]
    fn an_empty_stack_iterates_zero_times() {
        let stack: Stack<i32> = Stack::new();

        assert_eq!(stack.to_vec(), Vec::<i32>::new());
        assert_eq!(stack.values().collect::<Vec<_>>(), Vec::<i32>::new());
        assert_eq!(stack.peek(), None);
    }

    /// `from` on an empty iterable, and on an iterable that is not a slice —
    /// DIV-QUEUE-1's claim that core takes any `IntoIterator`.
    #[test]
    fn from_iter_accepts_any_iterator() {
        let empty: Stack<i32> = std::iter::empty().collect();
        assert_eq!(empty.size(), 0);

        let mapped: Stack<i32> = (1..=3).map(|n| n * 10).collect();
        assert_eq!(mapped.to_vec(), vec![30, 20, 10]);
    }

    /// Duplicates are values, not identities: a stack is not a set.
    #[test]
    fn duplicates_are_kept() {
        let stack: Stack<i32> = [7, 7, 7].into_iter().collect();

        assert_eq!(stack.size(), 3);
        assert_eq!(stack.to_vec(), vec![7, 7, 7]);
    }
}

//! Port of upstream `queue.js` (215 LOC).
//!
//! A FIFO queue over a growable array plus a read offset, with the array
//! compacted whenever the dead prefix reaches half its length. Structurally
//! `stack.js`'s twin, and deliberately ported straight after it — same
//! `forEach` boundary, same cursor machinery, one interesting difference each.
//!
//! # The difference that matters: the cursor's end is **live**
//!
//! ```js
//! Queue.prototype.values = function () {
//!   var items = this.items,
//!       i = this.offset;                       // offset FROZEN
//!
//!   return new Iterator(function () {
//!     if (i >= items.length) return {done: true};   // length RE-READ, every step
//!     ...
//!   });
//! };
//! ```
//!
//! `Stack.prototype.values`, four files away and otherwise identical in shape,
//! writes `l = items.length` once. So a queue that grows while a cursor sits at
//! its end **keeps going**, and because obliterator's `Iterator` has no `done`
//! flag of its own, a walk that already reported `{done: true}` resumes. That
//! is [`Sequence::limit`], and it exists for this module.
//!
//! # The compaction rebinds the array, which detaches open cursors
//!
//! ```js
//! if (++this.offset * 2 >= this.items.length) {
//!   this.items = this.items.slice(this.offset);   // a NEW array
//!   this.offset = 0;
//! }
//! ```
//!
//! A cursor captured the *old* array and its own frozen offset, so after a
//! compaction it is walking a snapshot the queue no longer owns — and the
//! elements it yields include ones the queue has already dequeued. Reproducing
//! that is why the backing store is [`Rc<RefCell<Vec<T>>>`](std::rc::Rc)
//! rather than a `Vec`, the same call as in [`stack`](super::stack).
//!
//! # `dequeue` does not remove anything
//!
//! It reads `items[offset]` and advances `offset`. The element stays in the
//! array until a compaction drops it, which is why `dequeue` here returns a
//! **clone** rather than moving the value out: moving it would leave a hole
//! that upstream does not have, and an older cursor can still legitimately
//! yield that element.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::cursor::{Cursor, Sequence};

/// The shared backing store: the Rust half of `this.items`.
type Items<T> = Rc<RefCell<Vec<T>>>;

/// What `Queue.prototype.values` captures at construction time: the array
/// *object* and the offset. Notably **not** the length.
pub struct QueueCapture<T> {
    items: Items<T>,
    offset: usize,
}

/// A FIFO queue.
pub struct Queue<T> {
    items: Items<T>,
    offset: usize,
    size: usize,
}

impl<T> Queue<T> {
    /// `new Queue()`, which upstream implements as `this.clear()`.
    pub fn new() -> Self {
        Self {
            items: Rc::new(RefCell::new(Vec::new())),
            offset: 0,
            size: 0,
        }
    }

    /// `#.clear` — rebinds the backing array; open cursors keep the old one.
    pub fn clear(&mut self) {
        self.items = Rc::new(RefCell::new(Vec::new()));
        self.offset = 0;
        self.size = 0;
    }

    /// `#.enqueue` — returns the new size, as upstream's `return ++this.size`.
    pub fn enqueue(&mut self, item: T) -> usize {
        self.items.borrow_mut().push(item);
        self.size += 1;

        self.size
    }

    /// `#.size`.
    pub fn size(&self) -> usize {
        self.size
    }

    /// `#.offset` — the index of the front element inside `items`.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// `this.items.length`, including the dead prefix a compaction has not yet
    /// dropped. Always `offset + size`.
    pub fn items_len(&self) -> usize {
        self.items.borrow().len()
    }

    /// Whether the queue holds nothing. Not upstream; upstream writes
    /// `!this.size`.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
}

impl<T: Clone> Queue<T> {
    /// `#.dequeue` — read the front, advance, compact when the dead prefix
    /// reaches half the array.
    ///
    /// The clone is load-bearing, not a convenience: upstream leaves the
    /// element where it is, and a cursor opened before this call still has a
    /// frozen offset that can reach it.
    pub fn dequeue(&mut self) -> Option<T> {
        if self.size == 0 {
            return None;
        }

        let item = self.items.borrow().get(self.offset).cloned();

        self.offset += 1;

        // `++this.offset * 2 >= this.items.length`. Note the comparison is
        // against the array's length, dead prefix included, not against `size`.
        if self.offset.saturating_mul(2) >= self.items.borrow().len() {
            let tail = self
                .items
                .borrow()
                .get(self.offset..)
                .map(<[T]>::to_vec)
                .unwrap_or_default();

            self.items = Rc::new(RefCell::new(tail));
            self.offset = 0;
        }

        self.size -= 1;

        item
    }

    /// `#.peek` — the front element, `undefined` when empty.
    ///
    /// Upstream guards on `!this.size` and returns early, so an empty queue
    /// never reads `items[offset]` at all.
    pub fn peek(&self) -> Option<T> {
        if self.size == 0 {
            return None;
        }

        self.items.borrow().get(self.offset).cloned()
    }

    /// `#.toArray` — `this.items.slice(this.offset)`, oldest first.
    pub fn to_vec(&self) -> Vec<T> {
        self.items
            .borrow()
            .get(self.offset..)
            .map(<[T]>::to_vec)
            .unwrap_or_default()
    }

    /// The backing array itself, dead prefix included.
    ///
    /// `items` is a **public property** upstream. The differential fuzzer
    /// observes it after every operation, which is what makes the compaction —
    /// invisible through `toArray` — checkable directly.
    pub fn items(&self) -> Vec<T> {
        self.items.borrow().clone()
    }

    /// One element of a **live** read: `this.items[index]`, absolute index.
    ///
    /// This is `Queue.prototype.forEach`'s read. Upstream freezes the loop
    /// bounds but re-reads `this.items` on every iteration:
    ///
    /// ```js
    /// for (var i = this.offset, j = 0, l = this.items.length; i < l; i++, j++)
    ///   callback.call(scope, this.items[i], j, this);
    /// ```
    ///
    /// so a callback that dequeues far enough to trigger a compaction sends
    /// every later read into the *new* array under the old index. That is a
    /// different behaviour from `values()`, which holds the array it captured.
    pub fn slot(&self, index: usize) -> Option<T> {
        self.items.borrow().get(index).cloned()
    }

    /// `#.values` — a fresh, non-restartable cursor, oldest first.
    pub fn values(&self) -> Cursor<'_, Self> {
        Cursor::new(self)
    }
}

/// The Rust-caller form of `Queue.from(iterable)`; see D-03 and
/// [`stack`](super::stack).
impl<T> FromIterator<T> for Queue<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iterable: I) -> Self {
        let mut queue = Self::new();

        for item in iterable {
            queue.enqueue(item);
        }

        queue
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Oldest first, over the array captured at cursor creation, ending at that
/// array's length **as of each step**.
impl<T: Clone> Sequence for Queue<T> {
    type Item = T;
    type Frozen = QueueCapture<T>;

    fn freeze(&self) -> (QueueCapture<T>, usize) {
        let len = self.items.borrow().len().saturating_sub(self.offset);

        (
            QueueCapture {
                items: Rc::clone(&self.items),
                offset: self.offset,
            },
            len,
        )
    }

    fn slot(&self, frozen: &QueueCapture<T>, ordinal: usize) -> Option<T> {
        frozen.items.borrow().get(frozen.offset + ordinal).cloned()
    }

    /// `i >= items.length`, re-read every step against the captured array.
    /// The frozen length from [`freeze`](Sequence::freeze) is deliberately
    /// ignored; it survives only as [`remaining`](crate::cursor::Cursor)'s hint.
    fn limit(&self, frozen: &QueueCapture<T>, _frozen_len: usize) -> usize {
        frozen.items.borrow().len().saturating_sub(frozen.offset)
    }
}

impl<T: fmt::Debug> fmt::Debug for Queue<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Queue")
            .field("size", &self.size)
            .field("offset", &self.offset)
            .field("items", &self.items.borrow())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{CursorState, Step};

    /// The upstream suite, ported 1:1. `test/queue.js`, 11 `it` blocks.
    #[test]
    fn reproduces_the_upstream_suite() {
        let mut queue: Queue<i32> = Queue::new();
        queue.enqueue(1);
        assert_eq!(queue.size(), 1);

        queue.enqueue(2);
        queue.clear();
        assert_eq!(queue.size(), 0);
        assert_eq!(queue.to_vec(), Vec::<i32>::new());

        assert_eq!(queue.peek(), None);
        queue.enqueue(1);
        assert_eq!(queue.peek(), Some(1));
        queue.enqueue(2);
        assert_eq!(queue.peek(), Some(1));

        let mut queue: Queue<i32> = [1, 2, 3].into_iter().collect();
        assert_eq!(queue.dequeue(), Some(1));
        assert_eq!(queue.dequeue(), Some(2));
        assert_eq!(queue.dequeue(), Some(3));
        assert_eq!(queue.dequeue(), None);

        let queue: Queue<i32> = [1, 2, 3].into_iter().collect();
        assert_eq!(queue.to_vec(), vec![1, 2, 3]);
        assert_eq!(queue.values().collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    /// `enqueue` returns the new size. Never asserted upstream.
    #[test]
    fn enqueue_returns_the_new_size() {
        let mut queue = Queue::new();

        assert_eq!(queue.enqueue('a'), 1);
        assert_eq!(queue.enqueue('b'), 2);
    }

    /// The compaction schedule, pinned index by index. The suite dequeues a
    /// three-element queue and never looks at `offset` or `items.length`, so
    /// none of this is observed upstream even though it is the whole point of
    /// the data structure.
    #[test]
    fn the_compaction_fires_when_the_dead_prefix_reaches_half_the_array() {
        let mut queue: Queue<i32> = (1..=4).collect();
        assert_eq!((queue.offset(), queue.items_len()), (0, 4));

        // offset 1, 1 * 2 = 2 < 4: no compaction.
        assert_eq!(queue.dequeue(), Some(1));
        assert_eq!((queue.offset(), queue.items_len()), (1, 4));

        // offset 2, 2 * 2 = 4 >= 4: the array is rebuilt from index 2.
        assert_eq!(queue.dequeue(), Some(2));
        assert_eq!((queue.offset(), queue.items_len()), (0, 2));

        // offset 1, 1 * 2 = 2 >= 2: rebuilt again.
        assert_eq!(queue.dequeue(), Some(3));
        assert_eq!((queue.offset(), queue.items_len()), (0, 1));

        assert_eq!(queue.dequeue(), Some(4));
        assert_eq!((queue.offset(), queue.items_len()), (0, 0));
        assert_eq!(queue.size(), 0);
    }

    /// A single-element queue compacts on its very first dequeue, because
    /// `1 * 2 >= 1`. The degenerate end of the schedule above.
    #[test]
    fn a_one_element_queue_compacts_immediately() {
        let mut queue: Queue<i32> = std::iter::once(42).collect();

        assert_eq!(queue.dequeue(), Some(42));
        assert_eq!((queue.offset(), queue.items_len(), queue.size()), (0, 0, 0));
    }

    /// Dequeue then enqueue then dequeue: the queue must stay FIFO across a
    /// compaction, which is the one way the offset arithmetic can go wrong.
    #[test]
    fn interleaved_enqueue_and_dequeue_stay_in_order() {
        let mut queue: Queue<i32> = Queue::new();
        let mut seen = Vec::new();

        for item in 1..=6 {
            queue.enqueue(item);

            if item % 2 == 0 {
                seen.push(queue.dequeue());
            }
        }

        while let Some(item) = queue.dequeue() {
            seen.push(Some(item));
        }

        assert_eq!(
            seen,
            (1..=6).map(Some).collect::<Vec<_>>(),
            "FIFO order survived the compactions"
        );
    }

    /// Dequeueing an empty queue leaves every field alone — upstream's
    /// `if (!this.size) return;` runs before the offset is touched.
    #[test]
    fn dequeueing_an_empty_queue_moves_nothing() {
        let mut queue: Queue<i32> = Queue::new();

        assert_eq!(queue.dequeue(), None);
        assert_eq!((queue.offset(), queue.items_len(), queue.size()), (0, 0, 0));
    }

    /// D-06/D-07 again: the cursor is stateful, the collection is a factory.
    #[test]
    fn cursors_do_not_restart_but_the_queue_can_be_walked_again() {
        let queue: Queue<i32> = [1, 2, 3].into_iter().collect();

        let mut cursor = queue.values();
        assert_eq!(cursor.next(), Some(1));
        assert_eq!(cursor.by_ref().collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(cursor.collect::<Vec<_>>(), Vec::<i32>::new());

        assert_eq!(queue.values().collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    /// The live end, on the module it was written for: an enqueue during
    /// iteration **is** seen, where the same code in `Stack` would not see it.
    #[test]
    fn an_enqueue_during_iteration_is_visible_to_the_cursor() {
        let mut queue: Queue<i32> = [1, 2].into_iter().collect();
        let mut state = CursorState::open(&queue);

        assert_eq!(state.step(&queue), Step::Item(1));
        queue.enqueue(3);

        assert_eq!(state.step(&queue), Step::Item(2));
        assert_eq!(state.step(&queue), Step::Item(3));
        assert_eq!(state.step(&queue), Step::Done);
    }

    /// And nothing latches: a walk that has already reported `{done: true}`
    /// resumes when the queue grows, because obliterator's `Iterator` re-runs
    /// its closure on every `next()` and has no flag to consult.
    #[test]
    fn a_finished_cursor_resumes_when_the_queue_grows() {
        let mut queue: Queue<i32> = std::iter::once(1).collect();
        let mut state = CursorState::open(&queue);

        assert_eq!(state.step(&queue), Step::Item(1));
        assert_eq!(state.step(&queue), Step::Done);

        queue.enqueue(2);

        assert_eq!(state.step(&queue), Step::Item(2));
        assert_eq!(state.step(&queue), Step::Done);
    }

    /// The compaction detaches an open cursor onto the array it captured — so
    /// it goes on yielding elements the queue has already handed out. Nothing
    /// upstream observes this, and a `Vec<T>` port could not express it.
    #[test]
    fn a_compaction_detaches_an_open_cursor_onto_the_old_array() {
        let mut queue: Queue<i32> = (1..=4).collect();
        let mut state = CursorState::open(&queue);

        // Two dequeues: the second compacts, rebinding `items` to [3, 4].
        assert_eq!(queue.dequeue(), Some(1));
        assert_eq!(queue.dequeue(), Some(2));
        assert_eq!(queue.to_vec(), vec![3, 4]);

        // The cursor froze offset 0 against the original [1, 2, 3, 4] and is
        // still walking it — including the two elements already dequeued.
        assert_eq!(state.step(&queue), Step::Item(1));
        assert_eq!(state.step(&queue), Step::Item(2));
        assert_eq!(state.step(&queue), Step::Item(3));
        assert_eq!(state.step(&queue), Step::Item(4));
        assert_eq!(state.step(&queue), Step::Done);
    }

    /// A cursor opened *after* some dequeues freezes the offset it found, and
    /// a later compaction cannot move it, because the array it froze against
    /// is not the one being rebound.
    #[test]
    fn a_cursor_freezes_the_offset_it_was_opened_with() {
        let mut queue: Queue<i32> = (1..=6).collect();
        assert_eq!(queue.dequeue(), Some(1));
        assert_eq!(queue.offset(), 1);

        let mut state = CursorState::open(&queue);
        assert_eq!(state.step(&queue), Step::Item(2));

        // Two more dequeues reach offset 3, and `3 * 2 >= 6` compacts:
        // items becomes [4, 5, 6] and the offset resets.
        assert_eq!(queue.dequeue(), Some(2));
        assert_eq!(queue.dequeue(), Some(3));
        assert_eq!((queue.offset(), queue.to_vec()), (0, vec![4, 5, 6]));

        // Still on the original array from index 1.
        assert_eq!(state.by_ref_steps(&queue), vec![3, 4, 5, 6]);
    }

    /// `clear` rebinds too, so a cursor opened first is untouched by it.
    #[test]
    fn clear_leaves_an_open_cursor_walking_the_old_array() {
        let mut queue: Queue<i32> = [1, 2, 3].into_iter().collect();
        let mut state = CursorState::open(&queue);

        queue.clear();

        assert_eq!(queue.size(), 0);
        assert_eq!(state.step(&queue), Step::Item(1));
        assert_eq!(state.step(&queue), Step::Item(2));
        assert_eq!(state.step(&queue), Step::Item(3));
        assert_eq!(state.step(&queue), Step::Done);
    }

    /// `forEach`'s read is live where the cursor's is captured: after a
    /// compaction the same absolute index means a different element.
    #[test]
    fn for_each_reads_the_live_array_where_the_cursor_reads_the_capture() {
        let mut queue: Queue<i32> = (1..=4).collect();
        let mut state = CursorState::open(&queue);

        assert_eq!(queue.slot(0), Some(1));

        queue.dequeue();
        queue.dequeue();

        // The live array is now [3, 4]: index 0 means 3.
        assert_eq!(queue.slot(0), Some(3));
        // The capture is still [1, 2, 3, 4]: ordinal 0 means 1.
        assert_eq!(state.step(&queue), Step::Item(1));
    }

    /// An empty queue: zero-length walk, empty array, `undefined` peek.
    #[test]
    fn an_empty_queue_iterates_zero_times() {
        let queue: Queue<i32> = Queue::new();

        assert_eq!(queue.to_vec(), Vec::<i32>::new());
        assert_eq!(queue.values().collect::<Vec<_>>(), Vec::<i32>::new());
        assert_eq!(queue.peek(), None);
    }

    #[test]
    fn from_iter_accepts_any_iterator() {
        let empty: Queue<i32> = std::iter::empty().collect();
        assert_eq!(empty.size(), 0);

        let mapped: Queue<i32> = (1..=3).map(|n| n * 10).collect();
        assert_eq!(mapped.to_vec(), vec![10, 20, 30]);
    }

    /// Test-only convenience: drain a detached cursor into the items it yields.
    trait DrainSteps<T: Clone> {
        fn by_ref_steps(&mut self, queue: &Queue<T>) -> Vec<T>;
    }

    impl<T: Clone> DrainSteps<T> for CursorState<Queue<T>> {
        fn by_ref_steps(&mut self, queue: &Queue<T>) -> Vec<T> {
            let mut items = Vec::new();

            loop {
                match self.step(queue) {
                    Step::Item(item) => items.push(item),
                    Step::Gap => continue,
                    Step::Done => return items,
                }
            }
        }
    }
}

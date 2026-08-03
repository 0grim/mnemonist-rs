//! Port of upstream `fixed-reverse-heap.js` (209 LOC).
//!
//! A bounded "keep the best `k`" heap. It is *reverse* because it stores the
//! elements under `reverseComparator(comparator)`, which puts the **worst**
//! surviving item at the root — so evicting it when a better one arrives is one
//! `replace`, not a scan.
//!
//! # It shares `heap`'s algorithms, and its own `siftUp`
//!
//! `fixed-reverse-heap.js` calls `Heap.siftDown` and `Heap.replace` directly,
//! but re-declares `siftUp` locally. The reason is structural rather than
//! stylistic: its backing array is `capacity` long from the moment it is
//! constructed, while the heap inside it is only `size` long, so
//! `heap.js`'s `var endIndex = heap.length` would walk into the unwritten tail.
//! The local copy takes `size` as a parameter and is otherwise identical — see
//! [`sift_up_within`](crate::structures::heap::sift_up_within), which is that
//! one function.
//!
//! # Two upstream defects live here
//!
//! * the capacity guard is `typeof capacity !== 'number' && capacity <= 0`,
//!   where `||` was meant, so it can never fire for *any* number — NOTES BUG-FIXED-REVERSE-HEAP-1.
//!   The guard is a JavaScript type test, so it is reproduced in the bridge,
//!   not here;
//! * `clear()` resets `size` and nothing else, so `peek()` on a cleared heap
//!   still answers the stale root — NOTES BUG-FIXED-REVERSE-HEAP-2. That one is reproduced here,
//!   because it is about the data and not about JavaScript.

use std::cell::Cell;

use crate::structures::heap::{sift_down, sift_up_within, Store};
use crate::utils::comparators::{Comparator, Reversed};

/// A heap of bounded capacity that keeps the `capacity` best items seen.
pub struct FixedReverseHeap<S: Store, C> {
    /// `this.items`. Never rebound upstream — `clear()` does not touch it —
    /// so unlike [`Heap`](crate::structures::heap::Heap) this is not in a
    /// `RefCell`. The store's own methods take `&self`, which is what a
    /// re-entrant comparator needs.
    items: S,
    capacity: usize,
    size: Cell<usize>,
    /// `this.comparator = reverseComparator(this.comparator)`, applied in the
    /// constructor, so **every** use below is the reversed one.
    comparator: Reversed<C>,
}

impl<S: Store, C: Comparator<S::Item, S::Error>> FixedReverseHeap<S, C> {
    /// `new FixedReverseHeap(ArrayClass, comparator, capacity)`.
    ///
    /// `items` is the caller's `new ArrayClass(capacity)`; the class is a
    /// JavaScript value with no Rust counterpart, so the store carries it.
    pub fn new(items: S, comparator: C, capacity: usize) -> Self {
        Self {
            items,
            capacity,
            size: Cell::new(0),
            comparator: Reversed(comparator),
        }
    }

    pub fn size(&self) -> usize {
        self.size.get()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// `this.items` — the live backing array, `capacity` slots long.
    pub fn items(&self) -> S {
        self.items.clone()
    }

    /// `#.clear` — `this.size = 0`, and **nothing else**.
    ///
    /// The array keeps its contents, which is why [`peek`](Self::peek) can
    /// answer with an item that was logically discarded. NOTES BUG-FIXED-REVERSE-HEAP-2.
    pub fn clear(&self) {
        self.size.set(0);
    }

    /// `#.push` — returns the new size.
    pub fn push(&self, item: S::Item) -> Result<usize, S::Error> {
        // Every `this.size` below is a fresh read, as upstream's are: a
        // comparator that re-enters and pushes changes what the next one sees.
        if self.size.get() < self.capacity {
            self.items.set(self.size.get(), item)?;
            sift_down(&self.comparator, &self.items, 0, self.size.get())?;
            self.size.set(self.size.get() + 1);
        } else if self.comparator.compare(&item, &self.items.get(0)?)? > 0.0 {
            // The heap is full: the root is the worst survivor, and this item
            // beats it. `Heap.replace` sifts over `items.length`, which for a
            // full fixed heap is exactly the heap.
            crate::structures::heap::replace(&self.comparator, &self.items, item)?;
        }

        Ok(self.size.get())
    }

    /// `#.peek` — `this.items[0]`, the **worst** item kept, not the best.
    pub fn peek(&self) -> Result<S::Item, S::Error> {
        self.items.get(0)
    }

    /// `#.consume` — drains into a sorted array and resets `size`.
    pub fn consume(&self) -> Result<S, S::Error> {
        let items = consume(&self.comparator, &self.items, self.size.get())?;

        self.size.set(0);

        Ok(items)
    }

    /// `#.toArray` — `consume` over `items.slice(0, size)`, so the heap
    /// survives and the untouched tail of the backing array is excluded.
    pub fn to_array(&self) -> Result<S, S::Error> {
        let slice = self.items.slice(0, self.size.get())?;

        consume(&self.comparator, &slice, self.size.get())
    }
}

/// `consume(ArrayClass, compare, heap, size)`.
///
/// Fills the result **backwards**: the reverse heap's root is the largest of
/// the survivors, so repeatedly evicting it writes a sorted array from the end
/// inwards. `size` shrinks as it goes, which is why this needs the `size`-aware
/// sift rather than `Heap.siftUp`.
pub fn consume<S, C>(compare: &C, heap: &S, size: usize) -> Result<S, S::Error>
where
    S: Store,
    C: Comparator<S::Item, S::Error>,
{
    let l = size;
    let mut i = l;
    let mut size = size;

    let array = heap.allocate(size)?;

    while i > 0 {
        i -= 1;

        let mut last_item = heap.get(i)?;

        if i != 0 {
            let item = heap.get(0)?;

            heap.set(0, last_item)?;
            size -= 1;
            sift_up_within(compare, heap, size, 0)?;
            last_item = item;
        }

        array.set(i, last_item)?;
    }

    Ok(array)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structures::heap::VecStore;
    use crate::utils::comparators::{Comparator, DefaultComparator, Thrown};

    type Slot = Option<i64>;

    fn heap(capacity: usize) -> FixedReverseHeap<VecStore<i64>, DefaultComparator> {
        // `new ArrayClass(capacity)` — capacity holes.
        let items = VecStore::<i64>::new();

        for _ in 0..capacity {
            items.push(None).unwrap();
        }

        FixedReverseHeap::new(items, DefaultComparator, capacity)
    }

    fn values(store: &VecStore<i64>) -> Vec<Slot> {
        store.to_vec()
    }

    #[test]
    fn keeps_only_the_smallest_items() {
        let heap = heap(3);

        for value in [45, 12, 46, 1, 90, 3, 234, 138, 0] {
            heap.push(Some(value)).unwrap();
        }

        assert_eq!(heap.size(), 3);
        assert_eq!(
            values(&heap.consume().unwrap()),
            vec![Some(0), Some(1), Some(3)]
        );
        assert_eq!(heap.size(), 0);
    }

    #[test]
    fn to_array_leaves_the_heap_intact() {
        let heap = heap(3);

        for value in [4, 1, 8] {
            heap.push(Some(value)).unwrap();
        }

        assert_eq!(
            values(&heap.to_array().unwrap()),
            vec![Some(1), Some(4), Some(8)]
        );
        assert_eq!(heap.size(), 3);
    }

    #[test]
    fn consume_below_capacity_returns_only_the_live_prefix() {
        let heap = heap(3);

        heap.push(Some(3)).unwrap();
        heap.push(Some(34)).unwrap();

        assert_eq!(heap.size(), 2);
        assert_eq!(values(&heap.consume().unwrap()), vec![Some(3), Some(34)]);
    }

    /// NOTES BUG-FIXED-REVERSE-HEAP-2: `clear()` resets `size` and leaves the array alone, so the
    /// root of the discarded heap is still what `peek()` reports.
    #[test]
    fn peek_after_clear_answers_a_discarded_item() {
        let heap = heap(3);

        for value in [45, 12, 46] {
            heap.push(Some(value)).unwrap();
        }

        let stale = heap.peek().unwrap();

        heap.clear();

        assert_eq!(heap.size(), 0);
        assert_eq!(heap.peek().unwrap(), stale);
        assert_ne!(heap.peek().unwrap(), None);
    }

    /// …and the stale contents are invisible to `consume`, which slices to
    /// `size`. The two together are why the bug is latent rather than active.
    #[test]
    fn consume_after_clear_ignores_the_stale_contents() {
        let heap = heap(3);

        for value in [45, 12, 46, 1, 90, 3] {
            heap.push(Some(value)).unwrap();
        }

        heap.clear();
        heap.push(Some(234)).unwrap();
        heap.push(Some(0)).unwrap();

        assert_eq!(heap.size(), 2);
        assert_eq!(values(&heap.consume().unwrap()), vec![Some(0), Some(234)]);
    }

    /// A capacity-0 heap accepts nothing and never throws — the constructor
    /// guard that should have refused it cannot fire. NOTES BUG-FIXED-REVERSE-HEAP-1.
    #[test]
    fn a_capacity_of_zero_silently_accepts_nothing() {
        let heap = heap(0);

        assert_eq!(heap.push(Some(1)).unwrap(), 0);
        assert_eq!(heap.push(Some(2)).unwrap(), 0);
        assert_eq!(heap.size(), 0);
        assert_eq!(values(&heap.consume().unwrap()), Vec::<Slot>::new());
    }

    #[test]
    fn a_reverse_comparator_keeps_the_largest_items() {
        struct Descending;

        impl Comparator<Slot, Thrown> for Descending {
            fn compare(&self, a: &Slot, b: &Slot) -> Result<f64, Thrown> {
                crate::utils::comparators::default_reverse_comparator(a, b)
            }
        }

        let items = VecStore::<i64>::new();

        for _ in 0..3 {
            items.push(None).unwrap();
        }

        let heap = FixedReverseHeap::new(items, Descending, 3);

        for value in [45, 12, 46, 1, 90, 3, 234, 138, 0] {
            heap.push(Some(value)).unwrap();
        }

        assert_eq!(
            values(&heap.consume().unwrap()),
            vec![Some(234), Some(138), Some(90)]
        );
    }

    /// A comparator that pushes into the heap it is comparing runs to
    /// completion rather than deadlocking on an exclusive borrow.
    #[test]
    fn a_comparator_may_re_enter_and_push() {
        use std::cell::RefCell;

        struct Pushy {
            target: RefCell<Option<VecStore<i64>>>,
            budget: Cell<u32>,
        }

        impl Comparator<Slot, Thrown> for Pushy {
            fn compare(&self, a: &Slot, b: &Slot) -> Result<f64, Thrown> {
                if self.budget.get() > 0 {
                    self.budget.set(self.budget.get() - 1);

                    if let Some(items) = self.target.borrow().as_ref() {
                        items.push(Some(-1))?;
                    }
                }

                crate::utils::comparators::default_comparator(a, b)
            }
        }

        let items = VecStore::<i64>::new();

        for _ in 0..3 {
            items.push(None).unwrap();
        }

        let comparator = Pushy {
            target: RefCell::new(Some(items.clone())),
            budget: Cell::new(3),
        };
        let heap = FixedReverseHeap::new(items.clone(), comparator, 3);

        for value in [5, 4, 3, 2, 1] {
            heap.push(Some(value)).unwrap();
        }

        // The array grew past `capacity` because the comparator appended to
        // it; upstream's would too, and nothing in either notices.
        assert!(items.length().unwrap() > 3);
        assert_eq!(heap.size(), 3);
    }
}

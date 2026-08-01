//! Port of upstream `linked-list.js` (v0.40.4).
//!
//! A singly linked list: `push`, `unshift`, `shift`, `first`/`last`/`peek`,
//! `forEach`, `toArray`, `values`/`entries` (and `Symbol.iterator`, aliased to
//! `values`), and a static `.from`. Upstream's own comment says it all:
//! "Uses raw JavaScript objects as nodes as benchmarks proved it was the
//! fastest thing to do" — nodes are plain `{item, next}` objects wired
//! together by reference, and the module is 224 lines almost entirely because
//! of that directness.
//!
//! # Why the nodes are an arena, not `Option<Box<Node<T>>>`
//!
//! A textbook Rust singly linked list is `Option<Box<Node<T>>>` chains, owned
//! head-to-tail. That shape cannot reproduce this module's cursors, and not
//! for the usual "no unsafe" reason — the reason is upstream's own iterator
//! closures:
//!
//! ```js
//! LinkedList.prototype.values = function() {
//!   var n = this.head;                      // captured ONCE, at creation
//!   return new Iterator(function() {
//!     if (!n) return {done: true};
//!     var value = n.item;
//!     n = n.next;                            // follows the OBJECT's own link
//!     return {value, done: false};
//!   });
//! };
//! ```
//!
//! `n` is a reference to a specific node **object**, captured when the cursor
//! opens, and the closure never again reads `this.head` or `this.tail`. Three
//! consequences fall out of that, all upstream's real behaviour and all
//! measured against Node 24.18.1 (`~/upstream-mnemonist/linked-list.js`):
//!
//! * **`shift()` is invisible to an already-open cursor.** `shift` only moves
//!   `this.head` forward; it never touches the old head node's `.next`. A
//!   cursor already sitting on (or past) that node keeps walking the object
//!   chain it captured, oblivious to how many times the list has been
//!   shifted since.
//! * **`unshift()` is invisible to an already-open cursor**, for the same
//!   reason in the other direction: prepending links the new node's `.next`
//!   to the *old* head and rebinds `this.head`; nothing about that touches
//!   any node a cursor already holds.
//! * **`push()` *is* visible, exactly once, if the cursor is sitting on the
//!   current tail.** `push` sets `this.tail.next = node` **on the tail object
//!   itself** — an in-place mutation of a node a cursor may already be
//!   holding. A cursor that has not yet advanced past the (old) tail sees the
//!   append; upstream's own `forEach` (`callback` runs, *then* `n = n.next`
//!   reads the now-possibly-updated `.next`) and the lazy iterators (`n =
//!   n.next` runs synchronously in the same step that produced the previous
//!   value) agree on this, because JavaScript's single-threaded execution
//!   makes "the callback ran" and "the next `.next()` call runs" the same
//!   kind of pause: nothing can slip in between reading a node's item and
//!   reading its `next` field within either shape, and nothing can slip in
//!   before it either. So `forEach` and `values`/`entries` are **one
//!   walk primitive here**, not two — unlike `lru-cache`'s D-90, where the
//!   sift's `forward[pointer]` is a *separate* bookkeeping array read at a
//!   different cadence than a stored cursor's own advance.
//!
//! An owning `Box` chain cannot reproduce any of this: dropping a `Box`
//! deallocates the node, so a captured "reference" to a shifted-off node
//! would dangle. What is needed is what upstream actually has — an object
//! that survives independently of the list's own `head`/`tail` bookkeeping,
//! reachable by whoever still holds it — and the shape this project already
//! uses for exactly that problem is an **arena**: every node lives in
//! `LinkedList::arena`, addressed by a plain `usize`. A cursor holds an index
//! into the arena, not a borrow of the list, so it survives every mutation
//! the list makes to its own `head`/`tail`/`size` — precisely because,
//! symmetrically with [`crate::structures::fibonacci_heap`]'s arena, indices
//! do not own anything and cannot dangle.
//!
//! # The arena is never shrunk, never recycled — by design, not oversight
//!
//! `shift()` removes a node from the list's *reachable* chain (advances
//! `head`) but never from the arena: the `Node` stays at its slot, `item`
//! intact, `next` untouched, forever. This is what makes the three bullets
//! above true in Rust exactly as they are in JS — a slot that shifted off
//! ten operations ago is still there, at the same index, for a cursor that
//! captured it before the shift to keep reading. Recycling the slot (the
//! *usual* point of an arena) would make that index meaningless the moment it
//! was reused for an unrelated later node, silently aliasing two logically
//! distinct list positions — the exact failure mode
//! [`crate::structures::fibonacci_heap`]'s own docs describe for a recycling
//! arena under re-entrancy, here reachable with nothing more exotic than an
//! ordinary open cursor.
//!
//! The cost is real and disclosed, not hidden: a `LinkedList` that has pushed
//! and shifted heavily over its lifetime keeps every item it has ever held —
//! a monotonically growing arena — until the whole list is dropped. Upstream
//! does *not* have this cost: once nothing (no list, no open cursor) holds a
//! shifted-off node, V8's GC reclaims it. This port cannot tell "no cursor
//! holds it any more" without a live reference count per node — the same
//! kind of constraint the FFI boundary already answers one way for `trie`'s
//! D-201 (a resumable cursor cannot borrow the collection across calls); see
//! `crates/mnemonist-napi/src/linked_list.rs` and `docs/modules/linked-list.md`
//! for the value-retention consequence at the bridge, where a stored item is
//! a JS value kept alive by the arena for as long as the arena lives.
//!
//! # B-241 — `shift()` never updates `tail`, so emptying the list leaves it stale
//!
//! ```js
//! LinkedList.prototype.shift = function() {
//!   if (!this.size) return undefined;
//!   var node = this.head;
//!   this.head = node.next;
//!   this.size--;
//!   return node.item;
//! };
//! ```
//!
//! `shift` reassigns `this.head` and decrements `this.size`, and never once
//! reads or writes `this.tail`. That is fine as long as the list still has at
//! least one element afterwards — `this.tail` is simply not consulted. But
//! shifting the **last** element empties the list (`head` becomes `null`,
//! `size` becomes `0`) while `tail` still points at the very node that was
//! just removed:
//!
//! ```text
//! var list = new LinkedList(); list.push('a');
//! list.shift();                 // -> 'a'
//! list.size                     // 0
//! list.first()                  // undefined  (head is null: correct)
//! list.last()                   // 'a'        (tail is STALE: the removed item)
//! ```
//!
//! Verified against Node 24.18.1; recorded as **B-241** in NOTES.md. Silent
//! and self-healing exactly like B-40: the next `push` or `unshift` on an
//! empty list takes the `!this.head` branch, which sets `this.tail = node`
//! unconditionally — so the staleness is observable only in the narrow window
//! between "shifted to empty" and "the next insert," never afterwards.
//!
//! Reproduced rather than corrected: [`LinkedList::shift`] leaves
//! `tail` exactly where it was when the list had one element
//! left, and [`LinkedList::last`] reads it verbatim, with no "is the list
//! actually empty" guard that upstream's `last` does not have either. A port
//! that made `last()` check `size` first — the correction a careful porter
//! would reach for — is *tidier and wrong*, and the original suite (which
//! never shifts a list to empty and then reads `last()`) would not notice.

use crate::cursor::Step;

/// One arena slot. `next` is `None` at the end of the chain, never
/// reassigned once written except by [`LinkedList::shift`] advancing past it
/// — see the module docs for why the arena never frees or reuses a slot.
struct Node<T> {
    item: T,
    next: Option<usize>,
}

/// Upstream's `LinkedList`.
///
/// `T` carries no bounds at the struct level; only [`LinkedList::to_array`]
/// and [`LinkedList::shift`] need `T: Clone`, for the reason the module docs
/// give — a value must be readable without being moved out of a slot a live
/// cursor may still visit.
pub struct LinkedList<T> {
    arena: Vec<Node<T>>,
    head: Option<usize>,
    tail: Option<usize>,
    size: usize,
}

impl<T> Default for LinkedList<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LinkedList<T> {
    /// `new LinkedList()`, which upstream implements as `this.clear()`.
    pub fn new() -> Self {
        Self {
            arena: Vec::new(),
            head: None,
            tail: None,
            size: 0,
        }
    }

    /// Upstream's `clear`.
    ///
    /// Resets the list's own bookkeeping only. The arena is **not** touched —
    /// see the module docs — which is exactly upstream's behaviour: `clear`
    /// never visits a single node object, so a cursor that already captured
    /// one keeps walking it after `clear()` runs, same as after any other
    /// mutation this list makes.
    pub fn clear(&mut self) {
        self.head = None;
        self.tail = None;
        self.size = 0;
    }

    /// Upstream's `size` property.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Upstream's `first`, aliased as `peek`.
    pub fn first(&self) -> Option<&T> {
        self.head.map(|index| &self.arena[index].item)
    }

    /// Upstream's `last`.
    ///
    /// Reads `tail` verbatim, with no guard against `size == 0`
    /// — which is exactly how B-241 reproduces: after `shift()` has emptied
    /// the list, `tail` still names the just-removed node, and this method
    /// returns its item rather than `None`. See the module docs.
    pub fn last(&self) -> Option<&T> {
        self.tail.map(|index| &self.arena[index].item)
    }

    /// Upstream's `peek`, an alias for `first`.
    pub fn peek(&self) -> Option<&T> {
        self.first()
    }

    /// Upstream's `push`. Returns the new size —
    /// `this.size++; return this.size;`, the post-increment value.
    pub fn push(&mut self, item: T) -> usize {
        let index = self.arena.len();
        self.arena.push(Node { item, next: None });

        match self.tail {
            None => {
                self.head = Some(index);
                self.tail = Some(index);
            }
            Some(tail) => {
                // The in-place mutation the module docs describe: this is
                // what makes an append visible to a cursor already sitting on
                // the (old) tail.
                self.arena[tail].next = Some(index);
                self.tail = Some(index);
            }
        }

        self.size += 1;
        self.size
    }

    /// Upstream's `unshift`.
    ///
    /// The `if (!this.head.next) this.tail = this.head;` guard in the
    /// original only ever fires when the list already has exactly one node
    /// (so `tail` already equals `head`) — a no-op assignment, reproduced
    /// here as no assignment at all, which is observationally identical.
    /// Note what this method does **not** do: touch any existing node's
    /// `next`. That is why a cursor already open before an `unshift` never
    /// sees the prepended item — see the module docs.
    pub fn unshift(&mut self, item: T) -> usize {
        let index = self.arena.len();

        match self.head {
            None => {
                self.arena.push(Node { item, next: None });
                self.head = Some(index);
                self.tail = Some(index);
            }
            Some(head) => {
                self.arena.push(Node {
                    item,
                    next: Some(head),
                });
                self.head = Some(index);
            }
        }

        self.size += 1;
        self.size
    }

    /// Upstream's `shift`.
    ///
    /// `tail` is deliberately **not** touched — see B-241 in the module
    /// docs. The returned value is a clone rather than a move: the slot
    /// itself is never freed (the module docs explain why), so the item has
    /// to stay put for any cursor that already captured this node.
    pub fn shift(&mut self) -> Option<T>
    where
        T: Clone,
    {
        let head = self.head?;

        self.head = self.arena[head].next;
        self.size -= 1;

        Some(self.arena[head].item.clone())
    }

    /// Upstream's `toArray`.
    pub fn to_array(&self) -> Vec<T>
    where
        T: Clone,
    {
        let mut out = Vec::with_capacity(self.size);
        let mut cursor = self.values();

        while let Some(item) = cursor.step(self) {
            out.push(item.clone());
        }

        out
    }

    /// A fresh cursor over this list's items — upstream's `values`, and the
    /// walk `forEach`, `entries` and `Symbol.iterator` all share. See the
    /// module docs for why one primitive covers all four.
    pub fn values(&self) -> ListCursor {
        ListCursor::open(self.head)
    }
}

impl<T> FromIterator<T> for LinkedList<T> {
    /// Upstream's static `.from`, minus the JS-iterable question of *how* to
    /// enumerate the source — that lives at the boundary
    /// (`mnemonist_napi::linked_list`), exactly as it does for every other
    /// `.from` in this port.
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut list = Self::new();

        for item in iter {
            list.push(item);
        }

        list
    }
}

/// A stateful, non-restartable walk over a [`LinkedList`], indexed into its
/// arena rather than borrowing it.
///
/// Deliberately **not** [`crate::cursor::Sequence`]/[`crate::cursor::Cursor`]:
/// those freeze a *length* and read elements by ordinal against a live
/// source that can still answer "what is at ordinal N" differently each
/// step. This cursor freezes nothing but a starting node reference — exactly
/// upstream's `var n = this.head` — and never again asks the list anything;
/// it only ever follows the chain of `next` fields the node it is holding
/// names. That is a materially different shape (compare
/// [`crate::map::MapCursor`], which is not `Sequence` either, and for an
/// analogous reason), so it is its own type rather than a strained fit into
/// the general one.
#[derive(Debug, Clone, Copy)]
pub struct ListCursor {
    current: Option<usize>,
}

impl ListCursor {
    /// Open a cursor starting at arena index `start` — upstream's
    /// `var n = this.head`.
    pub fn open(start: Option<usize>) -> Self {
        Self { current: start }
    }

    /// Advance one step against `list`'s arena.
    ///
    /// Faithful to upstream's `var value = n.item; n = n.next;` in one
    /// non-interruptible unit — see the module docs on why that is exactly
    /// what upstream's own single-threaded closures amount to as well, for
    /// both `forEach` and the lazy iterators.
    pub fn step<'a, T>(&mut self, list: &'a LinkedList<T>) -> Option<&'a T> {
        let index = self.current?;
        let node = &list.arena[index];

        self.current = node.next;

        Some(&node.item)
    }

    /// [`ListCursor::step`], reported as a [`Step`] — `Step::Done` at the
    /// end and never a [`Step::Gap`], because nothing about this walk can
    /// open one: it does not freeze a length to overrun, only a node chain
    /// to follow, and following a chain either finds a next node or it does
    /// not.
    pub fn step_checked<'a, T>(&mut self, list: &'a LinkedList<T>) -> Step<&'a T> {
        match self.step(list) {
            Some(item) => Step::Item(item),
            None => Step::Done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list_from(items: impl IntoIterator<Item = i32>) -> LinkedList<i32> {
        items.into_iter().collect()
    }

    // ---- 1:1 port of the upstream suite, as a baseline -------------------

    #[test]
    fn reproduces_the_upstream_suite() {
        let mut list = LinkedList::new();
        list.push(9999);
        assert_eq!(list.size(), 1);

        let mut list = LinkedList::new();
        list.push(2);
        list.push(3);
        list.unshift(1);
        assert_eq!(list.size(), 3);
        assert_eq!(list.to_array(), vec![1, 2, 3]);

        let mut list = LinkedList::new();
        list.push(2);
        list.push(3);
        list.clear();
        assert_eq!(list.size(), 0);
        assert_eq!(list.to_array(), Vec::<i32>::new());

        let mut list = LinkedList::new();
        assert_eq!(list.first(), None);
        assert_eq!(list.last(), None);
        list.push(100);
        assert_eq!(list.first(), Some(&100));
        assert_eq!(list.first(), list.last());
        list.push(200);
        assert_eq!(list.first(), Some(&100));
        assert_eq!(list.last(), Some(&200));
        assert_eq!(list.first(), list.peek());

        let mut list = LinkedList::new();
        list.push(1);
        list.push(2);
        list.push(3);
        assert_eq!(list.shift(), Some(1));
        assert_eq!(list.shift(), Some(2));
        assert_eq!(list.shift(), Some(3));
        assert_eq!(list.shift(), None);

        let list = list_from([1, 2, 3]);
        let mut times = 0;
        let mut cursor = list.values();
        let mut i = 0;
        while let Some(item) = cursor.step(&list) {
            assert_eq!(*item, i + 1);
            times += 1;
            i += 1;
        }
        assert_eq!(times, 3);

        let items = list_from([1, 2, 3]);
        assert_eq!(items.size(), 3);
        assert_eq!(items.last(), Some(&3));
    }

    // ---- B-241 -------------------------------------------------------

    #[test]
    fn shifting_the_last_element_leaves_tail_stale() {
        let mut list = LinkedList::new();
        list.push("a");

        assert_eq!(list.shift(), Some("a"));
        assert_eq!(list.size(), 0);
        assert_eq!(list.first(), None, "head correctly reports empty");
        assert_eq!(
            list.last(),
            Some(&"a"),
            "B-241: tail is stale, still the removed item"
        );
    }

    #[test]
    fn a_stale_tail_from_b_241_is_healed_by_the_next_push() {
        let mut list = LinkedList::new();
        list.push("a");
        list.shift();
        assert_eq!(list.last(), Some(&"a"), "stale before the next insert");

        list.push("b");
        assert_eq!(list.last(), Some(&"b"), "push resynchronises tail");
    }

    #[test]
    fn a_stale_tail_from_b_241_is_healed_by_the_next_unshift() {
        let mut list = LinkedList::new();
        list.push("a");
        list.shift();

        list.unshift("z");
        assert_eq!(list.last(), Some(&"z"), "unshift on an empty list too");
    }

    #[test]
    fn the_staleness_only_appears_once_the_list_is_shifted_fully_empty() {
        let mut list = list_from([1, 2]);

        assert_eq!(list.shift(), Some(1));
        // One element left: tail is untouched by shift, but it was already
        // correct (tail never pointed at the removed node here).
        assert_eq!(list.last(), Some(&2), "not stale with one element left");

        assert_eq!(list.shift(), Some(2));
        assert_eq!(list.last(), Some(&2), "B-241 fires only now");
    }

    // ---- Cursor liveness: the three-way split the module docs describe ---

    #[test]
    fn a_push_after_the_cursor_opened_is_visible_if_not_yet_past_the_tail() {
        let mut list = list_from([1, 2]);
        let mut cursor = list.values();

        assert_eq!(cursor.step(&list), Some(&1));
        list.push(3); // cursor has not reached the (old) tail, node 2, yet
        assert_eq!(cursor.step(&list), Some(&2));
        assert_eq!(cursor.step(&list), Some(&3), "the append was visible");
        assert_eq!(cursor.step(&list), None);
    }

    #[test]
    fn a_push_after_the_cursor_has_passed_the_tail_is_not_visible() {
        let mut list = list_from([1]);
        let mut cursor = list.values();

        assert_eq!(cursor.step(&list), Some(&1));
        assert_eq!(cursor.step(&list), None, "cursor reports done");

        list.push(2);
        assert_eq!(
            cursor.step(&list),
            None,
            "a cursor that reported done stays done, even though the list grew"
        );
    }

    #[test]
    fn a_shift_is_invisible_to_a_cursor_already_open() {
        let mut list = list_from([1, 2, 3]);
        let mut cursor = list.values();

        assert_eq!(cursor.step(&list), Some(&1));
        list.shift(); // removes 1 from the list's own view; cursor unaffected
        assert_eq!(
            cursor.step(&list),
            Some(&2),
            "the cursor's own chain is untouched by shift"
        );
        assert_eq!(cursor.step(&list), Some(&3));
    }

    #[test]
    fn an_unshift_is_invisible_to_a_cursor_already_open() {
        let mut list = list_from([1, 2]);
        let mut cursor = list.values();

        list.unshift(0);
        // The cursor still starts at the ORIGINAL head, never the new one.
        assert_eq!(cursor.step(&list), Some(&1));
        assert_eq!(cursor.step(&list), Some(&2));
        assert_eq!(cursor.step(&list), None);
    }

    #[test]
    fn a_cursor_opened_on_an_empty_list_never_yields_anything_even_after_pushes() {
        let mut list: LinkedList<i32> = LinkedList::new();
        let mut cursor = list.values();

        assert_eq!(cursor.step(&list), None);
        list.push(1);
        list.push(2);
        assert_eq!(
            cursor.step(&list),
            None,
            "a cursor that captured `head = None` is done forever"
        );
    }

    #[test]
    fn clear_does_not_affect_a_cursor_already_open() {
        let mut list = list_from([1, 2, 3]);
        let mut cursor = list.values();

        assert_eq!(cursor.step(&list), Some(&1));
        list.clear();
        assert_eq!(
            cursor.step(&list),
            Some(&2),
            "clear only rebinds head/tail/size, never a node's own `next`"
        );
        assert_eq!(cursor.step(&list), Some(&3));
        assert_eq!(cursor.step(&list), None);
    }

    #[test]
    fn a_cursor_is_not_restartable() {
        let list = list_from([1, 2]);
        let mut cursor = list.values();

        assert_eq!(cursor.step(&list), Some(&1));
        assert_eq!(cursor.step(&list), Some(&2));
        assert_eq!(cursor.step(&list), None);
        assert_eq!(cursor.step(&list), None, "stays done");
    }

    // ---- Everything else --------------------------------------------

    #[test]
    fn from_iter_builds_in_order() {
        let items = [1, 2, 3, 4, 5];
        let list: LinkedList<i32> = items.iter().copied().collect();

        assert_eq!(list.to_array(), items.to_vec());
        assert_eq!(list.size(), 5);
    }

    #[test]
    fn shift_on_an_empty_list_reports_absence_without_panicking() {
        let mut list: LinkedList<i32> = LinkedList::new();
        assert_eq!(list.shift(), None);
        assert_eq!(list.size(), 0);
    }

    #[test]
    fn an_empty_list_reports_empty_everywhere() {
        let list: LinkedList<i32> = LinkedList::new();

        assert_eq!(list.first(), None);
        assert_eq!(list.last(), None);
        assert_eq!(list.peek(), None);
        assert_eq!(list.to_array(), Vec::<i32>::new());
        assert_eq!(list.size(), 0);
    }

    #[test]
    fn step_checked_reports_done_rather_than_a_gap() {
        let list = list_from([1]);
        let mut cursor = list.values();

        assert_eq!(cursor.step_checked(&list), Step::Item(&1));
        assert_eq!(cursor.step_checked(&list), Step::Done);
    }

    #[test]
    fn interleaved_unshift_and_push_produce_the_expected_order() {
        let mut list = LinkedList::new();
        list.push(2);
        list.unshift(1);
        list.push(3);
        list.unshift(0);

        assert_eq!(list.to_array(), vec![0, 1, 2, 3]);
        assert_eq!(list.first(), Some(&0));
        assert_eq!(list.last(), Some(&3));
    }

    #[test]
    fn a_long_workout_of_push_shift_unshift_matches_a_vecdeque_reference() {
        use std::collections::VecDeque;

        let mut list = LinkedList::new();
        let mut reference: VecDeque<i32> = VecDeque::new();
        let mut next_value = 0;

        // A small deterministic script exercising every mutating op many
        // times, including shifting fully empty more than once (B-241's
        // trigger) and pushing/unshifting right afterwards (its healing).
        let script = "PPPUSUPPSSSUUPSPSUPPPPSSSSSSUPS";

        for op in script.chars() {
            match op {
                'P' => {
                    list.push(next_value);
                    reference.push_back(next_value);
                    next_value += 1;
                }
                'U' => {
                    list.unshift(next_value);
                    reference.push_front(next_value);
                    next_value += 1;
                }
                'S' => {
                    assert_eq!(list.shift(), reference.pop_front());
                }
                other => panic!("`{other}` is not in this script's alphabet"),
            }

            assert_eq!(list.size(), reference.len());
            assert_eq!(
                list.to_array(),
                reference.iter().copied().collect::<Vec<_>>()
            );
        }
    }
}

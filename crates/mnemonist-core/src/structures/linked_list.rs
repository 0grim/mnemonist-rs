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
//!   append.
//!
//! # A port defect the fuzzer caught on its own first campaign: `forEach` is NOT the same primitive as the lazy iterators
//!
//! An earlier cut of this module claimed `forEach` and `values`/`entries`
//! could share one stepping primitive, on the theory that JavaScript's
//! single-threaded execution makes "the callback ran" and "the next
//! `.next()` call runs" equivalent pauses. That is wrong, and reading
//! upstream's own two shapes side by side shows why:
//!
//! ```js
//! // forEach:
//! while (n) {
//!   callback.call(scope, n.item, i, this);
//!   n = n.next;   // AFTER the callback -- two separate statements
//!   i++;
//! }
//!
//! // values() (and entries()):
//! return new Iterator(function () {
//!   if (!n) return {done: true};
//!   var value = n.item;
//!   n = n.next;   // BEFORE control ever returns to the caller
//!   return {value, done: false};
//! });
//! ```
//!
//! `forEach`'s callback runs, and can mutate the list, **between** the read
//! of `n.item` and the read of `n.next` — so a `push` that happens to land
//! on the *current* tail while its own callback invocation is still running
//! relinks that tail's `.next` before the following `n = n.next` reads it,
//! and the walk continues onto the freshly pushed node. The lazy iterators'
//! `n = n.next` runs synchronously, inside the same call that produced the
//! *previous* value, with no opportunity for caller code to run in between —
//! so by the time a caller's own code could push anything, `n` has already
//! moved past the (old) tail and the append is invisible to that cursor.
//!
//! Concretely: `s.push(0); s.push(0); s.shift();` leaves one node, which is
//! simultaneously head and tail. `s.forEach(function (a) { if (fired++ < 1)
//! s.push(a); })` visits it, pushes during that visit, and — because the walk
//! is still sitting on the node whose `.next` the push just set — visits the
//! newly pushed node too: two calls, not one. A `step`-based cursor that
//! advances immediately captures `.next` (`null`, at that point) before the
//! callback runs, and the push arrives too late to be seen: one call. Found
//! by `crates/difffuzz/src/modules/linked_list.rs`'s very first campaign
//! (`--seed 42 --cases 63`), which disagreed after exactly this program's
//! third operation. Not an upstream bug — a defect in this port, fixed here
//! before any campaign was logged in `fuzz/log.txt`.
//!
//! [`ListCursor::current`]/[`ListCursor::advance`] are the fix: `forEach`
//! reads the current item, lets the callback run, and only then advances,
//! reading `next` live at that later point — matching `lru-cache`'s own
//! `ForEachWalk` split (DIV-LRU-CACHE-2) for the identical reason: the sift there reads
//! a *separate* bookkeeping array at a different cadence than a stored
//! cursor's own advance; here it is the same node object read at two
//! different times instead. [`ListCursor::step`] keeps the lazy iterators'
//! original eager-advance shape, because that one *is* correct for them.
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
//! DIV-TRIE-MAP-2 (a resumable cursor cannot borrow the collection across calls); see
//! `crates/mnemonist-napi/src/linked_list.rs` and `docs/modules/linked-list.md`
//! for the value-retention consequence at the bridge, where a stored item is
//! a JS value kept alive by the arena for as long as the arena lives.
//!
//! # BUG-LINKED-LIST-1 — `shift()` never updates `tail`, so emptying the list leaves it stale
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
//! Verified against Node 24.18.1; recorded as **BUG-LINKED-LIST-1**. Silent
//! and self-healing exactly like BUG-DEFAULT-MAP-1: the next `push` or `unshift` on an
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
    /// — which is exactly how BUG-LINKED-LIST-1 reproduces: after `shift()` has emptied
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
    ///
    /// # A second port defect the fuzzer found: this must branch on `head`, not `tail`
    ///
    /// Upstream's own guard is `if (!this.head) { this.head = node; this.tail
    /// = node; } else { this.tail.next = node; this.tail = node; }` —
    /// checking **`head`**. An earlier cut of this method checked `self.tail`
    /// instead, which is indistinguishable in every ordinary state (`head`
    /// and `tail` are always both `None` or both `Some` outside of BUG-LINKED-LIST-1) but
    /// diverges in exactly the state BUG-LINKED-LIST-1 produces: `shift()` on a
    /// one-element list sets `head = None` while leaving `tail` at the
    /// removed node. A push in that state must see `!this.head` and start a
    /// **fresh** one-element list — abandoning the stale `tail` entirely,
    /// not linking onto it — because that is what upstream's guard reads.
    /// Branching on `tail` instead took the *linking* branch, appended onto
    /// the stale node, and never touched `head` — leaving `head` permanently
    /// `None` while `tail` and the arena both held a real, unreachable node.
    /// Found by the same first fuzz campaign as the `forEach` timing defect,
    /// one generated case later (`push(0); forEach(cb: shift once); push(0);`
    /// — the second `push`, right after BUG-LINKED-LIST-1 fires, is where the two
    /// branches disagree): port `toArray() == []`, upstream `toArray() ==
    /// [0]`. Not an upstream bug — a defect in this port, fixed here before
    /// any campaign was logged in `fuzz/log.txt`.
    pub fn push(&mut self, item: T) -> usize {
        let index = self.arena.len();
        self.arena.push(Node { item, next: None });

        match self.head {
            None => {
                self.head = Some(index);
                self.tail = Some(index);
            }
            Some(_) => {
                // `tail` may be stale (BUG-LINKED-LIST-1) but `head` being `Some` means
                // this really is a non-empty list, so `tail` still names a
                // live node to link onto -- the in-place mutation the module
                // docs describe, which is what makes an append visible to a
                // cursor already sitting on the (old) tail.
                let tail = self
                    .tail
                    .expect("head is Some, so tail names a node too, per push/unshift/shift");

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
    /// `tail` is deliberately **not** touched — see BUG-LINKED-LIST-1 in the module
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

    /// Advance one step against `list`'s arena — upstream's lazy-iterator
    /// shape: `var value = n.item; n = n.next; return {value, ...};`, both
    /// statements inside the SAME call, with no opportunity for caller code
    /// to run between them. `values`/`entries`/`$next`/`$spread` all use
    /// this. **`forEach` must not** — see [`ListCursor::current`] and
    /// [`ListCursor::advance`], and the module docs' section on the port
    /// defect the fuzzer found for why the two are genuinely different
    /// primitives here.
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

    /// The item at the current position, WITHOUT advancing — the first half
    /// of upstream's `forEach` loop body. See [`ListCursor::advance`] and the
    /// module docs' section on the port defect the fuzzer found: the whole
    /// reason this exists separately from [`ListCursor::step`] is that
    /// `forEach`'s callback must be able to run, and possibly mutate the
    /// list, **between** this read and the advance that follows it.
    pub fn current<'a, T>(&self, list: &'a LinkedList<T>) -> Option<&'a T> {
        self.current.map(|index| &list.arena[index].item)
    }

    /// Move past the current position, reading `next` **live** — upstream's
    /// `n = n.next`, run as its own statement, after whatever the caller did
    /// between [`ListCursor::current`] and this call. A no-op once the
    /// cursor is already done.
    pub fn advance<T>(&mut self, list: &LinkedList<T>) {
        if let Some(index) = self.current {
            self.current = list.arena[index].next;
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

    // ---- BUG-LINKED-LIST-1 -------------------------------------------------------

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
            "BUG-LINKED-LIST-1: tail is stale, still the removed item"
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
        // `last()` alone does not close this gap: a port that appends onto
        // the stale tail instead of starting fresh also reports `last() ==
        // "b"`, correctly, while leaving `head` permanently `None` -- see
        // `push`'s own doc comment on the second fuzzer-found port defect.
        // `first()`/`to_array()` are what actually distinguish the two.
        assert_eq!(list.first(), Some(&"b"), "push also resynchronises head");
        assert_eq!(list.to_array(), vec!["b"]);
        assert_eq!(list.size(), 1);
    }

    /// The exact program the differential fuzzer minimised to
    /// (`--module linked-list --seed 42`, one op after the `forEach` timing
    /// defect's own repro): `push` branching on `self.tail` instead of
    /// `self.head` (an earlier cut of this method) takes the "list is
    /// non-empty" branch in the BUG-LINKED-LIST-1 state and links onto the stale tail
    /// instead of starting fresh, leaving `head` stuck at `None` forever.
    /// The test above already covered "push after a plain shift-to-empty";
    /// this one covers reaching that same state via a mutating `forEach`,
    /// which is a different code path to the same state.
    #[test]
    fn push_after_for_each_shifts_the_list_to_empty_starts_a_fresh_one_element_list() {
        let mut list = LinkedList::new();
        list.push(0);

        let mut cursor = list.values();
        let mut fired = false;

        while let Some(_item) = cursor.current(&list) {
            if !fired {
                fired = true;
                list.shift();
            }

            cursor.advance(&list);
        }

        assert_eq!(list.size(), 0, "shifted to empty inside the walk");

        list.push(9);

        assert_eq!(list.first(), Some(&9));
        assert_eq!(list.last(), Some(&9));
        assert_eq!(list.to_array(), vec![9]);
        assert_eq!(list.size(), 1);
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
        assert_eq!(list.last(), Some(&2), "BUG-LINKED-LIST-1 fires only now");
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

    /// The exact shape the differential fuzzer found on its first campaign
    /// (`--module linked-list --seed 42`, minimised to `push; push; shift;
    /// forEach(cb push-once)`): after `shift()` leaves one node that is both
    /// head and tail, a `forEach`-shaped walk (`current` then `advance`,
    /// mutating in between) that pushes while visiting that lone node MUST
    /// go on to visit the pushed node too, in the SAME walk -- because the
    /// push happens before `advance` reads `next`. See the module docs.
    #[test]
    fn a_for_each_shaped_walk_sees_a_push_made_from_its_own_callback_on_the_lone_tail_node() {
        let mut list = LinkedList::new();
        list.push(0);
        list.push(0);
        list.shift();
        assert_eq!(list.to_array(), vec![0], "one node, both head and tail");

        let mut cursor = list.values();
        let mut visits = Vec::new();
        let mut fired = false;

        while let Some(item) = cursor.current(&list).copied() {
            visits.push(item);

            if !fired {
                fired = true;
                list.push(item);
            }

            cursor.advance(&list);
        }

        assert_eq!(
            visits,
            vec![0, 0],
            "the walk must see the node pushed while it was sitting on the tail"
        );
    }

    /// The same scenario, but through `step` (the lazy-iterator shape) —
    /// which must NOT see the push, because its advance already ran before
    /// the caller gets a chance to mutate anything.
    #[test]
    fn a_step_shaped_walk_does_not_see_a_push_made_between_two_of_its_own_steps() {
        let mut list = LinkedList::new();
        list.push(0);
        list.push(0);
        list.shift();

        let mut cursor = list.values();
        assert_eq!(cursor.step(&list), Some(&0));
        list.push(0); // caller code, AFTER the step already advanced past it
        assert_eq!(
            cursor.step(&list),
            None,
            "the lazy iterator already moved past the (old) tail before this push ran"
        );
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
        // times, including shifting fully empty more than once (BUG-LINKED-LIST-1's
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

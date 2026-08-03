//! Port of upstream `fibonacci-heap.js` (321 LOC).
//!
//! A textbook Fibonacci heap: `push` is O(1) amortised, `peek` is O(1),
//! `pop` is O(log n) amortised and does the real work — merging the popped
//! node's children into the root list, then `consolidate`, which links trees
//! of equal degree until every root has a distinct one.
//!
//! # What upstream does NOT implement
//!
//! Read the source before porting it, not just the test file: **there is no
//! `decreaseKey` and no `delete`.** No `mark` field on a node, no cut, no
//! cascading cut — grep `~/upstream-mnemonist/fibonacci-heap.js` and
//! `fibonacci-heap.d.ts` for `decreaseKey`, `cut`, `mark`; none exist. The
//! public surface is `clear`, `push`, `peek`, `pop`, `inspect`, `.from`. This
//! matters for the fuzz campaign below: cascading cuts cannot be fuzzed into
//! existence because there is no operation that could ever trigger one. That
//! is upstream's own limitation, not a gap in this port's grammar — see
//! `docs/modules/fibonacci-heap.md`.
//!
//! # Why the nodes are an arena, not `Rc<RefCell<Node>>`
//!
//! Upstream's node is a plain object wired into a **circular** doubly-linked
//! list (`left`/`right`) plus a parent/child tree, and JavaScript's tracing
//! GC collects any cycle the moment nothing outside it points in. A literal
//! transliteration — `Rc<RefCell<Node<T>>>` for `left`/`right`/`parent`/
//! `child` — does not have that luxury: `Rc` is reference-counted, and a
//! circular list where every node's neighbours hold strong references to it
//! is a cycle that **never reaches zero**, reachable or not, singleton or
//! not. The push/pop pair that empties a heap to one element and pops it
//! reproduces exactly this shape: the last node's `left`/`right` are
//! self-references (see `pop`'s singleton branch below), which is a
//! two-strong-reference cycle of one that leaks forever the instant
//! `heap.root`/`heap.min` let go of it. Breaking the cycle by making one
//! link direction `Weak` does not fix it either, because the ring is still
//! one strong cycle end to end; the weak link would have to be a specific,
//! constantly-relocating edge as the ring rotates, which is fragile to
//! maintain and easy to get wrong silently.
//!
//! An arena sidesteps the question instead of solving it in `Rc`: every node
//! lives in `Arena::slots`, addressed by a plain `usize` (`NodeId`), and
//! `left`/`right`/`parent`/`child` are indices, not owning pointers. Nothing
//! here can form a reference cycle because indices do not own anything.
//! Unlike a typical arena, though, **a popped node's slot is never freed or
//! recycled** — see [`Arena`]'s own docs for why a recycling arena panics
//! (or worse, silently aliases two logically distinct nodes) the moment a
//! re-entrant comparator pops from inside another pop's `consolidate`, a
//! shape the fuzz grammar in `crates/difffuzz/src/modules/fibonacci_heap.rs`
//! found inside its first fifty generated cases. This is an implementation
//! detail invisible to any caller either way: JavaScript never observes node
//! identity, so nothing about the public API depends on which
//! representation backs it, or on whether an unreachable node's memory is
//! ever reclaimed.
//!
//! # Re-entrancy — the comparator runs *from inside* a sift, again
//!
//! Same tier, same hazard as [`crate::structures::heap`]: the comparator is
//! a callback that can call back into `push`/`pop`/`clear` on the very heap
//! it is comparing. Every accessor here (`item`, `degree`, `set_degree`, …)
//! takes the arena's `RefCell` for exactly one field access and releases it
//! before returning, and the comparator is only ever invoked *between* such
//! accesses, never while one is borrowed — the discipline
//! `crate::structures::heap`'s docs call out by name (a `Ref` alive across a
//! call into re-entrant code is what aborted Node with `SIGABRT` there).
//! `root`/`min`/`size` are `Cell`s for the same reason: a comparator can
//! observe or change them mid-operation, and every read here is fresh, never
//! cached across a comparator call.
//!
//! # `push`'s tie-break, and why it is not `Ordering`
//!
//! `if (!this.min || this.comparator(node.item, this.min.item) <= 0) this.min
//! = node;` — on a tie the **most recently pushed** node becomes the new
//! min, unconditionally. Combined with `consolidate`'s degree-bucket
//! merging, which node ends up at the root after several pops depends on the
//! heap's internal tree shape, not on insertion order alone — this is
//! exactly the tie-break `utils/merge.js`'s k-way algorithms depend on (see
//! `crate::utils::merge`'s module docs, DIV-UTILS-2) and the earlier linear-scan
//! substitute there could not reproduce.

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;

use crate::utils::comparators::{Comparator, Reversed, Thrown};

/// An index into [`Arena::slots`]. Never exposed on the public API; see the
/// module docs for why an arena stands in for upstream's object graph.
type NodeId = usize;

struct Node<T> {
    item: T,
    degree: usize,
    parent: Option<NodeId>,
    child: Option<NodeId>,
    /// Circular doubly-linked list pointers, always valid: a lone node's
    /// `left`/`right` are its own id (upstream's `node.left = node; node.right
    /// = node;`).
    left: NodeId,
    right: NodeId,
}

/// The node table backing every [`FibonacciHeap`]. See the module docs for
/// why this exists instead of `Rc<RefCell<Node<T>>>`.
///
/// # Slots are never recycled, on purpose
///
/// A first cut of this arena freed a node's slot the moment `pop` was done
/// with it and recycled the id on the next `create_node` — measured, this
/// panics under re-entrancy, and the reason is exactly the re-entrancy this
/// tier exists for. `consolidate`'s `nodes` list is a **snapshot** of
/// `NodeId`s taken once, at the top of the call, precisely so it survives
/// whatever a comparator does next (see that method's docs). If a
/// `fibPopper`-shaped comparator (see `crates/difffuzz/src/modules/
/// fibonacci_heap.rs`) runs a **nested** `pop` from inside that
/// `consolidate`, and the nested `pop` happens to `dealloc` a node that is
/// *also* sitting in the outer call's `nodes` snapshot, a recycling arena
/// would let a later `create_node` hand that same id to a brand-new,
/// unrelated node -- and the outer loop would then read or link the wrong
/// node entirely, or, once the slot legitimately went back to `None` between
/// the free and the reuse, panic outright (which is what a differential-fuzz
/// campaign against this exact grammar found in under 50 generated cases).
///
/// JavaScript has no such hazard: `consumeLinkedList`'s array holds real
/// object references, and a JS object stays exactly as it was for as long as
/// *anything* holds a reference to it — including a suspended outer call
/// frame's local variable during re-entrancy — regardless of whether it has
/// been spliced out of every list it used to belong to. Nothing here can
/// synthesize that guarantee from an id that gets handed to someone else, so
/// the arena instead never hands an id back out: a "freed" node's slot keeps
/// its last-known fields forever (unobservable through any public API, since
/// nothing exposes node identity), and the arena grows with the heap's total
/// lifetime creation count rather than its live size. The real-world
/// equivalent is V8 declining to collect an object a suspended stack frame
/// still closes over — bounded differently, but the same shape of promise.
struct Arena<T> {
    slots: Vec<Node<T>>,
}

impl<T> Arena<T> {
    fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Allocate a node and return its id. Never reused once handed out — see
    /// the struct's own docs.
    fn alloc(&mut self, node: Node<T>) -> NodeId {
        self.slots.push(node);
        self.slots.len() - 1
    }

    fn node(&self, id: NodeId) -> &Node<T> {
        &self.slots[id]
    }

    fn node_mut(&mut self, id: NodeId) -> &mut Node<T> {
        &mut self.slots[id]
    }
}

/// A Fibonacci heap.
///
/// `T` is the item type; `C` the comparator; `E` the error a comparator call
/// can fail with. `E` defaults to [`Thrown`], core's own error type, so a
/// heap built over a native comparator (as every test in this module is)
/// needs no explicit third argument; the bridge instantiates
/// `FibonacciHeap<JsSlot, BridgeComparator, napi::Error>` explicitly instead.
///
/// # Why `E` is a type parameter here at all
///
/// [`Comparator<T, E>`] takes `E` as a free parameter rather than an
/// associated type (see that trait's own docs), which means a single `C` can
/// in principle implement it for more than one `E` — [`DefaultComparator`]
/// does, for *every* `E`, since [`Relational`]'s blanket impls do too. So
/// `E` cannot be inferred from `C` alone the way
/// [`Heap`](crate::structures::heap::Heap) infers it from `Store::Error`
/// (a real associated type on a real type parameter already in play there).
/// This heap has no such second type parameter to hang it from, so `E` is
/// carried explicitly, with a default that keeps every native-comparator
/// call site exactly as it would read without one.
///
/// Build one with [`new`](Self::new) (a minimum heap) or
/// [`new_max`](Self::new_max) (`reverseComparator` applied, exactly as
/// upstream's `MaxFibonacciHeap` constructor does).
pub struct FibonacciHeap<T, C, E = Thrown> {
    arena: RefCell<Arena<T>>,
    /// `this.root` — some node in the (circular) root list, or `None` when
    /// empty. Not necessarily the minimum; see `min` for that.
    root: Cell<Option<NodeId>>,
    /// `this.min`.
    min: Cell<Option<NodeId>>,
    /// `this.size`.
    ///
    /// `i64`, not `usize` -- see NOTES.md BUG-FIBONACCI-HEAP-1. `pop`'s `this.size--` runs
    /// AFTER `consolidate`, so a comparator that calls `clear()` (which sets
    /// `this.size = 0`) from inside that `consolidate` leaves the pending
    /// decrement to compute `0 - 1`. JavaScript has no unsigned integers:
    /// that is `-1`, a real (if nonsensical) value the object goes on
    /// holding, not a thrown error. `usize` cannot represent it at all, and
    /// `usize::MAX` (a release-mode wraparound) or a panic (debug mode)
    /// would each be a materially different, more "defensive" answer than
    /// upstream's own silent corruption -- exactly the kind of accidental
    /// improvement this port's bug-for-bug mandate rules out.
    size: Cell<i64>,
    /// Not part of upstream's API. Exists so a native test or the fuzzer can
    /// **measure** whether `consolidate` actually merged two trees, rather
    /// than inferring it from op weights — see the module's fuzz spec and
    /// `docs/modules/fibonacci-heap.md`'s "Fuzz + bench". Incremented once
    /// per [`link`](Self::link) call, which is the one place two trees
    /// become one.
    merges: Cell<u64>,
    comparator: C,
    /// Ties this heap's `Result` error type to `E` without storing one —
    /// `compare`'s `Result<f64, E>` is the only place `E` ever appears.
    /// `fn() -> E` rather than a bare `E` keeps the phantom covariant in `E`,
    /// which matters not at all functionally here but costs nothing either.
    _error: PhantomData<fn() -> E>,
}

impl<T: Clone, C, E> FibonacciHeap<T, C, E> {
    /// `new FibonacciHeap(comparator)`. Comparator validation (`typeof
    /// comparator !== 'function'`) is a JavaScript-value question and lives
    /// at the boundary, exactly as it does for [`crate::structures::heap`].
    pub fn new(comparator: C) -> Self {
        Self {
            arena: RefCell::new(Arena::new()),
            root: Cell::new(None),
            min: Cell::new(None),
            size: Cell::new(0),
            merges: Cell::new(0),
            comparator,
            _error: PhantomData,
        }
    }

    /// `this.size`. `i64`, not `usize` -- see the field's own docs, NOTES.md
    /// BUG-FIBONACCI-HEAP-1.
    pub fn size(&self) -> i64 {
        self.size.get()
    }

    /// `#.clear` — `this.root = null; this.min = null; this.size = 0;`.
    ///
    /// Nodes reachable from the old root are not eagerly freed: upstream
    /// leaves them for the GC to reclaim whenever it gets around to it, and
    /// this arena is under no more obligation than that — nothing observes
    /// arena occupancy from the public API. They are reclaimed the next
    /// time this heap's own slots are reused, or all at once when the heap
    /// itself is dropped.
    pub fn clear(&self) {
        self.root.set(None);
        self.min.set(None);
        self.size.set(0);
    }

    /// `#.peek` — `this.min ? this.min.item : undefined`.
    pub fn peek(&self) -> Option<T> {
        self.min.get().map(|id| self.item(id))
    }

    /// How many times [`link`](Self::link) has run: two trees became one.
    /// See the field's own docs for why this exists.
    pub fn merges(&self) -> u64 {
        self.merges.get()
    }

    /// The comparator this heap was built with.
    ///
    /// Not part of upstream's API (no test reads `this.comparator`
    /// meaningfully), but needed by any comparator that has to reach back
    /// into the very heap holding it — a re-entrant fuzz comparator attaches
    /// itself here after construction, exactly as
    /// `crate::structures::heap::Heap::comparator` is used for the same
    /// purpose.
    pub fn comparator(&self) -> &C {
        &self.comparator
    }

    fn item(&self, id: NodeId) -> T {
        self.arena.borrow().node(id).item.clone()
    }

    fn degree(&self, id: NodeId) -> usize {
        self.arena.borrow().node(id).degree
    }

    fn set_degree(&self, id: NodeId, degree: usize) {
        self.arena.borrow_mut().node_mut(id).degree = degree;
    }

    fn set_parent(&self, id: NodeId, parent: Option<NodeId>) {
        self.arena.borrow_mut().node_mut(id).parent = parent;
    }

    fn child(&self, id: NodeId) -> Option<NodeId> {
        self.arena.borrow().node(id).child
    }

    fn set_child(&self, id: NodeId, child: Option<NodeId>) {
        self.arena.borrow_mut().node_mut(id).child = child;
    }

    fn left(&self, id: NodeId) -> NodeId {
        self.arena.borrow().node(id).left
    }

    fn set_left(&self, id: NodeId, left: NodeId) {
        self.arena.borrow_mut().node_mut(id).left = left;
    }

    fn right(&self, id: NodeId) -> NodeId {
        self.arena.borrow().node(id).right
    }

    fn set_right(&self, id: NodeId, right: NodeId) {
        self.arena.borrow_mut().node_mut(id).right = right;
    }

    /// `createNode(item)` — a lone node is its own left and right neighbour.
    fn create_node(&self, item: T) -> NodeId {
        let id = self.arena.borrow_mut().alloc(Node {
            item,
            degree: 0,
            parent: None,
            child: None,
            left: 0,
            right: 0,
        });

        self.set_left(id, id);
        self.set_right(id, id);

        id
    }

    /// `mergeWithRoot(heap, node)`.
    fn merge_with_root(&self, node: NodeId) {
        match self.root.get() {
            None => self.root.set(Some(node)),
            Some(root) => {
                let root_right = self.right(root);

                self.set_right(node, root_right);
                self.set_left(node, root);
                self.set_left(root_right, node);
                self.set_right(root, node);
            }
        }
    }

    /// `mergeWithChild(parent, node)`.
    fn merge_with_child(&self, parent: NodeId, node: NodeId) {
        match self.child(parent) {
            None => self.set_child(parent, Some(node)),
            Some(child) => {
                let child_right = self.right(child);

                self.set_right(node, child_right);
                self.set_left(node, child);
                self.set_left(child_right, node);
                self.set_right(child, node);
            }
        }
    }

    /// `removeFromRoot(heap, node)`. Note what it does NOT touch: `node`'s
    /// own `left`/`right` are left exactly as they were — only the
    /// *neighbours'* pointers are spliced. `pop` depends on this: it reads
    /// the removed node's `right` immediately afterwards to find "the next
    /// root", and that read has to see the old neighbour, not itself.
    fn remove_from_root(&self, node: NodeId) {
        if self.root.get() == Some(node) {
            self.root.set(Some(self.right(node)));
        }

        let left = self.left(node);
        let right = self.right(node);

        self.set_right(left, right);
        self.set_left(right, left);
    }

    /// `link(heap, y, x)` — `y` becomes a child of `x`.
    fn link(&self, y: NodeId, x: NodeId) {
        self.remove_from_root(y);
        self.set_left(y, y);
        self.set_right(y, y);
        self.merge_with_child(x, y);
        self.set_degree(x, self.degree(x) + 1);
        self.set_parent(y, Some(x));
        self.merges.set(self.merges.get() + 1);
    }

    /// `consumeLinkedList(head)` — walk the circular list starting at `head`
    /// until it loops back, as a snapshot `Vec` captured before any of it is
    /// touched. The `looped` flag mirrors upstream's own loop exactly: a
    /// lone node (`head.right === head`) must still be visited once, not
    /// zero times.
    fn consume_linked_list(&self, head: NodeId) -> Vec<NodeId> {
        let mut nodes = Vec::new();
        let mut node = head;
        let mut looped = false;

        loop {
            if node == head && looped {
                break;
            }

            if node == head {
                looped = true;
            }

            nodes.push(node);
            node = self.right(node);
        }

        nodes
    }
}

impl<T: Clone, E, C: Comparator<T, E>> FibonacciHeap<T, C, E> {
    /// `#.push` — returns `++this.size`. `i64`, matching [`size`](Self::size)
    /// — see that field's own docs, NOTES.md BUG-FIBONACCI-HEAP-1.
    pub fn push(&self, item: T) -> Result<i64, E> {
        let node = self.create_node(item);

        self.merge_with_root(node);

        // `if (!this.min || this.comparator(node.item, this.min.item) <= 0)
        // this.min = node;` -- read `this.min` fresh, so a comparator that
        // re-entered and changed it (via a nested push/pop/clear) is not
        // second-guessed by a value cached before the call.
        let should_replace = match self.min.get() {
            None => true,
            Some(min_id) => {
                let node_item = self.item(node);
                let min_item = self.item(min_id);

                self.comparator.compare(&node_item, &min_item)? <= 0.0
            }
        };

        if should_replace {
            self.min.set(Some(node));
        }

        self.size.set(self.size.get() + 1);

        Ok(self.size.get())
    }

    /// `#.pop` — `undefined` (here, `None`) on an empty heap.
    pub fn pop(&self) -> Result<Option<T>, E> {
        // `if (!this.size) return undefined;` -- a JS falsy check, which
        // only a `size` of exactly `0` (or `NaN`, unreachable here) passes.
        // A NEGATIVE `size` (NOTES.md BUG-FIBONACCI-HEAP-1) is truthy and does NOT satisfy
        // this guard, so a `pop()` that follows the exact re-entrant-`clear`
        // sequence BUG-FIBONACCI-HEAP-1 describes proceeds past this point with `this.min`
        // already `null` -- upstream's next line, `z.child`, is then a
        // `TypeError: Cannot read properties of null (reading 'child')`.
        // Reproduced below as a panic rather than a `Result::Err`: both
        // sides fail hard on this path, by different mechanisms, and
        // building a raised-message channel for a state reachable only
        // through this one adversarial re-entrancy is disproportionate to
        // what any caller -- fuzzer included -- can otherwise reach. See
        // NOTES.md BUG-FIBONACCI-HEAP-1 and `docs/modules/fibonacci-heap.md`.
        if self.size.get() == 0 {
            return Ok(None);
        }

        // The panic message IS the reproduction: upstream's own
        // `TypeError` text for `null.child`, verbatim, so a caller (the
        // fuzz harness included) that catches this panic can use the
        // payload directly as the thrown message rather than needing a
        // translation table that could drift from what Node actually says.
        let z = self
            .min
            .get()
            .expect("Cannot read properties of null (reading 'child')");

        if let Some(child) = self.child(z) {
            // Captured as a `Vec` BEFORE any of it is touched, exactly as
            // upstream's own `consumeLinkedList` call is -- `mergeWithRoot`
            // below rewrites each node's `left`/`right` into the root list,
            // which would corrupt a live traversal of the child list itself.
            let children = self.consume_linked_list(child);

            for node in children {
                self.merge_with_root(node);
                // `delete node.parent;` -- upstream deletes the property
                // outright rather than nulling it; `Option::None` is the
                // same observable absence for every read this port has.
                self.set_parent(node, None);
            }
        }

        self.remove_from_root(z);

        if self.right(z) == z {
            // `z === z.right` -- `z` was the only root left. `removeFromRoot`
            // never touched `z`'s own pointers (see that method's docs), so
            // this reads the value it had on entry: itself.
            self.root.set(None);
            self.min.set(None);
        } else {
            self.min.set(Some(self.right(z)));
            self.consolidate()?;
        }

        // `this.size--` -- AFTER consolidate runs, so `consolidate` sees the
        // pre-decrement count. This is load-bearing: `consolidate`'s own
        // second loop bound is `heap.size`, read fresh on every iteration
        // (see that method's docs).
        self.size.set(self.size.get() - 1);

        // `return z.item;` -- a read, not a removal. The arena never frees
        // `z`'s slot (see `Arena`'s own docs): `z` may still be sitting in a
        // re-entrant caller's own snapshot of the root list, and JavaScript
        // would let that caller go on reading `z.item` too.
        Ok(Some(self.item(z)))
    }

    /// `consolidate(heap)` — merges root-list trees of equal degree until
    /// every root has a distinct one, then re-scans for the new minimum.
    ///
    /// # The second loop's bound is LIVE, not captured
    ///
    /// Upstream's `for (i = 0; i < heap.size; i++)` reads `heap.size`
    /// directly in the loop condition -- not into a local -- so a
    /// re-entrant comparator that calls `clear()` mid-scan (setting
    /// `this.size = 0`) is seen on the very next condition check and the
    /// loop simply stops. Reproduced here with a `while` whose condition is
    /// `self.size.get()`, read fresh every iteration, rather than a `for i
    /// in 0..captured_size`, which would silently diverge under exactly
    /// that re-entrant case.
    ///
    /// # NOTES.md BUG-FIBONACCI-HEAP-3 — `root` can be `null` here too, from a DIFFERENT
    /// re-entrant path than BUG-FIBONACCI-HEAP-1's
    ///
    /// `pop`'s caller only reaches this method when `z.right !== z` held —
    /// "more than one root existed", checked against `z`'s own (frozen)
    /// `right` pointer. That field is never touched by `clear()`, which only
    /// resets the *heap's* `root`/`min`/`size`. So a `clear()` that fires
    /// from inside a **`push`'s** tie-break comparison (not a `pop`'s
    /// `consolidate` — a different call site than BUG-FIBONACCI-HEAP-1's) can leave
    /// `heap.root` `null` while `heap.min` is restored to a real node
    /// immediately afterward, by the same `push`'s own `this.min = node`
    /// line. A later `pop` then reads a perfectly real `z` (no crash at the
    /// BUG-FIBONACCI-HEAP-1 site at all), walks past it, and reaches *this* method with
    /// `heap.root` still `null`. Upstream's `consumeLinkedList(null)` pushes
    /// `null` into its own accumulator on its first iteration and then reads
    /// `null.right` — a `TypeError`, one property name over from BUG-FIBONACCI-HEAP-1's.
    /// Verified by tracing upstream's own deterministic control flow
    /// (`~/upstream-mnemonist/fibonacci-heap.js`'s `consumeLinkedList`), the
    /// same way BUG-FIBONACCI-HEAP-1 was. Reproduced the same way: the panic message below
    /// IS the exact upstream text.
    fn consolidate(&self) -> Result<(), E> {
        let root = self
            .root
            .get()
            .expect("Cannot read properties of null (reading 'right')");
        let nodes = self.consume_linked_list(root);

        // `new Array(heap.size)` upstream is a capacity hint, not a bound --
        // a JS array grows on an out-of-range assignment. `Vec` does the
        // same via the growth loop below, so no size is pre-allocated here.
        let mut table: Vec<Option<NodeId>> = Vec::new();

        for node in nodes {
            let mut x = node;
            let mut degree = self.degree(x);

            while let Some(mut y) = table.get(degree).copied().flatten() {
                let x_item = self.item(x);
                let y_item = self.item(y);

                // `if (heap.comparator(x.item, y.item) > 0) { t = x; x = y;
                // y = t; }` -- after this, `x` is never the greater of the
                // two.
                if self.comparator.compare(&x_item, &y_item)? > 0.0 {
                    std::mem::swap(&mut x, &mut y);
                }

                self.link(y, x);
                table[degree] = None;
                degree += 1;
            }

            while table.len() <= degree {
                table.push(None);
            }

            table[degree] = Some(x);
        }

        // `i64`, matching `size`: if a re-entrant `clear` has driven `size`
        // negative (BUG-FIBONACCI-HEAP-1), `0 < size` is false immediately and this loop
        // runs zero times, exactly as upstream's `for (i = 0; i <
        // heap.size; i++)` does.
        let mut i: i64 = 0;

        while i < self.size.get() {
            if let Some(candidate) = table.get(i as usize).copied().flatten() {
                // `heap.min.item` -- read fresh every iteration, and guarded
                // rather than unwrapped: the only way `this.min` could be
                // null here upstream is a re-entrant `clear()`, which also
                // zeroes `this.size`, so the `while` condition above would
                // already have stopped the loop on the next check. Guarding
                // here rather than asserting is the belt to that braces --
                // see the module docs on why this loop's bound is live.
                if let Some(min_id) = self.min.get() {
                    let candidate_item = self.item(candidate);
                    let min_item = self.item(min_id);

                    if self.comparator.compare(&candidate_item, &min_item)? <= 0.0 {
                        self.min.set(Some(candidate));
                    }
                }
            }

            i += 1;
        }

        Ok(())
    }

    /// `FibonacciHeap.from(iterable, comparator)`, given the items already
    /// materialised. `forEach` itself is a boundary function (DESIGN.md
    /// §3.5) and does not belong in core; this is `N` ordinary `push` calls,
    /// exactly as upstream's own `.from` is.
    pub fn from_iter<I: IntoIterator<Item = T>>(iterable: I, comparator: C) -> Result<Self, E> {
        let heap = Self::new(comparator);

        for item in iterable {
            heap.push(item)?;
        }

        Ok(heap)
    }
}

impl<T: Clone, C, E> FibonacciHeap<T, Reversed<C>, E> {
    /// `new MaxFibonacciHeap(comparator)` — the same heap under
    /// `reverseComparator(comparator)`.
    pub fn new_max(comparator: C) -> Self {
        Self::new(Reversed(comparator))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::comparators::{DefaultComparator, Thrown};

    type Slot = i64;

    fn heap() -> FibonacciHeap<Slot, DefaultComparator> {
        FibonacciHeap::new(DefaultComparator)
    }

    // ---------------------------------------------------------------
    // Upstream's own test/fibonacci-heap.js cases, transcribed.
    // ---------------------------------------------------------------

    #[test]
    fn push_increments_size() {
        let heap = heap();

        heap.push(0).unwrap();
        heap.push(0).unwrap();

        assert_eq!(heap.size(), 2);
    }

    #[test]
    fn peek_reads_the_minimum_without_removing_it() {
        let heap = heap();

        assert_eq!(heap.peek(), None);

        heap.push(3).unwrap();
        heap.push(24).unwrap();

        assert_eq!(heap.peek(), Some(3));

        heap.push(1).unwrap();

        assert_eq!(heap.peek(), Some(1));
    }

    #[test]
    fn pop_drains_in_ascending_order() {
        let heap = heap();

        heap.push(3).unwrap();
        heap.push(34).unwrap();
        heap.push(1).unwrap();
        heap.push(2).unwrap();

        assert_eq!(heap.size(), 4);

        assert_eq!(heap.pop().unwrap(), Some(1));
        assert_eq!(heap.size(), 3);
        assert_eq!(heap.pop().unwrap(), Some(2));
        assert_eq!(heap.size(), 2);
        assert_eq!(heap.pop().unwrap(), Some(3));
        assert_eq!(heap.size(), 1);
        assert_eq!(heap.pop().unwrap(), Some(34));
        assert_eq!(heap.size(), 0);
        assert_eq!(heap.pop().unwrap(), None);
        assert_eq!(heap.size(), 0);
    }

    #[test]
    fn a_max_heap_drains_in_descending_order() {
        let heap: FibonacciHeap<Slot, Reversed<DefaultComparator>> =
            FibonacciHeap::new_max(DefaultComparator);

        heap.push(3).unwrap();
        heap.push(34).unwrap();
        heap.push(1).unwrap();
        heap.push(2).unwrap();

        assert_eq!(heap.size(), 4);

        assert_eq!(heap.pop().unwrap(), Some(34));
        assert_eq!(heap.size(), 3);
        assert_eq!(heap.pop().unwrap(), Some(3));
        assert_eq!(heap.size(), 2);
        assert_eq!(heap.pop().unwrap(), Some(2));
        assert_eq!(heap.size(), 1);
        assert_eq!(heap.pop().unwrap(), Some(1));
        assert_eq!(heap.size(), 0);
    }

    /// Upstream's custom-comparator case, over `{value: ...}` structs --
    /// modelled with a tuple, since core has no JS object.
    #[test]
    fn a_custom_comparator_orders_by_a_projected_field() {
        #[derive(Debug, Clone, Copy, PartialEq)]
        struct ByValue(i64);

        struct FieldComparator;

        impl crate::utils::comparators::Comparator<ByValue, Thrown> for FieldComparator {
            fn compare(&self, a: &ByValue, b: &ByValue) -> Result<f64, Thrown> {
                if a.0 < b.0 {
                    return Ok(-1.0);
                }
                if a.0 > b.0 {
                    return Ok(1.0);
                }
                Ok(0.0)
            }
        }

        let heap = FibonacciHeap::new(FieldComparator);

        heap.push(ByValue(34)).unwrap();
        heap.push(ByValue(2)).unwrap();

        assert_eq!(heap.peek(), Some(ByValue(2)));

        let max_heap = FibonacciHeap::new_max(FieldComparator);

        max_heap.push(ByValue(34)).unwrap();
        max_heap.push(ByValue(2)).unwrap();

        assert_eq!(max_heap.peek(), Some(ByValue(34)));
    }

    #[test]
    fn from_iter_builds_a_heap_from_an_iterable() {
        let heap: FibonacciHeap<i64, DefaultComparator, Thrown> =
            FibonacciHeap::from_iter([45, 56, 23], DefaultComparator).unwrap();

        assert_eq!(heap.size(), 3);
        assert_eq!(heap.peek(), Some(23));
    }

    // ---------------------------------------------------------------
    // Consolidation and degree-merging -- what test/fibonacci-heap.js
    // never reaches. See docs/modules/fibonacci-heap.md.
    // ---------------------------------------------------------------

    /// A push/pop pattern long enough that `consolidate` must link trees of
    /// equal degree more than once per `pop` -- not merely a single link.
    #[test]
    fn consolidation_merges_trees_across_many_pushes_and_pops() {
        let heap = heap();

        for value in 0..64 {
            heap.push(value).unwrap();
        }

        // A single pop already forces the whole root list (64 singleton
        // trees) through `consolidate`, which cannot finish without several
        // same-degree links -- log2(64) levels' worth at minimum.
        assert_eq!(heap.pop().unwrap(), Some(0));
        assert!(
            heap.merges() >= 6,
            "expected at least 6 links consolidating 63 singleton trees, got {}",
            heap.merges()
        );

        let mut popped = Vec::new();

        while let Some(value) = heap.pop().unwrap() {
            popped.push(value);
        }

        assert_eq!(popped, (1..64).collect::<Vec<_>>());
    }

    /// Interleaved push/pop (not drain-then-refill) is what makes
    /// consolidate run repeatedly against a live, changing root list rather
    /// than once against a static one.
    #[test]
    fn interleaved_push_and_pop_stays_sorted_and_merges_repeatedly() {
        let heap = heap();
        let mut reference: Vec<i64> = Vec::new();

        // xorshift32, fixed seed -- deterministic, no external dependency.
        let mut state: u32 = 88172645;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state % 200) as i64
        };

        for step in 0..400 {
            let value = next();

            heap.push(value).unwrap();
            reference.push(value);

            if step % 3 == 0 {
                reference.sort_unstable();
                let expected = reference.remove(0);

                assert_eq!(heap.pop().unwrap(), Some(expected));
            }
        }

        reference.sort_unstable();
        let mut popped = Vec::new();

        while let Some(value) = heap.pop().unwrap() {
            popped.push(value);
        }

        assert_eq!(popped, reference);
        assert!(heap.merges() > 20, "merges={}", heap.merges());
    }

    /// `push`'s tie-break: on an EQUAL comparison the just-pushed node wins,
    /// unconditionally -- `<= 0`, not `< 0`. Both pushed values are `5`, so
    /// nothing here can distinguish which physical node survives; the
    /// heavier tests above are what actually pins the rule's effect via
    /// `merges()`/ordering. This one just confirms the tie itself is not
    /// silently skipped.
    #[test]
    fn push_favours_the_most_recently_pushed_node_on_a_tie() {
        let heap = heap();

        heap.push(5).unwrap();
        heap.push(5).unwrap();

        assert_eq!(heap.peek(), Some(5));
        assert_eq!(heap.size(), 2);
    }

    /// A comparator that pushes into the heap it is comparing must not
    /// deadlock or panic -- the re-entrancy this tier exists for.
    #[test]
    fn a_comparator_may_re_enter_and_push() {
        use std::rc::Rc;

        struct Pushy {
            heap: RefCell<Option<Rc<FibonacciHeap<i64, Self>>>>,
            budget: Cell<u32>,
        }

        impl crate::utils::comparators::Comparator<i64, Thrown> for Pushy {
            fn compare(&self, a: &i64, b: &i64) -> Result<f64, Thrown> {
                if self.budget.get() > 0 {
                    self.budget.set(self.budget.get() - 1);

                    if let Some(heap) = self.heap.borrow().as_ref() {
                        heap.push(999)?;
                    }
                }

                crate::utils::comparators::default_comparator(a, b)
            }
        }

        let heap = Rc::new(FibonacciHeap::new(Pushy {
            heap: RefCell::new(None),
            budget: Cell::new(3),
        }));

        // Comparators live inside `heap.comparator`, a private field of this
        // very module -- reachable here because `tests` is a submodule.
        *heap.comparator.heap.borrow_mut() = Some(Rc::clone(&heap));

        for value in [8, 3, 5, 1, 9, 2, 7, 4, 6] {
            heap.push(value).unwrap();
        }

        // The point is completion without a panic; a re-entrant push during
        // a sift is legitimate upstream and must be here too.
        assert!(heap.size() >= 9);

        let mut popped = Vec::new();

        while let Some(value) = heap.pop().unwrap() {
            popped.push(value);
        }

        let mut sorted = popped.clone();
        sorted.sort_unstable();

        assert_eq!(popped, sorted, "pop must still drain in ascending order");
    }

    /// A comparator that clears the heap it is comparing, mid-`pop`/
    /// `consolidate`. This is the shape gate 6's falsification targets (see
    /// the module doc): if `consolidate`'s "live size" reading regressed to
    /// a value captured once at the top of the loop, this would panic on a
    /// stale node id instead of simply stopping early.
    #[test]
    fn a_comparator_that_clears_the_heap_mid_pop_does_not_panic() {
        use std::rc::{Rc, Weak};

        const ITEMS: i64 = 40;
        // Pushing N items makes exactly N-1 comparator calls (the first push
        // has no `min` to compare against), so the Nth call overall is the
        // FIRST comparison `consolidate` makes on the first `pop` -- which is
        // the call this test needs to land on, not merely "some call during
        // setup".
        const FIRE_ON_CALL: u32 = ITEMS as u32;

        struct Clearer {
            heap: RefCell<Weak<FibonacciHeap<i64, Self>>>,
            calls: Cell<u32>,
        }

        impl crate::utils::comparators::Comparator<i64, Thrown> for Clearer {
            fn compare(&self, a: &i64, b: &i64) -> Result<f64, Thrown> {
                let call = self.calls.get() + 1;

                self.calls.set(call);

                if call == FIRE_ON_CALL {
                    if let Some(heap) = self.heap.borrow().upgrade() {
                        heap.clear();
                    }
                }

                crate::utils::comparators::default_comparator(a, b)
            }
        }

        let heap = Rc::new(FibonacciHeap::new(Clearer {
            heap: RefCell::new(Weak::new()),
            calls: Cell::new(0),
        }));

        *heap.comparator.heap.borrow_mut() = Rc::downgrade(&heap);

        for value in 0..ITEMS {
            heap.push(value).unwrap();
        }

        assert_eq!(heap.size(), ITEMS);

        // The first pop's `consolidate` call makes its first comparison on
        // call number `FIRE_ON_CALL`, which is exactly where the clear
        // fires. The point is that this returns at all, rather than
        // panicking on a node id the clear left dangling -- and that the
        // clear's effect (a live `size` read, not a value cached before the
        // comparator ran) is actually visible afterwards.
        let _ = heap.pop().unwrap();

        // `calls` keeps advancing for the rest of `consolidate` after the
        // clear fires, so it is well past `FIRE_ON_CALL` by the time `pop`
        // returns; what matters is that at least that many calls happened,
        // i.e. the clear's trigger point was reached at all.
        assert!(
            heap.comparator.calls.get() >= FIRE_ON_CALL,
            "the clear's trigger point must have been reached from inside this pop"
        );

        // NOTES.md BUG-FIBONACCI-HEAP-1: `this.size--` runs AFTER `consolidate`, so the
        // clear's `size = 0` is what the decrement actually sees --
        // `0 - 1 == -1`, not `0`. A heap "helpfully" clamped to `0` here
        // would be MORE correct than upstream, which is exactly the
        // divergence this port's bug-for-bug mandate rules out.
        assert_eq!(
            heap.size(),
            -1,
            "BUG-FIBONACCI-HEAP-1: this.size-- after a mid-consolidate clear() lands on -1, not 0"
        );
    }

    /// NOTES.md BUG-FIBONACCI-HEAP-1's second half: once `size` is negative, `pop`'s own
    /// `if (!this.size)` guard no longer catches "nothing left", because a
    /// negative number is truthy in JavaScript. Upstream proceeds into
    /// `z.child` with `z` (`this.min`) already `null` and crashes with a
    /// `TypeError`. `mnemonist-core` has no exceptions, so this is
    /// reproduced as a Rust panic — a different mechanism, the same
    /// outcome: both sides fail hard, deliberately, rather than the port
    /// quietly recovering into a state upstream cannot reach.
    #[test]
    #[should_panic(expected = "Cannot read properties of null (reading 'child')")]
    fn a_pop_after_b_220s_negative_size_panics_matching_upstreams_null_dereference() {
        use std::rc::{Rc, Weak};

        const ITEMS: i64 = 40;
        const FIRE_ON_CALL: u32 = ITEMS as u32;

        struct Clearer {
            heap: RefCell<Weak<FibonacciHeap<i64, Self>>>,
            calls: Cell<u32>,
        }

        impl crate::utils::comparators::Comparator<i64, Thrown> for Clearer {
            fn compare(&self, a: &i64, b: &i64) -> Result<f64, Thrown> {
                let call = self.calls.get() + 1;

                self.calls.set(call);

                if call == FIRE_ON_CALL {
                    if let Some(heap) = self.heap.borrow().upgrade() {
                        heap.clear();
                    }
                }

                crate::utils::comparators::default_comparator(a, b)
            }
        }

        let heap = Rc::new(FibonacciHeap::new(Clearer {
            heap: RefCell::new(Weak::new()),
            calls: Cell::new(0),
        }));

        *heap.comparator.heap.borrow_mut() = Rc::downgrade(&heap);

        for value in 0..ITEMS {
            heap.push(value).unwrap();
        }

        // First pop: fires the clear mid-consolidate, lands size at -1.
        let _ = heap.pop().unwrap();
        assert_eq!(heap.size(), -1);

        // Second pop: `-1` is truthy, the empty-heap guard does not fire,
        // and `self.min.get().expect(...)` panics on the `None` the clear
        // left behind -- upstream's `z.child` on a `null` `z`, one line
        // later.
        let _ = heap.pop();
    }
}

//! Cursor semantics, ported from `obliterator` v2.0.5 (DESIGN.md 3.4, 3.6, 3.7).
//!
//! Roughly 30 upstream modules hand out iterators, and every one of them is an
//! `obliterator/iterator` instance built the same way. This module is that
//! shape, written once. A structure supplies a [`Sequence`] impl — about ten
//! lines — and gets the cursor, the freeze semantics and the gap handling for
//! free.
//!
//! # The three behaviours a natural Rust design gets wrong
//!
//! **1. Not restartable.** `Iterator.prototype[Symbol.iterator]` returns
//! `this`, so draining the same instance twice yields nothing the second time.
//! Rust's [`IntoIterator`] hands out a *fresh* iterator per `for` loop, which
//! silently restarts. So a collection here never implements [`IntoIterator`];
//! it exposes a `values()`-style method that constructs a [`Cursor`], and the
//! cursor is the stateful thing. That is D-06.
//!
//! **2. Hybrid capture — length frozen, elements live.** Upstream:
//!
//! ```js
//! var i = 0, l = sequence.length;              // frozen AT CREATION
//! return new Iterator(function () {
//!   if (i >= l) return {done: true};
//!   return {done: false, value: sequence[i++]}; // read LAZILY
//! });
//! ```
//!
//! It is neither a snapshot nor a live view. Mutating an *element* during
//! iteration **is** visible; changing the *length* is **not**. [`Cursor`]
//! reproduces this exactly: `len` is captured by [`Sequence::freeze`], and
//! every element read goes back through [`Sequence::slot`] against the live
//! source. That is D-08.
//!
//! **3. The shrink window.** Because `i >= l` tests the *frozen* `l`, a source
//! that shrinks mid-iteration is read past its new end and JS yields
//! `{done: false, value: undefined}` — `undefined` values rather than
//! termination. [`Step::Gap`] is that state, kept distinct from
//! [`Step::Done`]. That is D-09 / DESIGN.md 3.7 Option A.
//!
//! # Where the gap can and cannot happen
//!
//! A [`Cursor`] borrows its source with `&'a S`, so within safe Rust the
//! borrow checker forbids the mutation that would open a gap. Reaching
//! [`Step::Gap`] takes either interior mutability (see this module's tests) or
//! the FFI boundary, where JS holds a live handle to the same object and napi
//! hands the cursor a shared reference to it. **That is the point:** the
//! `undefined` gap is a JavaScript-visible behaviour and it stays confined to
//! the layer that has JavaScript in it. Core types never mention `undefined`.

use std::fmt;

/// One step of a [`Cursor`].
///
/// Three states rather than [`Option`], because upstream has three: a value, a
/// *hole* inside the frozen length that the source can no longer fill, and the
/// end. Collapsing the middle one into `Done` is exactly the Option B
/// divergence DESIGN.md 3.7 rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step<T> {
    /// The source supplied an element.
    Item(T),
    /// Inside the frozen length, but the source no longer has this slot.
    ///
    /// JS yields `{done: false, value: undefined}` here and keeps going.
    Gap,
    /// Past the frozen length. `{done: true}`.
    Done,
}

impl<T> Step<T> {
    /// The element, if this step produced one.
    pub fn item(self) -> Option<T> {
        match self {
            Self::Item(value) => Some(value),
            Self::Gap | Self::Done => None,
        }
    }

    /// Transform the element, leaving [`Gap`](Step::Gap) and
    /// [`Done`](Step::Done) alone.
    ///
    /// Exists for the projected walks: a `SparseMap` cursor yields a
    /// `Projected` and the bridge needs whichever half of it that walk is
    /// about, without collapsing the three-state answer into an [`Option`] on
    /// the way — which is the one thing the shrink window cannot survive.
    pub fn map<U>(self, transform: impl FnOnce(T) -> U) -> Step<U> {
        match self {
            Self::Item(value) => Step::Item(transform(value)),
            Self::Gap => Step::Gap,
            Self::Done => Step::Done,
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done)
    }

    pub fn is_gap(&self) -> bool {
        matches!(self, Self::Gap)
    }
}

/// A source a [`Cursor`] can walk: what it freezes, and how it reads live.
///
/// The split between [`freeze`](Sequence::freeze) and [`slot`](Sequence::slot)
/// *is* the hybrid capture. Everything a cursor must not see change goes into
/// `Frozen`; everything it must see change is read through `&self` on each
/// step.
///
/// `Frozen` is an associated type because the frozen state is not always just
/// a length. `FixedDeque.prototype.values` freezes `start`, `capacity` **and**
/// `size`; `Stack.prototype.values` freezes `items.length` and then counts
/// *down*. Both are `Frozen` payloads plus a `slot` that knows the layout, so
/// neither needs a second cursor type.
pub trait Sequence {
    /// What the cursor yields.
    type Item;

    /// Positional state captured once, at cursor creation.
    ///
    /// `()` when the ordinal is the physical index and nothing else matters.
    type Frozen;

    /// Capture the frozen state and the frozen length, together.
    ///
    /// Returned as a pair rather than through two methods so there is no
    /// window in which one could be read from a different version of `self`
    /// than the other.
    fn freeze(&self) -> (Self::Frozen, usize);

    /// Live read of the element at ordinal `ordinal` of this walk.
    ///
    /// `ordinal` counts steps, from `0`; mapping it onto a physical position
    /// is the implementation's job, using `frozen`. Returning `None` for an
    /// ordinal below the frozen length is what opens a [`Step::Gap`], and it
    /// should mean "the source cannot supply this slot any more", never "this
    /// walk is over" — the frozen length already decides that.
    fn slot(&self, frozen: &Self::Frozen, ordinal: usize) -> Option<Self::Item>;
}

/// Everything an in-flight walk consists of, with no borrow of the source.
///
/// This is the *whole* of an upstream cursor's closure state — `i`, `l` and
/// whatever else was frozen — and nothing else. It is separate from [`Cursor`]
/// for one structural reason: **upstream cursors outlive any borrow of the
/// thing they walk.** JS lets a cursor and a mutable handle to the collection
/// coexist; a `&'a S` inside the cursor makes that a compile error.
///
/// Two callers need exactly this shape, and neither can use [`Cursor`]:
///
/// * the napi bridge, where the cursor is a JS object holding a napi reference
///   to a JS-owned parent, and the `&S` only exists for the duration of one
///   `next()` call; and
/// * the differential fuzzer, whose instance holds a set *and* a live cursor
///   over it in one struct, which is self-referential with a borrow inside.
///
/// [`Cursor`] is then the thin ergonomic wrapper for ordinary Rust code.
///
/// [`Debug`](fmt::Debug) is hand-written rather than derived: `derive` would
/// demand `S: Debug` when only `S::Frozen` is ever stored, and `S` is the
/// whole collection.
pub struct CursorState<S>
where
    S: Sequence + ?Sized,
{
    frozen: S::Frozen,
    /// `l` in the upstream closure. Never re-read from the source.
    len: usize,
    /// `i` in the upstream closure.
    ordinal: usize,
}

impl<S> CursorState<S>
where
    S: Sequence + ?Sized,
{
    /// Freeze `source` now, and start at ordinal zero.
    pub fn open(source: &S) -> Self {
        let (frozen, len) = source.freeze();

        Self {
            frozen,
            len,
            ordinal: 0,
        }
    }

    /// Freeze `source` now, but walk it under a caller-chosen projection.
    ///
    /// Some upstream structures hand out **several** iterators over the same
    /// slots. `SparseMap` has three — `keys()`, `values()` and `entries()` —
    /// and all three are the identical closure over the identical frozen
    /// `size`, differing only in what they read out of slot `i`. Rust cannot
    /// give one type three [`Sequence`] impls, so the projection travels in
    /// [`Sequence::Frozen`] and this constructor is how a caller says which
    /// one it wants.
    ///
    /// The length still comes from [`Sequence::freeze`], so the "no window
    /// between the two reads" guarantee of [`open`](CursorState::open) is
    /// unchanged; only the payload is replaced. A source with a single walk
    /// keeps using `open` and never sees this.
    pub fn open_projected(source: &S, frozen: S::Frozen) -> Self {
        let (_, len) = source.freeze();

        Self {
            frozen,
            len,
            ordinal: 0,
        }
    }

    /// Advance one position against the live source — including the gap.
    ///
    /// This is the primitive the napi bridge drives, because it is the only
    /// form that can tell `undefined` from `{done: true}`.
    ///
    /// `source` must be the same source [`open`](CursorState::open) froze.
    /// Nothing enforces that — the type system cannot, which is the price of
    /// dropping the borrow — so the two callers above each hold exactly one
    /// source and pass it back unchanged.
    pub fn step(&mut self, source: &S) -> Step<S::Item> {
        // The frozen length, exactly as upstream's `if (i >= l)`. Asking the
        // source for its current length here would be the whole divergence.
        if self.ordinal >= self.len {
            return Step::Done;
        }

        let ordinal = self.ordinal;
        self.ordinal += 1;

        match source.slot(&self.frozen, ordinal) {
            Some(item) => Step::Item(item),
            None => Step::Gap,
        }
    }

    /// The length captured at creation — `l`, not the source's length now.
    pub fn frozen_len(&self) -> usize {
        self.len
    }

    /// How many steps have been taken — `i`.
    pub fn position(&self) -> usize {
        self.ordinal
    }

    /// Steps remaining before [`Step::Done`], gaps included.
    pub fn remaining(&self) -> usize {
        self.len.saturating_sub(self.ordinal)
    }
}

/// A stateful, non-restartable cursor over a [`Sequence`], for Rust callers.
///
/// The Rust half of an `obliterator/iterator`: a [`CursorState`] plus the
/// borrow that makes it usable as an [`Iterator`]. Construct one per
/// iteration; never hand out a fresh one from something that looks like
/// re-iterating the collection, or behaviour (1) in the module docs is lost.
pub struct Cursor<'a, S>
where
    S: Sequence + ?Sized,
{
    source: &'a S,
    state: CursorState<S>,
}

impl<'a, S> Cursor<'a, S>
where
    S: Sequence + ?Sized,
{
    /// Open a cursor, freezing the source's positional state now.
    pub fn new(source: &'a S) -> Self {
        Self {
            source,
            state: CursorState::open(source),
        }
    }

    /// Open a cursor over one of several walks the source offers.
    ///
    /// See [`CursorState::open_projected`] for why the projection is a
    /// [`Sequence::Frozen`] payload rather than a second impl.
    pub fn projected(source: &'a S, frozen: S::Frozen) -> Self {
        Self {
            source,
            state: CursorState::open_projected(source, frozen),
        }
    }

    /// Advance one position, faithfully — including the gap.
    ///
    /// Rust callers normally want the [`Iterator`] impl instead.
    pub fn step(&mut self) -> Step<S::Item> {
        self.state.step(self.source)
    }

    /// The length captured at creation — `l`, not the source's length now.
    pub fn frozen_len(&self) -> usize {
        self.state.frozen_len()
    }

    /// How many steps have been taken — `i`.
    pub fn position(&self) -> usize {
        self.state.position()
    }

    /// Steps remaining before [`Step::Done`], gaps included.
    pub fn remaining(&self) -> usize {
        self.state.remaining()
    }
}

impl<S> fmt::Debug for CursorState<S>
where
    S: Sequence + ?Sized,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CursorState")
            .field("frozen_len", &self.len)
            .field("position", &self.ordinal)
            .finish_non_exhaustive()
    }
}

impl<S> fmt::Debug for Cursor<'_, S>
where
    S: Sequence + ?Sized,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cursor")
            .field("frozen_len", &self.frozen_len())
            .field("position", &self.position())
            .finish_non_exhaustive()
    }
}

/// Rust-side consumption.
///
/// Note what this does *not* do: [`Cursor`] implements [`Iterator`], but no
/// collection implements [`IntoIterator`] by handing one out, so the
/// non-restartability of behaviour (1) survives. Draining a `Cursor` twice
/// gives nothing the second time, which is precisely upstream.
///
/// Gaps are **skipped**, not terminated on. Rust has no `undefined` to yield,
/// and stopping would turn a shrink into an early end — the divergence this
/// design exists to avoid. Skipping keeps the sequence of *real* elements
/// identical to what a JS caller filtering out `undefined` would see.
impl<S> Iterator for Cursor<'_, S>
where
    S: Sequence + ?Sized,
{
    type Item = S::Item;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.step() {
                Step::Item(item) => return Some(item),
                Step::Gap => continue,
                Step::Done => return None,
            }
        }
    }

    /// Upper bound only. The lower bound is `0` because every remaining step
    /// could turn out to be a gap.
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.remaining()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A source whose backing store can shrink *while a cursor holds it*.
    ///
    /// The interior mutability is the point: it is the only way to reach
    /// [`Step::Gap`] from safe Rust, because an ordinary `&'a S` borrow makes
    /// the shrink a compile error. At the napi boundary the same aliasing
    /// arrives for real, via a shared reference to a JS-owned object.
    struct Shrinkable {
        items: RefCell<Vec<u32>>,
    }

    impl Shrinkable {
        fn new(items: &[u32]) -> Self {
            Self {
                items: RefCell::new(items.to_vec()),
            }
        }

        fn truncate(&self, len: usize) {
            self.items.borrow_mut().truncate(len);
        }

        fn set(&self, index: usize, value: u32) {
            self.items.borrow_mut()[index] = value;
        }

        fn extend(&self, values: &[u32]) {
            self.items.borrow_mut().extend_from_slice(values);
        }
    }

    impl Sequence for Shrinkable {
        type Item = u32;
        type Frozen = ();

        fn freeze(&self) -> ((), usize) {
            ((), self.items.borrow().len())
        }

        fn slot(&self, _frozen: &(), ordinal: usize) -> Option<u32> {
            self.items.borrow().get(ordinal).copied()
        }
    }

    /// A source that walks backwards, like `Stack.prototype.values`, to prove
    /// the ordinal is a step counter rather than an index.
    struct Reversed(Vec<u32>);

    impl Sequence for Reversed {
        type Item = u32;
        /// The frozen length, kept in `Frozen` because `slot` needs it to
        /// convert an ordinal into a physical index.
        type Frozen = usize;

        fn freeze(&self) -> (usize, usize) {
            (self.0.len(), self.0.len())
        }

        fn slot(&self, frozen: &usize, ordinal: usize) -> Option<u32> {
            self.0.get(frozen.checked_sub(ordinal + 1)?).copied()
        }
    }

    #[test]
    fn walks_in_order_and_then_stops() {
        let source = Shrinkable::new(&[3, 6, 9]);
        let mut cursor = Cursor::new(&source);

        assert_eq!(cursor.step(), Step::Item(3));
        assert_eq!(cursor.step(), Step::Item(6));
        assert_eq!(cursor.step(), Step::Item(9));
        assert_eq!(cursor.step(), Step::Done);
        assert_eq!(cursor.step(), Step::Done);
    }

    /// D-06: the cursor is stateful, so a second drain is empty.
    #[test]
    fn is_not_restartable() {
        let source = Shrinkable::new(&[1, 2, 3]);
        let mut cursor = Cursor::new(&source);

        assert_eq!(cursor.by_ref().collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(cursor.collect::<Vec<_>>(), Vec::<u32>::new());
    }

    /// D-06 again, the mixed-consumption case: `next(); next(); [...c]`.
    #[test]
    fn partial_consumption_leaves_the_rest() {
        let source = Shrinkable::new(&[1, 2, 3]);
        let mut cursor = Cursor::new(&source);

        assert_eq!(cursor.next(), Some(1));
        assert_eq!(cursor.next(), Some(2));
        assert_eq!(cursor.collect::<Vec<_>>(), vec![3]);
    }

    /// D-08, first half: element mutation during iteration IS visible.
    #[test]
    fn element_writes_during_iteration_are_visible() {
        let source = Shrinkable::new(&[1, 2, 3]);
        let mut cursor = Cursor::new(&source);

        assert_eq!(cursor.step(), Step::Item(1));
        source.set(2, 99);
        assert_eq!(cursor.step(), Step::Item(2));
        assert_eq!(cursor.step(), Step::Item(99));
    }

    /// D-08, second half: growth is NOT visible, because `l` is frozen.
    #[test]
    fn growth_during_iteration_is_not_visible() {
        let source = Shrinkable::new(&[1, 2]);
        let mut cursor = Cursor::new(&source);

        assert_eq!(cursor.frozen_len(), 2);
        source.extend(&[3, 4, 5]);

        assert_eq!(cursor.by_ref().collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(cursor.frozen_len(), 2);
    }

    /// D-09: shrinking below the frozen length opens gaps rather than ending
    /// the walk. This is the `undefined` window, and it is the assertion that
    /// distinguishes Option A from Option B.
    #[test]
    fn shrinking_opens_a_gap_instead_of_terminating() {
        let source = Shrinkable::new(&[1, 2, 3, 4]);
        let mut cursor = Cursor::new(&source);

        assert_eq!(cursor.step(), Step::Item(1));
        source.truncate(2);

        assert_eq!(cursor.step(), Step::Item(2));
        // Ordinals 2 and 3 are inside the frozen length of 4 but past the
        // source's new end: `{done: false, value: undefined}`, twice.
        assert_eq!(cursor.step(), Step::Gap);
        assert_eq!(cursor.step(), Step::Gap);
        assert_eq!(cursor.step(), Step::Done);
    }

    /// The Rust-side view of the same shrink: gaps vanish, the walk does not
    /// end early, and the surviving elements are exactly the real ones.
    #[test]
    fn the_iterator_view_skips_gaps_rather_than_stopping() {
        let source = Shrinkable::new(&[1, 2, 3, 4]);
        let mut cursor = Cursor::new(&source);

        assert_eq!(cursor.next(), Some(1));
        source.truncate(2);

        assert_eq!(cursor.collect::<Vec<_>>(), vec![2]);
    }

    /// A source that empties completely is all gaps, never an early `Done`.
    #[test]
    fn a_fully_emptied_source_yields_gaps_to_the_frozen_length() {
        let source = Shrinkable::new(&[1, 2, 3]);
        let mut cursor = Cursor::new(&source);

        source.truncate(0);

        assert_eq!(cursor.step(), Step::Gap);
        assert_eq!(cursor.step(), Step::Gap);
        assert_eq!(cursor.step(), Step::Gap);
        assert_eq!(cursor.step(), Step::Done);
    }

    #[test]
    fn frozen_state_can_reverse_the_walk() {
        let source = Reversed(vec![1, 2, 3]);
        let cursor = Cursor::new(&source);

        assert_eq!(cursor.collect::<Vec<_>>(), vec![3, 2, 1]);
    }

    #[test]
    fn an_empty_source_is_done_immediately() {
        let source = Shrinkable::new(&[]);
        let mut cursor = Cursor::new(&source);

        assert_eq!(cursor.frozen_len(), 0);
        assert_eq!(cursor.step(), Step::Done);
    }

    #[test]
    fn reports_position_and_remaining() {
        let source = Shrinkable::new(&[1, 2, 3]);
        let mut cursor = Cursor::new(&source);

        assert_eq!((cursor.position(), cursor.remaining()), (0, 3));
        assert_eq!(cursor.size_hint(), (0, Some(3)));

        cursor.next();

        assert_eq!((cursor.position(), cursor.remaining()), (1, 2));
        assert_eq!(cursor.size_hint(), (0, Some(2)));

        cursor.by_ref().count();

        assert_eq!((cursor.position(), cursor.remaining()), (3, 0));
    }

    /// The detached form is the one the bridge and the fuzzer drive: state in
    /// one place, source supplied per step, no borrow in between. Mutating the
    /// source between steps is legal here precisely because there is no borrow
    /// to conflict with — which is the aliasing JS has natively.
    #[test]
    fn detached_state_walks_a_source_it_does_not_borrow() {
        let mut source = vec![1u32, 2, 3];
        let mut state = CursorState::open(&Slice(source.clone()));

        assert_eq!(state.frozen_len(), 3);
        assert_eq!(state.step(&Slice(source.clone())), Step::Item(1));

        // A plain `&mut` while the walk is in flight: impossible with `Cursor`,
        // routine at the boundary.
        source.truncate(1);

        assert_eq!(state.step(&Slice(source.clone())), Step::Gap);
        assert_eq!(state.position(), 2);
        assert_eq!(state.remaining(), 1);
        assert_eq!(state.step(&Slice(source)), Step::Gap);
        assert_eq!(state.step(&Slice(Vec::new())), Step::Done);
    }

    struct Slice(Vec<u32>);

    impl Sequence for Slice {
        type Item = u32;
        type Frozen = ();

        fn freeze(&self) -> ((), usize) {
            ((), self.0.len())
        }

        fn slot(&self, _frozen: &(), ordinal: usize) -> Option<u32> {
            self.0.get(ordinal).copied()
        }
    }

    /// A source offering two different walks over the same slots — the shape
    /// `SparseMap` has three of. The projection rides in `Frozen`, so one impl
    /// serves both and `open_projected` chooses.
    struct Pairs(Vec<(u32, u32)>);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Half {
        Left,
        Right,
    }

    impl Sequence for Pairs {
        type Item = u32;
        type Frozen = Half;

        fn freeze(&self) -> (Half, usize) {
            (Half::Left, self.0.len())
        }

        fn slot(&self, frozen: &Half, ordinal: usize) -> Option<u32> {
            self.0.get(ordinal).map(|(left, right)| match frozen {
                Half::Left => *left,
                Half::Right => *right,
            })
        }
    }

    #[test]
    fn a_projection_selects_which_walk_without_a_second_impl() {
        let source = Pairs(vec![(1, 10), (2, 20), (3, 30)]);

        // `freeze` still supplies the default walk and, in both cases, the
        // length — which is what keeps the two reads from drifting apart.
        assert_eq!(Cursor::new(&source).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(
            Cursor::projected(&source, Half::Right).collect::<Vec<_>>(),
            vec![10, 20, 30]
        );

        let mut projected = Cursor::projected(&source, Half::Right);
        assert_eq!(projected.frozen_len(), 3);
        assert_eq!(projected.step(), Step::Item(10));
    }

    #[test]
    fn step_projections() {
        assert_eq!(Step::Item(7).item(), Some(7));
        assert_eq!(Step::<u32>::Gap.item(), None);
        assert_eq!(Step::<u32>::Done.item(), None);
        assert!(Step::<u32>::Gap.is_gap());
        assert!(!Step::<u32>::Gap.is_done());
        assert!(Step::<u32>::Done.is_done());
        assert!(!Step::Item(1).is_gap());

        // `map` must not turn a gap into an end, which is the whole of D-09.
        assert_eq!(Step::Item(7).map(|value| value * 2), Step::Item(14));
        assert_eq!(Step::<u32>::Gap.map(|value| value * 2), Step::Gap);
        assert_eq!(Step::<u32>::Done.map(|value| value * 2), Step::Done);
    }
}

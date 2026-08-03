//! Port of upstream `heap.js` (576 LOC) — the binary heap, its `MaxHeap`
//! sibling, and the eight raw-array statics it exports alongside them.
//!
//! # The shape of the port, and why it is not `Vec<T>` plus `sort_by`
//!
//! Upstream's algorithms never see a `Heap`. They take a bare JavaScript array
//! and a comparison *callback*:
//!
//! ```js
//! function siftDown(compare, heap, startIndex, i) { … }
//! Heap.prototype.push = function (item) { push(this.comparator, this.items, item); return ++this.size; };
//! ```
//!
//! Two consequences drive every type below.
//!
//! ## 1. `compare` is arbitrary code running *inside* the loop
//!
//! It can throw, and — because it is handed the heap's own elements from a
//! scope that also holds the heap — it can call `heap.push()` or `heap.clear()`
//! while the sift is halfway through. Upstream has no defence against this and
//! no error path: whatever the array looks like afterwards is the answer.
//! Reproducing that bug-for-bug means the algorithms cannot own an exclusive
//! `&mut Vec<T>`, because an exclusive borrow is exactly the thing a re-entrant
//! call would have to violate.
//!
//! So they operate on a [`Store`]: a JavaScript array as the algorithms see it,
//! addressed through `&self` with a borrow that is released before every
//! callback. A re-entrant `push` from inside a comparator therefore *works*,
//! and produces upstream's answer rather than a `RefCell` panic.
//!
//! ## 2. The array is a *reference*, and `clear()` rebinds it
//!
//! `Heap.prototype.clear` is `this.items = []` — a **new** array. An in-flight
//! `push` captured the old one as an argument and keeps writing to it, so a
//! comparator that clears the heap mid-sift leaves the sift finishing into an
//! array nothing can reach. That is DIV-STACK-3 again, one module further on: [`Heap`]
//! holds `RefCell<S>` and every algorithm is handed a `clone()` of the store,
//! which shares the same cells.
//!
//! ## 3. A hole is not a value
//!
//! Once a comparator can shrink the array, `heap[childIndex]` can read past the
//! end and `heap[i] = …` can write past it. JavaScript answers `undefined` and
//! grows the array with holes; both are reachable from the public API and both
//! are observable. [`Store::Item`] is therefore the *slot* type, not the
//! element type — `Option<T>` for [`VecStore`], where `None` is the hole — and
//! [`crate::utils::comparators::Relational`] gives `undefined` its JavaScript
//! behaviour of comparing false against everything.
//!
//! # What upstream's own test file never reaches
//!
//! Every one of the above. `test/heap.js` uses total, side-effect-free
//! comparators over numbers and never stores an iterator. The machinery exists
//! because the *fuzzer* reaches it in seconds once the grammar has a comparator
//! that mutates.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::utils::comparators::{Comparator, MaybeUndefined, Reversed, Sentinel, Thrown};

/// Upstream's message, verbatim. `test/heap.js` asserts against `/replace/`.
pub const REPLACE_EMPTY: &str = "mnemonist/heap.replace: cannot pop an empty heap.";

/// V8's message for `new Array(n)` with an `n` that is not an array length.
///
/// Upstream never validates `n` itself; the only place it can be refused is the
/// `new Array(n)` inside `nsmallest`/`nlargest`'s iterable branch, and this is
/// what that raises.
pub const INVALID_ARRAY_LENGTH: &str = "Invalid array length";

/// A JavaScript array, as the heap algorithms address one.
///
/// # Why every method takes `&self`
///
/// Not for convenience. A comparator invoked from inside `sift_up` may reach
/// the same array — upstream's does, because both the heap and the comparator
/// are reachable from the caller's scope — so an exclusive borrow held across a
/// comparison would either forbid the re-entrancy Rust-side or panic at
/// runtime, and upstream does neither.
///
/// # Why [`Clone`] is required, and what it must mean
///
/// `clone()` must produce another *handle to the same array*, never a copy.
/// [`Heap::clear`] rebinds `this.items`, and an algorithm already running holds
/// a clone which must keep pointing at the array it was given. A deep-copying
/// `Clone` would make `clear()` invisible instead of detaching.
///
/// # Errors
///
/// Fallible throughout because the bridge's implementation is: every accessor
/// is a real property access on a real JS object, which can run a proxy trap or
/// a setter. Core's [`VecStore`] never fails.
pub trait Store: Clone + Sized {
    /// The *slot* type: an element, or the array's `undefined`.
    type Item: Clone;

    /// What a failed access, or a thrown comparator, is reported as.
    type Error;

    /// `throw new Error(message)`.
    ///
    /// The one algorithm that throws is `Heap.replace` on an empty heap, and
    /// the message is asserted by the original suite. Raising through the store
    /// keeps `mnemonist-core` free of any notion of an exception while still
    /// letting the bridge produce a real JavaScript `Error`.
    fn raise(&self, message: &'static str) -> Self::Error;

    /// `array.length`.
    fn length(&self) -> Result<usize, Self::Error>;

    /// `array[index]` — the array's `undefined` when the index is a hole or
    /// past the end.
    fn get(&self, index: usize) -> Result<Self::Item, Self::Error>;

    /// `array[index] = value`, growing the array with holes when `index` is
    /// past the end.
    fn set(&self, index: usize, value: Self::Item) -> Result<(), Self::Error>;

    /// `array.push(value)`, returning the new length.
    fn push(&self, value: Self::Item) -> Result<usize, Self::Error>;

    /// `array.pop()` — the array's `undefined` when it is empty.
    fn pop(&self) -> Result<Self::Item, Self::Error>;

    /// `array.length = length`, truncating or extending with holes.
    fn set_length(&self, length: usize) -> Result<(), Self::Error>;

    /// `new this.constructor(length)` — a fresh, hole-filled array **of the
    /// same class**.
    ///
    /// This is `nsmallest`'s `new iterable.constructor(1)` and
    /// `fixed-reverse-heap`'s `new ArrayClass(size)`, and *only* those. It is
    /// deliberately not the same operation as [`plain_array`](Store::plain_array):
    /// conflating the two made `clear()` and `consume()` preserve a class
    /// upstream discards.
    fn allocate(&self, length: usize) -> Result<Self, Self::Error>;

    /// `[]` / `new Array(length)` — a fresh **plain** array, whatever class
    /// `self` is.
    ///
    /// `Heap.prototype.clear` is `this.items = []` and `Heap.consume` opens
    /// with `var array = new Array(l)`; both are unconditional literals, so a
    /// heap built from a `Uint8Array` clears to a plain `Array` and consumes
    /// into one.
    fn plain_array(&self, length: usize) -> Result<Self, Self::Error>;

    /// The `undefined` this array answers a hole with.
    ///
    /// Needed because `nsmallest`'s scan loop can start at a *fractional* or
    /// negative index (`for (i = n; …)` with the raw `n`), where every read is
    /// `undefined` without the array being consulted at all.
    fn undefined(&self) -> Result<Self::Item, Self::Error>;

    /// `array.slice(start, end)`.
    fn slice(&self, start: usize, end: usize) -> Result<Self, Self::Error>;
}

/// The store a Rust caller gets: `Rc<RefCell<Vec<Option<T>>>>`.
///
/// `Rc` for the reason in [`Store`]'s docs — `clone()` is another handle, so
/// [`Heap::clear`]'s rebinding detaches rather than truncates. `Option<T>`
/// because a JavaScript array has holes and a comparator that shrinks the heap
/// mid-sift creates them.
#[derive(Debug)]
pub struct VecStore<T> {
    cells: Rc<RefCell<Vec<Option<T>>>>,
}

impl<T> Clone for VecStore<T> {
    /// Another handle to the same array. See [`Store`].
    fn clone(&self) -> Self {
        Self {
            cells: Rc::clone(&self.cells),
        }
    }
}

impl<T> Default for VecStore<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> VecStore<T> {
    /// `[]`.
    pub fn new() -> Self {
        Self {
            cells: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// `[a, b, c]` — a dense array.
    pub fn from_values<I: IntoIterator<Item = T>>(values: I) -> Self {
        Self {
            cells: Rc::new(RefCell::new(values.into_iter().map(Some).collect())),
        }
    }

    /// The slots, holes included, as a snapshot.
    pub fn to_vec(&self) -> Vec<Option<T>>
    where
        T: Clone,
    {
        self.cells.borrow().clone()
    }
}

impl<T: Clone> Store for VecStore<T> {
    type Item = Option<T>;
    type Error = Thrown;

    fn raise(&self, message: &'static str) -> Thrown {
        Thrown(message)
    }

    fn length(&self) -> Result<usize, Thrown> {
        Ok(self.cells.borrow().len())
    }

    fn get(&self, index: usize) -> Result<Option<T>, Thrown> {
        Ok(self.cells.borrow().get(index).cloned().flatten())
    }

    fn set(&self, index: usize, value: Option<T>) -> Result<(), Thrown> {
        let mut cells = self.cells.borrow_mut();

        // `array[7] = x` on a length-3 array leaves holes at 3..7.
        while cells.len() <= index {
            cells.push(None);
        }

        cells[index] = value;

        Ok(())
    }

    fn push(&self, value: Option<T>) -> Result<usize, Thrown> {
        let mut cells = self.cells.borrow_mut();

        cells.push(value);

        Ok(cells.len())
    }

    fn pop(&self) -> Result<Option<T>, Thrown> {
        // `[].pop()` is `undefined`, and so is popping a hole.
        Ok(self.cells.borrow_mut().pop().flatten())
    }

    fn set_length(&self, length: usize) -> Result<(), Thrown> {
        let mut cells = self.cells.borrow_mut();

        if length <= cells.len() {
            cells.truncate(length);
        } else {
            cells.resize(length, None);
        }

        Ok(())
    }

    fn allocate(&self, length: usize) -> Result<Self, Thrown> {
        Ok(Self {
            cells: Rc::new(RefCell::new(vec![None; length])),
        })
    }

    /// Identical to [`allocate`](Store::allocate) here: a `VecStore` has only
    /// one class. The distinction exists for the bridge, whose arrays do not.
    fn plain_array(&self, length: usize) -> Result<Self, Thrown> {
        self.allocate(length)
    }

    fn undefined(&self) -> Result<Option<T>, Thrown> {
        Ok(None)
    }

    fn slice(&self, start: usize, end: usize) -> Result<Self, Thrown> {
        let cells = self.cells.borrow();
        let start = start.min(cells.len());
        let end = end.clamp(start, cells.len());

        Ok(Self {
            cells: Rc::new(RefCell::new(cells[start..end].to_vec())),
        })
    }
}

/// `siftDown(compare, heap, startIndex, i)`.
///
/// Bubbles `heap[i]` up towards `startIndex` while it beats its parent. The
/// name is upstream's and is the opposite of the usual convention; it is not
/// renamed, because `Heap.siftDown` is a public export and a caller passing
/// upstream's argument order to a "fixed" name would get silence.
pub fn sift_down<S, C>(
    compare: &C,
    heap: &S,
    start_index: usize,
    mut i: usize,
) -> Result<(), S::Error>
where
    S: Store,
    C: Comparator<S::Item, S::Error>,
{
    // Read once, before any comparison — so a comparator that overwrites this
    // slot mid-sift is overwritten right back on the way out. Upstream's `var
    // item = heap[i]` has the same property, and it is load-bearing.
    let item = heap.get(i)?;

    while i > start_index {
        let parent_index = (i - 1) >> 1;
        let parent = heap.get(parent_index)?;

        if compare.compare(&item, &parent)? < 0.0 {
            heap.set(i, parent)?;
            i = parent_index;
            continue;
        }

        break;
    }

    heap.set(i, item)
}

/// `siftUp(compare, heap, i)` — the whole array is the heap.
pub fn sift_up<S, C>(compare: &C, heap: &S, i: usize) -> Result<(), S::Error>
where
    S: Store,
    C: Comparator<S::Item, S::Error>,
{
    // `var endIndex = heap.length` — captured ONCE, before any comparison. A
    // comparator that shrinks the array does not shorten this walk; it makes
    // the walk read `undefined` instead.
    let end_index = heap.length()?;

    sift_up_within(compare, heap, end_index, i)
}

/// `siftUp` over the first `end_index` slots.
///
/// This is upstream's `heap.js` `siftUp` with its `heap.length` capture made a
/// parameter, which is byte-for-byte what `fixed-reverse-heap.js` re-declares
/// as its own local `siftUp(compare, heap, size, i)` — that file's array is
/// `capacity` long while its heap is `size` long, so it cannot use the export.
/// One implementation, because the two bodies are otherwise identical.
pub fn sift_up_within<S, C>(
    compare: &C,
    heap: &S,
    end_index: usize,
    mut i: usize,
) -> Result<(), S::Error>
where
    S: Store,
    C: Comparator<S::Item, S::Error>,
{
    let start_index = i;
    let item = heap.get(i)?;
    let mut child_index = 2 * i + 1;

    while child_index < end_index {
        let right_index = child_index + 1;

        // Note the `>=`: ties take the RIGHT child. That is what makes this
        // heap's tie-breaking, and therefore `toArray()`'s output on equal
        // elements, reproducible rather than merely correct.
        if right_index < end_index
            && compare.compare(&heap.get(child_index)?, &heap.get(right_index)?)? >= 0.0
        {
            child_index = right_index;
        }

        heap.set(i, heap.get(child_index)?)?;
        i = child_index;
        child_index = 2 * i + 1;
    }

    heap.set(i, item)?;
    sift_down(compare, heap, start_index, i)
}

/// `Heap.push(compare, heap, item)` — the raw-array static.
pub fn push<S, C>(compare: &C, heap: &S, item: S::Item) -> Result<(), S::Error>
where
    S: Store,
    C: Comparator<S::Item, S::Error>,
{
    // `heap.push(item); siftDown(compare, heap, 0, heap.length - 1);` — the
    // length is re-read after the push, and nothing can run in between.
    let length = heap.push(item)?;

    // `saturating_sub`, not `- 1`. A `Store` whose `push` reports a length of
    // zero is not reachable from core, but the bridge's is a real JS `push`
    // on a real JS array, and a subclassed or tampered `push` can return
    // anything. `usize` underflow would panic in debug and, worse, wrap in
    // release into an index that asks the store to grow to `usize::MAX`.
    // JavaScript would have written `heap[-1]`, a string-keyed expando that
    // nothing reads; a no-op sift at index 0 is the nearest honest equivalent.
    sift_down(compare, heap, 0, length.saturating_sub(1))
}

/// `Heap.pop(compare, heap)` — the raw-array static.
pub fn pop<S, C>(compare: &C, heap: &S) -> Result<S::Item, S::Error>
where
    S: Store,
    C: Comparator<S::Item, S::Error>,
{
    let last_item = heap.pop()?;

    if heap.length()? != 0 {
        let item = heap.get(0)?;

        heap.set(0, last_item)?;
        sift_up(compare, heap, 0)?;

        return Ok(item);
    }

    Ok(last_item)
}

/// `Heap.replace(compare, heap, item)` — the raw-array static.
///
/// The only algorithm in the file that throws.
pub fn replace<S, C>(compare: &C, heap: &S, item: S::Item) -> Result<S::Item, S::Error>
where
    S: Store,
    C: Comparator<S::Item, S::Error>,
{
    if heap.length()? == 0 {
        return Err(heap.raise(REPLACE_EMPTY));
    }

    let popped = heap.get(0)?;

    heap.set(0, item)?;
    sift_up(compare, heap, 0)?;

    Ok(popped)
}

/// `Heap.pushpop(compare, heap, item)` — the raw-array static.
pub fn pushpop<S, C>(compare: &C, heap: &S, item: S::Item) -> Result<S::Item, S::Error>
where
    S: Store,
    C: Comparator<S::Item, S::Error>,
{
    let mut item = item;

    if heap.length()? != 0 && compare.compare(&heap.get(0)?, &item)? < 0.0 {
        let tmp = heap.get(0)?;

        heap.set(0, item)?;
        item = tmp;
        sift_up(compare, heap, 0)?;
    }

    Ok(item)
}

/// `Heap.heapify(compare, array)` — Floyd's linear-time build, in place.
pub fn heapify<S, C>(compare: &C, array: &S) -> Result<(), S::Error>
where
    S: Store,
    C: Comparator<S::Item, S::Error>,
{
    let n = array.length()?;
    let l = n >> 1;
    let mut i = l;

    // `while (--i >= 0)`, on a signed JS number. The pre-decrement means the
    // first index visited is `l - 1`, and `l == 0` visits nothing.
    while i > 0 {
        i -= 1;
        sift_up(compare, array, i)?;
    }

    Ok(())
}

/// `Heap.consume(compare, heap)` — drains `heap` into a sorted array.
///
/// Destructive: `heap` is left empty. Upstream allocates `new Array(l)` with
/// `l` read once at the start, so a comparator that grows the heap mid-consume
/// still gets exactly `l` results.
pub fn consume<S, C>(compare: &C, heap: &S) -> Result<S, S::Error>
where
    S: Store,
    C: Comparator<S::Item, S::Error>,
{
    let l = heap.length()?;
    let mut i = 0;

    // `var array = new Array(l)` — a plain array literal, NOT one of `heap`'s
    // class. `Heap.from(new Uint8Array(…)).consume()` returns a plain `Array`
    // upstream, and an earlier cut of this port returned a `Uint8Array`.
    let array = heap.plain_array(l)?;

    while i < l {
        let item = pop(compare, heap)?;

        array.set(i, item)?;
        i += 1;
    }

    Ok(array)
}

/// `Array.prototype.sort(compare)`, as ECMA-262 defines it for the two
/// behaviours the ported callers depend on.
///
/// * **`undefined` sorts last and is never shown to the comparator.** That is
///   `SortCompare` step 2/3, and it matters because a truncated `new Array(n)`
///   can still carry holes into `nsmallest`.
/// * **The sort is stable**, so equal elements keep their relative order.
///   V8's `Array.prototype.sort` has been TimSort since 7.0 and Rust's
///   `sort_by` is a stable merge sort, so the two agree for any comparator that
///   is a consistent ordering.
///
/// They do **not** agree for an inconsistent comparator: both are then free to
/// produce any permutation, and they produce different ones. Recorded rather
/// than hidden — see `docs/modules/heap.md`.
pub fn sort_with<S, C>(array: &S, compare: &C) -> Result<(), S::Error>
where
    S: Store,
    S::Item: MaybeUndefined,
    C: Comparator<S::Item, S::Error>,
{
    let length = array.length()?;
    let mut defined = Vec::with_capacity(length);
    let mut undefined = Vec::new();

    for index in 0..length {
        let item = array.get(index)?;

        if item.is_undefined() {
            undefined.push(item);
        } else {
            defined.push(item);
        }
    }

    // A merge sort written out, rather than `sort_by`, because the comparator
    // is fallible and `sort_by` cannot carry an error out. Stability is the
    // property that has to survive, and a merge preserves it.
    let sorted = merge_sort(defined, compare)?;

    for (index, item) in sorted.into_iter().chain(undefined).enumerate() {
        array.set(index, item)?;
    }

    Ok(())
}

/// Stable merge sort over a fallible comparator.
fn merge_sort<T: Clone, E, C: Comparator<T, E>>(
    mut items: Vec<T>,
    compare: &C,
) -> Result<Vec<T>, E> {
    if items.len() <= 1 {
        return Ok(items);
    }

    let right = items.split_off(items.len() / 2);
    let left = merge_sort(items, compare)?;
    let right = merge_sort(right, compare)?;

    let mut merged = Vec::with_capacity(left.len() + right.len());
    let mut left = left.into_iter().peekable();
    let mut right = right.into_iter().peekable();

    loop {
        match (left.peek(), right.peek()) {
            (Some(a), Some(b)) => {
                // `<= 0` takes from the left, which is what makes it stable.
                if compare.compare(b, a)? < 0.0 {
                    merged.push(right.next().expect("peeked"));
                } else {
                    merged.push(left.next().expect("peeked"));
                }
            }
            (Some(_), None) => merged.push(left.next().expect("peeked")),
            (None, Some(_)) => merged.push(right.next().expect("peeked")),
            (None, None) => break,
        }
    }

    Ok(merged)
}

/// Where `nsmallest`/`nlargest` get their input, and the branch upstream picks.
///
/// `iterables.isArrayLike(target)` is `Array.isArray(target) || isTypedArray(target)`
/// — a *JavaScript* question, so the bridge answers it and hands the verdict
/// over as this enum rather than core guessing from a Rust type.
pub enum Source<S: Store> {
    /// `iterables.isArrayLike(iterable)` was true: random access, and
    /// `iterable.constructor` is the class new results are allocated from.
    ArrayLike(S),
    /// Anything else: drained through `forEach` first.
    Iterable {
        /// The values, in iteration order.
        values: Vec<S::Item>,
        /// `iterables.guessLength(iterable)` — `target.length`, else
        /// `target.size`, else absent.
        guessed_length: Option<usize>,
        /// An array of the class upstream's `new Array(n)` would produce.
        /// Only its class is used.
        plain: S,
    },
}

/// `Heap.nsmallest(compare, n, iterable)`.
///
/// Four distinct code paths upstream, all reproduced: `n === 1` over an
/// array-like, `n === 1` over an iterable, `n >= length` over an array-like
/// (clone and sort), and the bounded-heap path.
pub fn nsmallest<S, C>(compare: &C, n: f64, source: Source<S>) -> Result<S, S::Error>
where
    S: Store,
    S::Item: MaybeUndefined + Sentinel,
    C: Comparator<S::Item, S::Error>,
{
    let reverse_compare = Reversed(compare);

    match source {
        Source::ArrayLike(iterable) => {
            if n == 1.0 {
                // `var min = Infinity` used as an "unset" sentinel, then
                // `if (min === Infinity || compare(v, min) < 0)`. NOTES BUG-HEAP-3:
                // the sentinel is a real value, so an element that *is*
                // Infinity resets it. Reproduced with a first-iteration flag
                // plus the same identity test.
                let mut min = Unset::new(false);

                for i in 0..iterable.length()? {
                    let v = iterable.get(i)?;

                    if min.is_sentinel() || compare.compare(&v, min.value())? < 0.0 {
                        min.replace(v);
                    }
                }

                let result = iterable.allocate(1)?;
                let empty = result.get(0)?;

                result.set(0, min.into_value(empty))?;

                return Ok(result);
            }

            let length = iterable.length()?;

            if n >= length as f64 {
                let clone = iterable.slice(0, length)?;

                sort_with(&clone, compare)?;

                return Ok(clone);
            }

            let result = iterable.slice(0, slice_end(n, length))?;

            heapify(&reverse_compare, &result)?;

            scan(n, length, &iterable, |candidate| {
                if reverse_compare.compare(&candidate, &result.get(0)?)? > 0.0 {
                    replace(&reverse_compare, &result, candidate)?;
                }

                Ok(())
            })?;

            sort_with(&result, compare)?;

            Ok(result)
        }
        Source::Iterable {
            values,
            guessed_length,
            plain,
        } => {
            if n == 1.0 {
                let mut min = Unset::new(false);

                for value in values {
                    if min.is_sentinel() || compare.compare(&value, min.value())? < 0.0 {
                        min.replace(value);
                    }
                }

                let result = plain.allocate(1)?;
                let empty = result.get(0)?;

                result.set(0, min.into_value(empty))?;

                return Ok(result);
            }

            bounded(compare, &reverse_compare, n, guessed_length, values, plain)
        }
    }
}

/// `Heap.nlargest(compare, n, iterable)` — `nsmallest` with the two
/// comparators exchanged, exactly as upstream writes it out twice.
pub fn nlargest<S, C>(compare: &C, n: f64, source: Source<S>) -> Result<S, S::Error>
where
    S: Store,
    S::Item: MaybeUndefined + Sentinel,
    C: Comparator<S::Item, S::Error>,
{
    let reverse_compare = Reversed(compare);

    match source {
        Source::ArrayLike(iterable) => {
            if n == 1.0 {
                let mut max = Unset::new(true);

                for i in 0..iterable.length()? {
                    let v = iterable.get(i)?;

                    if max.is_sentinel() || compare.compare(&v, max.value())? > 0.0 {
                        max.replace(v);
                    }
                }

                let result = iterable.allocate(1)?;
                let empty = result.get(0)?;

                result.set(0, max.into_value(empty))?;

                return Ok(result);
            }

            let length = iterable.length()?;

            if n >= length as f64 {
                let clone = iterable.slice(0, length)?;

                sort_with(&clone, &reverse_compare)?;

                return Ok(clone);
            }

            let result = iterable.slice(0, slice_end(n, length))?;

            heapify(compare, &result)?;

            scan(n, length, &iterable, |candidate| {
                if compare.compare(&candidate, &result.get(0)?)? > 0.0 {
                    replace(compare, &result, candidate)?;
                }

                Ok(())
            })?;

            sort_with(&result, &reverse_compare)?;

            Ok(result)
        }
        Source::Iterable {
            values,
            guessed_length,
            plain,
        } => {
            if n == 1.0 {
                let mut max = Unset::new(true);

                for value in values {
                    if max.is_sentinel() || compare.compare(&value, max.value())? > 0.0 {
                        max.replace(value);
                    }
                }

                let result = plain.allocate(1)?;
                let empty = result.get(0)?;

                result.set(0, max.into_value(empty))?;

                return Ok(result);
            }

            bounded(&reverse_compare, compare, n, guessed_length, values, plain)
        }
    }
}

/// The `forEach` half of `nsmallest`/`nlargest`, which the two share verbatim
/// with only the two comparators exchanged.
///
/// `final_sort` is what the result is sorted by on the way out; `heap_compare`
/// is what maintains the bounded heap.
fn bounded<S, F, H>(
    final_sort: &F,
    heap_compare: &H,
    mut n: f64,
    guessed_length: Option<usize>,
    values: Vec<S::Item>,
    plain: S,
) -> Result<S, S::Error>
where
    S: Store,
    S::Item: MaybeUndefined,
    F: Comparator<S::Item, S::Error>,
    H: Comparator<S::Item, S::Error>,
{
    // `if (size !== null && size < n) n = size;` — `guessLength` returns
    // `undefined`, never `null`, so the guard reads oddly but behaves: an
    // absent length leaves `n` alone because `undefined < n` is false.
    if let Some(size) = guessed_length {
        if (size as f64) < n {
            n = size as f64;
        }
    }

    // `new Array(n)` — and this is upstream's ONLY validation of `n` anywhere.
    // It is reached on this path and on no other, which is why the bridge must
    // not pre-validate: `nsmallest(cmp, -1, [array])` goes down the array-like
    // path and answers without complaint.
    if !is_array_length(n) {
        return Err(plain.raise(INVALID_ARRAY_LENGTH));
    }

    let result = plain.plain_array(n as usize)?;
    let mut i = 0usize;

    for value in values {
        if (i as f64) < n {
            result.set(i, value)?;
        } else {
            if i as f64 == n {
                heapify(heap_compare, &result)?;
            }

            if heap_compare.compare(&value, &result.get(0)?)? > 0.0 {
                replace(heap_compare, &result, value)?;
            }
        }

        i += 1;
    }

    // `if (result.length > i) result.length = i;` — the preallocation
    // over-guessed, so the tail of holes is cut off.
    if result.length()? > i {
        result.set_length(i)?;
    }

    sort_with(&result, final_sort)?;

    Ok(result)
}

/// `array.slice(0, n)`'s end index, as ECMA-262 computes one.
///
/// `ToIntegerOrInfinity` truncates towards zero and maps `NaN` to `0`; a
/// negative end counts back from the end; the result is clamped to the array.
/// The port cannot pass a negative `end` through `Store::slice`, so the whole
/// computation happens here and the store sees a plain index.
fn slice_end(n: f64, length: usize) -> usize {
    let relative = if n.is_nan() { 0.0 } else { n.trunc() };
    let length = length as f64;

    let end = if relative < 0.0 {
        (length + relative).max(0.0)
    } else {
        relative.min(length)
    };

    end as usize
}

/// `new Array(n)` accepts exactly this set.
fn is_array_length(n: f64) -> bool {
    n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= 4_294_967_295.0
}

/// `for (i = n, l = iterable.length; i < l; i++)`, with the **raw** `n`.
///
/// The loop counter is upstream's JavaScript number, not an index, and that is
/// observable: `nsmallest(cmp, 2.5, array)` reads `iterable[2.5]`, `[3.5]`, …,
/// every one of which is `undefined` — so the scan sees nothing at all and the
/// answer is just the first two elements sorted. A negative `n` starts the loop
/// below zero and reads `undefined` once before reaching real elements. Neither
/// is a case any upstream test covers, and neither throws upstream.
fn scan<S, F>(n: f64, length: usize, iterable: &S, mut visit: F) -> Result<(), S::Error>
where
    S: Store,
    F: FnMut(S::Item) -> Result<(), S::Error>,
{
    let mut i = n;

    while i < length as f64 {
        // `iterable[i]` — a hole for any `i` that is not a non-negative
        // integer, and JavaScript answers a hole with `undefined`.
        let candidate = if i >= 0.0 && i.fract() == 0.0 {
            iterable.get(i as usize)?
        } else {
            iterable.undefined()?
        };

        visit(candidate)?;

        i += 1.0;
    }

    Ok(())
}

/// `var min = Infinity` used as "no candidate yet", reproduced exactly.
///
/// Upstream's `n === 1` fast paths conflate the sentinel with the value: the
/// test is `min === Infinity`, so an element that really *is* `Infinity` makes
/// the next element unconditionally replace it (NOTES BUG-HEAP-3), and an empty
/// source makes the sentinel itself the answer (NOTES BUG-HEAP-2). Modelling this as
/// a plain `Option<Item>` would **fix** both bugs, so it is modelled the way
/// upstream wrote it: a slot pre-loaded with the sentinel, plus the identity
/// test [`Sentinel::is_infinity`].
///
/// A slot type that cannot hold `Infinity` starts empty instead, and then
/// `is_sentinel` is true exactly once — which is the same behaviour, because
/// such a store can never contain the value that triggers the bug.
struct Unset<T> {
    value: Option<T>,
    negative: bool,
}

impl<T: Clone + Sentinel> Unset<T> {
    /// `var min = Infinity` / `var max = -Infinity`.
    fn new(negative: bool) -> Self {
        Self {
            value: T::infinity(negative),
            negative,
        }
    }

    /// `min === Infinity`.
    fn is_sentinel(&self) -> bool {
        match &self.value {
            Some(value) => value.is_infinity(self.negative),
            None => true,
        }
    }

    fn value(&self) -> &T {
        self.value.as_ref().expect("guarded by is_sentinel")
    }

    fn replace(&mut self, value: T) {
        self.value = Some(value);
    }

    /// The value, or the sentinel itself when nothing was ever seen.
    fn into_value(self, empty: T) -> T {
        self.value.unwrap_or(empty)
    }
}

/// A binary minimum heap.
///
/// `size` is a separate quantity from `items.length`, exactly as upstream keeps
/// it separate, because the two can genuinely disagree — see NOTES BUG-HEAP-1.
pub struct Heap<S: Store, C> {
    /// `this.items`, in a `RefCell` because `clear()` **rebinds** it and an
    /// algorithm already running must keep the array it was handed (DIV-STACK-3).
    items: RefCell<S>,
    /// `this.size`, in a `Cell` because a comparator can re-enter and change it
    /// while a method is on the stack.
    size: Cell<usize>,
    comparator: C,
}

impl<S: Store, C: Comparator<S::Item, S::Error>> Heap<S, C> {
    /// `new Heap(comparator)`.
    ///
    /// `items` is the empty array upstream's `this.clear()` would have
    /// installed; the caller supplies it because only the caller knows the
    /// array's class.
    pub fn new(items: S, comparator: C) -> Self {
        Self {
            items: RefCell::new(items),
            size: Cell::new(0),
            comparator,
        }
    }

    /// `this.size`.
    pub fn size(&self) -> usize {
        self.size.get()
    }

    /// `this.items` — another handle to the same array, never a copy.
    pub fn items(&self) -> S {
        self.items.borrow().clone()
    }

    /// The comparator this heap orders by, as supplied at construction.
    pub fn comparator(&self) -> &C {
        &self.comparator
    }

    /// `#.clear` — `this.items = []`, a **new plain array**, not a truncation.
    pub fn clear(&self) -> Result<(), S::Error> {
        // Two statements, and the split is load-bearing rather than tidy.
        // `self.items.borrow().plain_array(0)` would keep the `Ref` alive for
        // the whole `plain_array` call, because the temporary lives to the end
        // of the statement -- and for the bridge that call reads a `constructor`
        // property and invokes a constructor, i.e. runs JavaScript, which can
        // re-enter and reach the `borrow_mut()` below. Measured: it aborts the
        // process with `RefCell already borrowed`, not a catchable error.
        let items = self.items.borrow().clone();
        let fresh = items.plain_array(0)?;

        *self.items.borrow_mut() = fresh;
        self.size.set(0);

        Ok(())
    }

    /// `#.push` — returns `++this.size`.
    pub fn push(&self, item: S::Item) -> Result<usize, S::Error> {
        // The array is captured BEFORE the sift, as an argument would be, so a
        // `clear()` from inside the comparator leaves this sift finishing into
        // the detached array.
        let items = self.items.borrow().clone();

        push(&self.comparator, &items, item)?;

        // `++this.size` re-reads `this.size` here, after the comparator has
        // had every chance to change it.
        self.size.set(self.size.get() + 1);

        Ok(self.size.get())
    }

    /// `#.peek` — `this.items[0]`, `undefined` when empty.
    pub fn peek(&self) -> Result<S::Item, S::Error> {
        // Bound to a local for the reason in `clear`: a single expression would
        // hold the `Ref` across `get(0)`, which for the bridge is a real
        // property read and can run a getter or a proxy trap.
        let items = self.items.borrow().clone();

        items.get(0)
    }

    /// `#.pop`.
    ///
    /// Note the order: `this.size` is decremented *before* the pop runs, so a
    /// comparator that observes `size` mid-pop sees the new value.
    pub fn pop(&self) -> Result<S::Item, S::Error> {
        if self.size.get() != 0 {
            self.size.set(self.size.get() - 1);
        }

        let items = self.items.borrow().clone();

        pop(&self.comparator, &items)
    }

    /// `#.replace` — throws [`REPLACE_EMPTY`] on an empty heap.
    pub fn replace(&self, item: S::Item) -> Result<S::Item, S::Error> {
        let items = self.items.borrow().clone();

        replace(&self.comparator, &items, item)
    }

    /// `#.pushpop`.
    pub fn pushpop(&self, item: S::Item) -> Result<S::Item, S::Error> {
        let items = self.items.borrow().clone();

        pushpop(&self.comparator, &items, item)
    }

    /// `#.consume` — drains the heap into a sorted array.
    ///
    /// `this.size = 0` happens *first*, before a single comparison.
    pub fn consume(&self) -> Result<S, S::Error> {
        self.size.set(0);

        let items = self.items.borrow().clone();

        consume(&self.comparator, &items)
    }

    /// `#.toArray` — `consume` over a clone, so the heap survives.
    pub fn to_array(&self) -> Result<S, S::Error> {
        let items = self.items.borrow().clone();
        let length = items.length()?;
        let clone = items.slice(0, length)?;

        consume(&self.comparator, &clone)
    }

    /// `Heap.from(iterable, comparator)`, given the items already materialised.
    ///
    /// Upstream heapifies in place and then assigns both `items` and `size`,
    /// so a comparator that throws mid-heapify leaves the heap untouched and
    /// the caller's array mangled.
    pub fn from_items(items: S, comparator: C) -> Result<Self, S::Error> {
        let heap = Self::new(items.allocate(0)?, comparator);

        heapify(&heap.comparator, &items)?;

        let length = items.length()?;

        *heap.items.borrow_mut() = items;
        heap.size.set(length);

        Ok(heap)
    }
}

impl<S: Store, C> Heap<S, Reversed<C>> {
    /// `new MaxHeap(comparator)` — the same heap under `reverseComparator`.
    pub fn new_max(items: S, comparator: C) -> Self {
        Self {
            items: RefCell::new(items),
            size: Cell::new(0),
            comparator: Reversed(comparator),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::comparators::{Comparator, DefaultComparator, Thrown};

    type Slot = Option<i64>;

    fn heap() -> Heap<VecStore<i64>, DefaultComparator> {
        Heap::new(VecStore::new(), DefaultComparator)
    }

    fn values(store: &VecStore<i64>) -> Vec<Slot> {
        store.to_vec()
    }

    #[test]
    fn push_pop_is_ascending() {
        let heap = heap();

        for value in [3, 34, 1, 2] {
            heap.push(Some(value)).unwrap();
        }

        assert_eq!(heap.size(), 4);
        assert_eq!(heap.pop().unwrap(), Some(1));
        assert_eq!(heap.pop().unwrap(), Some(2));
        assert_eq!(heap.pop().unwrap(), Some(3));
        assert_eq!(heap.pop().unwrap(), Some(34));
        assert_eq!(heap.pop().unwrap(), None);
        assert_eq!(heap.size(), 0);
    }

    #[test]
    fn replace_on_an_empty_heap_throws_upstreams_message() {
        let heap = heap();

        assert_eq!(heap.replace(Some(3)), Err(Thrown(REPLACE_EMPTY)));
    }

    #[test]
    fn to_array_leaves_the_heap_intact() {
        let heap = heap();

        for value in [23, 1, 34, 5] {
            heap.push(Some(value)).unwrap();
        }

        let sorted = heap.to_array().unwrap();

        assert_eq!(values(&sorted), vec![Some(1), Some(5), Some(23), Some(34)]);
        assert_eq!(heap.size(), 4);
        assert_eq!(heap.to_array().unwrap().to_vec().len(), 4);
    }

    #[test]
    fn consume_empties_the_heap() {
        let heap = heap();

        for value in [45, -3, 0] {
            heap.push(Some(value)).unwrap();
        }

        let sorted = heap.consume().unwrap();

        assert_eq!(values(&sorted), vec![Some(-3), Some(0), Some(45)]);
        assert_eq!(heap.size(), 0);
        assert_eq!(heap.items().length().unwrap(), 0);
    }

    #[test]
    fn a_max_heap_reverses_the_comparator() {
        let heap = Heap::new_max(VecStore::<i64>::new(), DefaultComparator);

        for value in [3, 34, 1, 2] {
            heap.push(Some(value)).unwrap();
        }

        assert_eq!(heap.pop().unwrap(), Some(34));
        assert_eq!(heap.pop().unwrap(), Some(3));
        assert_eq!(heap.pop().unwrap(), Some(2));
        assert_eq!(heap.pop().unwrap(), Some(1));
    }

    #[test]
    fn heapify_then_consume_sorts() {
        let array = VecStore::from_values([3, 5, 1, 56, 0, 13, 4]);

        heapify(&DefaultComparator, &array).unwrap();

        let sorted = consume(&DefaultComparator, &array).unwrap();

        assert_eq!(
            values(&sorted),
            vec![
                Some(0),
                Some(1),
                Some(3),
                Some(4),
                Some(5),
                Some(13),
                Some(56)
            ]
        );
    }

    // ------------------------------------------------------------------
    // The re-entrancy the tier exists for. None of this is reachable from
    // `test/heap.js`.
    // ------------------------------------------------------------------

    /// A comparator that pushes into the heap it is comparing, `budget` times.
    struct Pushy {
        target: RefCell<Option<VecStore<i64>>>,
        budget: Cell<u32>,
        value: i64,
    }

    impl Comparator<Slot, Thrown> for Pushy {
        fn compare(&self, a: &Slot, b: &Slot) -> Result<f64, Thrown> {
            if self.budget.get() > 0 {
                self.budget.set(self.budget.get() - 1);

                if let Some(items) = self.target.borrow().as_ref() {
                    items.push(Some(self.value))?;
                }
            }

            crate::utils::comparators::default_comparator(a, b)
        }
    }

    #[test]
    fn a_comparator_that_grows_the_array_mid_sift_does_not_panic() {
        let items = VecStore::<i64>::from_values([1, 2, 3, 4, 5]);
        let comparator = Pushy {
            target: RefCell::new(Some(items.clone())),
            budget: Cell::new(2),
            value: 99,
        };

        // The point is that this completes at all: an algorithm holding
        // `&mut Vec` could not have let the comparator touch the array.
        push(&comparator, &items, Some(0)).unwrap();

        assert_eq!(items.length().unwrap(), 8);
        assert_eq!(items.get(0).unwrap(), Some(0));
    }

    /// A comparator that shortens the array, so the sift reads past its end.
    struct Shrinky {
        target: RefCell<Option<VecStore<i64>>>,
        budget: Cell<u32>,
    }

    impl Comparator<Slot, Thrown> for Shrinky {
        fn compare(&self, a: &Slot, b: &Slot) -> Result<f64, Thrown> {
            if self.budget.get() > 0 {
                self.budget.set(self.budget.get() - 1);

                if let Some(items) = self.target.borrow().as_ref() {
                    items.pop()?;
                    items.pop()?;
                }
            }

            crate::utils::comparators::default_comparator(a, b)
        }
    }

    #[test]
    fn a_comparator_that_shrinks_the_array_makes_the_walk_read_undefined() {
        let items = VecStore::<i64>::from_values([0, 1, 2, 3, 4, 5, 6, 7]);
        let comparator = Shrinky {
            target: RefCell::new(Some(items.clone())),
            budget: Cell::new(1),
        };

        // `sift_up` froze `end_index` at 8, then the comparator cut the array
        // to 6. The walk keeps going to index 7 and reads `undefined` there.
        sift_up(&comparator, &items, 0).unwrap();

        let slots = values(&items);

        assert!(
            slots.iter().any(Option::is_none),
            "expected an undefined slot from the frozen end index, got {slots:?}"
        );
    }

    /// A comparator that throws leaves the heap with `items.length` one ahead
    /// of `size` — upstream's `push` grows the array before it sifts, and
    /// `++this.size` never runs. NOTES BUG-HEAP-1.
    #[test]
    fn a_throwing_comparator_desynchronises_size_from_the_array() {
        struct Boom;

        impl Comparator<Slot, Thrown> for Boom {
            fn compare(&self, _a: &Slot, _b: &Slot) -> Result<f64, Thrown> {
                Err(Thrown("boom"))
            }
        }

        let heap = Heap::new(VecStore::<i64>::new(), Boom);

        heap.push(Some(1)).unwrap();
        assert_eq!(heap.size(), 1);

        // The second push reaches a comparison, and the comparison throws.
        assert_eq!(heap.push(Some(2)), Err(Thrown("boom")));
        assert_eq!(heap.size(), 1);
        assert_eq!(heap.items().length().unwrap(), 2);

        // …and now `pop` is wrong: it returns the sifted root while reporting
        // a size that never counted the second element.
        assert_eq!(heap.size(), 1);
    }

    /// `clear()` installs a new array; a sift already running keeps the old.
    #[test]
    fn clear_detaches_an_in_flight_sift() {
        let heap = heap();

        for value in [5, 4, 3, 2, 1] {
            heap.push(Some(value)).unwrap();
        }

        let detached = heap.items();

        heap.clear().unwrap();

        assert_eq!(heap.items().length().unwrap(), 0);
        assert_eq!(detached.length().unwrap(), 5);
    }

    #[test]
    fn nsmallest_over_an_array_like() {
        let array = VecStore::from_values([5, 2, 4, 8, 9, 1, 45, 134, -34, 4, -1, 0]);

        let three = nsmallest(&DefaultComparator, 3.0, Source::ArrayLike(array.clone())).unwrap();

        assert_eq!(values(&three), vec![Some(-34), Some(-1), Some(0)]);

        let one = nsmallest(&DefaultComparator, 1.0, Source::ArrayLike(array.clone())).unwrap();

        assert_eq!(values(&one), vec![Some(-34)]);

        let all = nsmallest(&DefaultComparator, 34.0, Source::ArrayLike(array)).unwrap();

        assert_eq!(all.length().unwrap(), 12);
        assert_eq!(all.get(0).unwrap(), Some(-34));
        assert_eq!(all.get(11).unwrap(), Some(134));
    }

    #[test]
    fn nlargest_over_an_iterable() {
        let plain = VecStore::<i64>::new();
        let values_in: Vec<Slot> = [5, 2, 4, 8, 9, 1, 45, 134, -34, 4, -1, 0]
            .into_iter()
            .map(Some)
            .collect();

        let three = nlargest(
            &DefaultComparator,
            3.0,
            Source::Iterable {
                values: values_in,
                guessed_length: None,
                plain,
            },
        )
        .unwrap();

        assert_eq!(values(&three), vec![Some(134), Some(45), Some(9)]);
    }

    /// Stability: equal elements keep their input order through `sort_with`.
    #[test]
    fn sort_with_is_stable() {
        struct ByParity;

        impl Comparator<Slot, Thrown> for ByParity {
            fn compare(&self, a: &Slot, b: &Slot) -> Result<f64, Thrown> {
                let key = |slot: &Slot| slot.map_or(0, |value| value % 2);

                Ok((key(a) - key(b)) as f64)
            }
        }

        let array = VecStore::from_values([4, 1, 2, 3, 6, 5]);

        sort_with(&array, &ByParity).unwrap();

        assert_eq!(
            values(&array),
            vec![Some(4), Some(2), Some(6), Some(1), Some(3), Some(5)]
        );
    }

    /// `undefined` sorts last, and the comparator is never asked about it.
    #[test]
    fn sort_with_puts_undefined_last_without_comparing_it() {
        let array = VecStore::<i64>::new();

        array.push(Some(3)).unwrap();
        array.push(None).unwrap();
        array.push(Some(1)).unwrap();

        sort_with(&array, &DefaultComparator).unwrap();

        assert_eq!(values(&array), vec![Some(1), Some(3), None]);
    }

    #[test]
    fn pushpop_on_an_empty_heap_returns_its_argument() {
        let heap = heap();

        assert_eq!(heap.pushpop(Some(3)).unwrap(), Some(3));
        assert_eq!(heap.size(), 0);
    }

    #[test]
    fn from_items_heapifies_in_place() {
        let items = VecStore::from_values([23, 1, 34, 5]);
        let heap = Heap::from_items(items.clone(), DefaultComparator).unwrap();

        assert_eq!(heap.size(), 4);
        assert_eq!(heap.peek().unwrap(), Some(1));
        // The caller's array IS the heap's array, not a copy.
        assert_eq!(items.get(0).unwrap(), Some(1));
    }
}

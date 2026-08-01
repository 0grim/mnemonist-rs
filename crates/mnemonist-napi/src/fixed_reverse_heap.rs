//! JS bridge for [`mnemonist_core::structures::fixed_reverse_heap`].
//!
//! Thin translation only. Four things the bridge carries.
//!
//! 1. **`ArrayClass` stays a JavaScript constructor.** Unlike
//!    [`crate::hashed_array_tree`], which maps the class onto a
//!    [`PointerWidth`](mnemonist_core::utils::typed_arrays::PointerWidth), this
//!    one keeps the real constructor and allocates through it. It has to:
//!    `test/fixed-reverse-heap.js` asserts `heap.consume() instanceof Uint8Array`
//!    and compares against `new Uint8Array([0, 1, 3])`, and it also uses plain
//!    `Array`, which has no width. Storing through the real typed array is also
//!    where `push(300)` becoming `44` comes from, for free.
//! 2. **The constructor's statement order is load-bearing.**
//!    `new ArrayClass(capacity)` runs *before* either guard, so
//!    `new FixedReverseHeap(Array, -1)` dies with `Array`'s own `RangeError`
//!    rather than with mnemonist's message. Reproduced by allocating first.
//! 3. **The capacity guard is `&&` where `||` was meant** (NOTES B-73), so it
//!    can never fire for a number — `new FixedReverseHeap(Array, 0)` is
//!    accepted and then silently discards every push. Reproduced verbatim,
//!    including the odd `typeof capacity !== 'number'` half.
//! 4. **`arguments.length === 2`** selects the comparator-omitted form. napi
//!    cannot see arity, so the discriminator is "was a third argument
//!    supplied"; the difference is an explicit
//!    `new FixedReverseHeap(Array, cmp, undefined)`.

use std::cell::RefCell;

use mnemonist_core::structures::fixed_reverse_heap::FixedReverseHeap as CoreHeap;
use mnemonist_core::structures::heap::Store;
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::comparators::{coerce_length, is_number, BridgeComparator};
use crate::js_array::JsArray;
use crate::js_slot::JsSlot;

/// Upstream's messages, verbatim.
const BAD_CAPACITY: &str =
    "mnemonist/FixedReverseHeap.constructor: capacity should be a number > 0.";
const BAD_COMPARATOR: &str =
    "mnemonist/FixedReverseHeap.constructor: given comparator should be a function.";

/// A heap of bounded capacity that keeps the `capacity` best items seen.
#[napi(js_name = "FixedReverseHeap")]
pub struct JsFixedReverseHeap {
    /// `RefCell` for D-43's reason, `borrow()`-only for T2's: a comparator can
    /// re-enter while a sift is on the stack, and a `borrow_mut()` anywhere
    /// below would deadlock against itself when it did.
    inner: RefCell<CoreHeap<JsArray, BridgeComparator>>,
    /// `this.capacity`, kept as the JavaScript number it was given.
    capacity: f64,
}

#[napi]
impl JsFixedReverseHeap {
    /// `new FixedReverseHeap(ArrayClass, [comparator], capacity)`.
    #[napi(constructor)]
    pub fn new(
        env: Env,
        array_class: Unknown,
        second: Unknown,
        third: Option<Unknown>,
    ) -> Result<Self> {
        // `if (arguments.length === 2) { capacity = comparator; comparator = null; }`
        let (comparator, capacity) = match third {
            Some(capacity) => (Some(second), capacity),
            None => (None, second),
        };

        // `this.items = new ArrayClass(capacity)` — BEFORE both guards, so a
        // capacity the class itself refuses throws the class's error.
        let items = JsArray::construct(&env, &array_class, &capacity)?;

        let comparator = BridgeComparator::resolve(&env, comparator, BAD_COMPARATOR)?;
        let width = coerce_length(&env, &capacity)?;

        // `if (typeof capacity !== 'number' && capacity <= 0) throw` — `&&`,
        // upstream's, where `||` was meant. NOTES B-73: for any number this
        // short-circuits to false, so the guard protects nothing it was written
        // to protect. Kept exactly, second half included.
        if !is_number(&capacity)? && width <= 0.0 {
            return Err(Error::new(Status::InvalidArg, BAD_CAPACITY.to_owned()));
        }

        // `this.comparator = reverseComparator(this.comparator)` happens inside
        // the core constructor, which is where upstream's line is.
        let slots = if width.is_finite() && width >= 0.0 {
            width as usize
        } else {
            0
        };

        Ok(Self {
            inner: RefCell::new(CoreHeap::new(items, comparator, slots)),
            capacity: width,
        })
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    #[napi(getter)]
    pub fn capacity(&self) -> f64 {
        self.capacity
    }

    /// `this.items` — the live backing array, `capacity` slots long whatever
    /// `size` says.
    #[napi(getter)]
    pub fn items(&self) -> JsSlot {
        self.inner.borrow().items().as_slot()
    }

    /// `#.clear` — `this.size = 0`, and nothing else. The array keeps its
    /// contents, which is why `peek()` afterwards is stale (NOTES B-74).
    #[napi]
    pub fn clear(&self) {
        self.inner.borrow().clear();
    }

    /// `#.push` — returns the new size.
    #[napi]
    pub fn push(&self, env: Env, item: Unknown) -> Result<u32> {
        let item = JsSlot::new(&env, &item)?;

        Ok(self.inner.borrow().push(item)? as u32)
    }

    /// `#.peek` — the **worst** item kept, not the best.
    #[napi]
    pub fn peek(&self) -> Result<JsSlot> {
        self.inner.borrow().peek()
    }

    /// `#.consume` — drains into a sorted `ArrayClass` and resets `size`.
    #[napi]
    pub fn consume(&self) -> Result<JsSlot> {
        Ok(self.inner.borrow().consume()?.as_slot())
    }

    /// `#.toArray` — the same, over `items.slice(0, size)`, so the heap
    /// survives.
    #[napi(js_name = "toArray")]
    pub fn to_array(&self) -> Result<JsSlot> {
        Ok(self.inner.borrow().to_array()?.as_slot())
    }
}

/// Keeps the `Store` import honest — `items()` returns one and the trait must
/// be in scope for the bridge to name its methods.
const _: fn(&JsArray) -> Result<usize> = |array| array.length();

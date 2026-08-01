//! JS bridge for [`mnemonist_core::structures::fixed_deque`].
//!
//! Thin translation only; the `ArrayClass` handling is
//! [`crate::array_class`] and is shared with the other two fixed-capacity
//! modules, as is [`crate::fixed_stack::from_parts`], which is upstream's
//! identical fourteen-line `from` static and reproduces NOTES B-60.
//!
//! Two things here that the `FixedStack` bridge does not have.
//!
//! # `forEach` is the cursor's walk, driven by hand
//!
//! Upstream's `forEach`, `values` and `entries` freeze the *same three
//! quantities* — `capacity`, `size`, `start` — and read `this.items` live on
//! every step. So `forEach` is not a second traversal here: it opens a
//! [`CursorState`] and steps it, which is the same code the cursor runs. The
//! borrow is taken and released per step, because the callback may re-enter and
//! mutate (D-43, B-31), and reproducing upstream's live read *requires* that
//! each read really be a read.
//!
//! This is the one place `FixedDeque` and `FixedStack` genuinely differ:
//! `FixedStack.prototype.forEach` walks a different bound from its own cursor
//! (B-61) and so cannot share the walk.
//!
//! # `#.get` takes a JavaScript number, and upstream's guard has no lower bound
//!
//! ```js
//! FixedDeque.prototype.get = function (index) {
//!   if (this.size === 0 || index >= this.capacity) return;
//!   index = this.start + index;
//!   if (index >= this.capacity) index -= this.capacity;
//!   return this.items[index];
//! };
//! ```
//!
//! `index >= this.capacity` is the only bound, so a **negative** index passes
//! it and the arithmetic then lands on a real slot. Measured on Node 24.18.1,
//! on a capacity-4 deque holding `[3, 4]` after two shifts (`start === 2`):
//!
//! ```text
//! d.get(-1)   // 2   <- shifted out, still returned
//! d.get(-2)   // 1
//! d.get(1.5)  // undefined
//! ```
//!
//! Reproduced, which is why this method reaches
//! [`slot_at`](mnemonist_core::structures::fixed_deque::FixedDeque::slot_at)
//! for the odd cases: the ordinary non-negative integer path goes through the
//! core's `get`, and the negative/fractional one performs upstream's own
//! arithmetic and then reads the physical slot it produced.

use std::cell::RefCell;

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::fixed_deque::{
    FixedDeque as CoreFixedDeque, BAD_CAPACITY, CANNOT_GUESS_LENGTH, MISSING_ARGUMENTS,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::array_class::{capacity_of, ArrayClass, Messages};
use crate::cursor::{yielded, CellCursor};
use crate::fixed_stack::from_parts;
use crate::js_slot::JsSlot;

/// What V8 says when this module's `ArrayClass` is not a constructor.
///
/// Both this file's upstream original and `circular-buffer.js` allocate with
/// `new ArrayClass(this.capacity)` — the bare parameter — where
/// `fixed-stack.js` writes `new this.ArrayClass(...)`. The message is the
/// expression V8 was evaluating, so the two differ.
const NOT_A_CONSTRUCTOR: &str = "ArrayClass is not a constructor";

/// This module's four upstream message strings.
const MESSAGES: Messages = Messages {
    cannot_guess: CANNOT_GUESS_LENGTH,
    missing: MISSING_ARGUMENTS,
    bad: BAD_CAPACITY,
    not_a_constructor: NOT_A_CONSTRUCTOR,
};

/// A fixed-capacity double-ended queue of arbitrary JavaScript values.
#[napi(js_name = "FixedDeque")]
pub struct JsFixedDeque {
    inner: RefCell<CoreFixedDeque<JsSlot>>,
    class: ArrayClass,
}

#[napi]
impl JsFixedDeque {
    /// `new FixedDeque(ArrayClass, capacity)`. See
    /// [`crate::fixed_stack::JsFixedStack::new`] for why both parameters are
    /// `Unknown` rather than `Option<Unknown>` (D-61).
    #[napi(constructor)]
    pub fn new(env: Env, array_class: Unknown, capacity: Unknown) -> Result<Self> {
        if array_class.get_type()? == ValueType::Undefined {
            return Err(Error::new(Status::InvalidArg, MISSING_ARGUMENTS));
        }

        let capacity = capacity_of(&env, &capacity, MISSING_ARGUMENTS, BAD_CAPACITY)?;
        let class = ArrayClass::probe(&env, &array_class, NOT_A_CONSTRUCTOR)?;

        CoreFixedDeque::new(class.backing(), capacity)
            .map(|inner| Self {
                inner: RefCell::new(inner),
                class,
            })
            .map_err(raise)
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    #[napi(getter)]
    pub fn capacity(&self) -> u32 {
        self.inner.borrow().capacity() as u32
    }

    /// `this.start`, which `test/fixed-deque.js` asserts on directly.
    #[napi(getter)]
    pub fn start(&self) -> u32 {
        self.inner.borrow().start() as u32
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// Append. Returns the new size, or throws once full.
    #[napi]
    pub fn push(&self, env: Env, item: Unknown) -> Result<u32> {
        let slot = self.class.coerce(&env, &item)?;

        self.inner
            .borrow_mut()
            .push(slot)
            .map(|size| size as u32)
            .map_err(raise)
    }

    /// Prepend. Returns the new size, or throws once full — with a message
    /// naming `unshift` rather than `push`.
    #[napi]
    pub fn unshift(&self, env: Env, item: Unknown) -> Result<u32> {
        let slot = self.class.coerce(&env, &item)?;

        self.inner
            .borrow_mut()
            .unshift(slot)
            .map(|size| size as u32)
            .map_err(raise)
    }

    #[napi]
    pub fn pop(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow_mut().pop().into()
    }

    #[napi]
    pub fn shift(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow_mut().shift().into()
    }

    #[napi(js_name = "peekFirst")]
    pub fn peek_first(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow().peek_first().into()
    }

    #[napi(js_name = "peekLast")]
    pub fn peek_last(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow().peek_last().into()
    }

    /// `#.get` — bounded by the capacity, not by the size (B-62), and with no
    /// lower bound at all. See the module docs.
    #[napi]
    pub fn get(&self, index: Either<f64, Unknown>) -> Either<JsSlot, Undefined> {
        let inner = self.inner.borrow();

        get_at(
            index,
            inner.size(),
            inner.capacity(),
            inner.start(),
            |logical| inner.get(logical),
            |physical| inner.slot_at(physical),
        )
    }

    /// `#.toArray`, front to back, as an instance of the deque's `ArrayClass`.
    #[napi(js_name = "toArray")]
    pub fn to_array<'env>(&self, env: &'env Env) -> Result<Unknown<'env>> {
        self.class.materialise(env, &self.inner.borrow().to_array())
    }

    /// `#.forEach` — `(value, index, deque)`, front to back, over the size
    /// frozen at entry.
    ///
    /// The walk is the cursor's: `capacity`, `size` and `start` are frozen
    /// together and `this.items` is read live, so a callback that pushes or
    /// pops is visible to the reads after it. `scope` is upstream's
    /// `arguments.length > 1 ? scope : this`; napi cannot see arity, so
    /// `forEach(cb, undefined)` binds the deque here where upstream binds
    /// `undefined`. The omitted case is exact.
    #[allow(clippy::type_complexity)]
    #[napi]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(Either<JsSlot, Undefined>, u32, Object)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let deque = this.object;
        let mut state = CursorState::open(&*self.inner.borrow());
        let mut index = 0u32;

        loop {
            // The borrow ends with this statement, on purpose: the callback
            // below may re-enter and mutate.
            let step = state.step(&self.inner.borrow());
            let value: Either<JsSlot, Undefined> = match step {
                Step::Item(slot) => Either::A(slot),
                Step::Gap => Either::B(()),
                Step::Done => return Ok(()),
            };
            let arguments = (value, index, deque).into();

            match &scope {
                Some(scope) => callback.apply(*scope, arguments)?,
                None => callback.apply(deque, arguments)?,
            };

            index += 1;
        }
    }

    /// A fresh cursor over the values, front to back — the factory half of
    /// D-07.
    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsFixedDeque>) -> Result<JsFixedDequeValues> {
        let source = this.share_with(env, |deque| Ok(&deque.inner))?;

        Ok(JsFixedDequeValues {
            cursor: CellCursor::open(source),
        })
    }

    /// A fresh cursor over `[index, value]` pairs.
    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsFixedDeque>) -> Result<JsFixedDequeEntries> {
        let source = this.share_with(env, |deque| Ok(&deque.inner))?;

        Ok(JsFixedDequeEntries {
            cursor: CellCursor::open(source),
            index: 0,
        })
    }

    /// `FixedDeque.from(iterable, ArrayClass, capacity)` — including the branch
    /// that cannot work; see [`from_parts`] and NOTES B-60.
    #[napi(factory)]
    pub fn from(
        env: Env,
        iterable: Unknown,
        array_class: Unknown,
        capacity: Unknown,
    ) -> Result<Self> {
        let (class, capacity, values) =
            from_parts(&env, &iterable, &array_class, &capacity, &MESSAGES)?;

        CoreFixedDeque::from_array_like(class.backing(), capacity, values)
            .map(|inner| Self {
                inner: RefCell::new(inner),
                class,
            })
            .map_err(raise)
    }
}

/// Upstream's `#.get`, shared by `FixedDeque` and `CircularBuffer` because the
/// two are the same function object there.
///
/// ```js
/// if (this.size === 0 || index >= this.capacity) return;
/// index = this.start + index;
/// if (index >= this.capacity) index -= this.capacity;
/// return this.items[index];
/// ```
///
/// Three paths out:
///
/// * a **non-number** index is `undefined` here. Upstream reaches string
///   concatenation — `2 + "1"` is `"21"`, which the next comparison coerces
///   back to a number — and can therefore return a real element for a numeric
///   string on a large enough deque. Not reproduced; recorded as D-65.
/// * a **non-negative integer** below the capacity goes through the core's
///   `get`, so the ordinary path is the one the fuzzer exercises.
/// * a **negative or fractional** index performs upstream's arithmetic here and
///   then reads the physical slot it produced, which for a negative index on a
///   shifted deque is a real element.
pub(crate) fn get_at(
    index: Either<f64, Unknown>,
    size: usize,
    capacity: usize,
    start: usize,
    logical: impl FnOnce(usize) -> Option<JsSlot>,
    physical: impl FnOnce(usize) -> Option<JsSlot>,
) -> Either<JsSlot, Undefined> {
    let Either::A(index) = index else {
        return Either::B(());
    };

    // `this.size === 0 || index >= this.capacity`. NaN fails the comparison in
    // both languages and falls through, exactly as upstream does — and then
    // dies on the array read below, which is also what upstream does.
    if size == 0 || index >= capacity as f64 {
        return Either::B(());
    }

    if index >= 0.0 && index.fract() == 0.0 {
        return logical(index as usize).into();
    }

    let mut slot = start as f64 + index;

    if slot >= capacity as f64 {
        slot -= capacity as f64;
    }

    if !(slot >= 0.0 && slot.fract() == 0.0) {
        return Either::B(());
    }

    physical(slot as usize).into()
}

/// Surface a core error with upstream's own message.
fn raise(error: mnemonist_core::structures::fixed_deque::Error) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

/// The cursor `FixedDeque.prototype.values()` hands out.
#[napi(iterator, js_name = "FixedDequeValues")]
pub struct JsFixedDequeValues {
    cursor: CellCursor<JsFixedDeque, CoreFixedDeque<JsSlot>>,
}

impl Generator for JsFixedDequeValues {
    type Yield = Either<JsSlot, Undefined>;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        yielded(self.cursor.step())
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}

/// The cursor `FixedDeque.prototype.entries()` hands out.
#[napi(iterator, js_name = "FixedDequeEntries")]
pub struct JsFixedDequeEntries {
    cursor: CellCursor<JsFixedDeque, CoreFixedDeque<JsSlot>>,
    /// `j` in upstream's `[j++, value]`, advanced only on a yielded step.
    index: u32,
}

impl Generator for JsFixedDequeEntries {
    type Yield = (u32, Either<JsSlot, Undefined>);
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        let value: Either<JsSlot, Undefined> = match self.cursor.step() {
            Step::Item(slot) => Either::A(slot),
            Step::Gap => Either::B(()),
            Step::Done => return None,
        };
        let index = self.index;

        self.index += 1;

        Some((index, value))
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}

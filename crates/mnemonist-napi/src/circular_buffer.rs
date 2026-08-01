//! JS bridge for [`mnemonist_core::structures::circular_buffer`].
//!
//! Structurally the `FixedDeque` bridge with two methods replaced, which is
//! what upstream is: `CircularBuffer.prototype` is `FixedDeque.prototype`
//! copied key by key, and then `push` and `unshift` are overwritten. The
//! shared pieces are shared here too —
//! [`crate::fixed_deque::get_at`] for `#.get`, [`crate::fixed_stack::from_parts`]
//! for the `from` static (NOTES B-60), [`crate::array_class`] for the class.
//!
//! The one behavioural difference worth naming at the bridge: **`push` and
//! `unshift` cannot fail here**, so they return a `u32` rather than a
//! `Result<u32>`. What they return when the buffer is full is the size
//! *unchanged*, which is the only externally visible signal that an element was
//! dropped.

use std::cell::RefCell;

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::circular_buffer::{
    CircularBuffer as CoreCircularBuffer, BAD_CAPACITY, CANNOT_GUESS_LENGTH, MISSING_ARGUMENTS,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::array_class::{capacity_of, ArrayClass, Messages};
use crate::cursor::{yielded, CellCursor};
use crate::fixed_deque::get_at;
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

/// A fixed-capacity ring of arbitrary JavaScript values that overwrites its
/// oldest element rather than refusing a new one.
#[napi(js_name = "CircularBuffer")]
pub struct JsCircularBuffer {
    inner: RefCell<CoreCircularBuffer<JsSlot>>,
    class: ArrayClass,
}

#[napi]
impl JsCircularBuffer {
    /// `new CircularBuffer(ArrayClass, capacity)`. See
    /// [`crate::fixed_stack::JsFixedStack::new`] for the parameter types (D-61).
    #[napi(constructor)]
    pub fn new(env: Env, array_class: Unknown, capacity: Unknown) -> Result<Self> {
        if array_class.get_type()? == ValueType::Undefined {
            return Err(Error::new(Status::InvalidArg, MISSING_ARGUMENTS));
        }

        let capacity = capacity_of(&env, &capacity, MISSING_ARGUMENTS, BAD_CAPACITY)?;
        let class = ArrayClass::probe(&env, &array_class, NOT_A_CONSTRUCTOR)?;

        CoreCircularBuffer::new(class.backing(), capacity)
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

    /// `this.start`, which `test/circular-buffer.js` asserts on directly.
    #[napi(getter)]
    pub fn start(&self) -> u32 {
        self.inner.borrow().start() as u32
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// Append, overwriting the oldest element when full.
    ///
    /// Returns the new size — which is the size **unchanged** when it
    /// overwrote, and is the only signal a caller gets that something was
    /// dropped.
    #[napi]
    pub fn push(&self, env: Env, item: Unknown) -> Result<u32> {
        let slot = self.class.coerce(&env, &item)?;

        Ok(self.inner.borrow_mut().push(slot) as u32)
    }

    /// Prepend, overwriting the newest element when full.
    #[napi]
    pub fn unshift(&self, env: Env, item: Unknown) -> Result<u32> {
        let slot = self.class.coerce(&env, &item)?;

        Ok(self.inner.borrow_mut().unshift(slot) as u32)
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

    /// Bounded by the capacity, not by the size — B-62, inherited literally
    /// because upstream pastes the same function.
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

    /// `#.toArray`, front to back, as an instance of the buffer's `ArrayClass`.
    #[napi(js_name = "toArray")]
    pub fn to_array<'env>(&self, env: &'env Env) -> Result<Unknown<'env>> {
        self.class.materialise(env, &self.inner.borrow().to_array())
    }

    /// `#.forEach` — the pasted `FixedDeque` walk, so a callback that pushes
    /// mid-iteration sees its own overwrite in the reads that follow.
    #[allow(clippy::type_complexity)]
    #[napi]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(Either<JsSlot, Undefined>, u32, Object)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let buffer = this.object;
        let mut state = CursorState::open(&*self.inner.borrow());
        let mut index = 0u32;

        loop {
            // The borrow ends with this statement: the callback may re-enter.
            let step = state.step(&self.inner.borrow());
            let value: Either<JsSlot, Undefined> = match step {
                Step::Item(slot) => Either::A(slot),
                Step::Gap => Either::B(()),
                Step::Done => return Ok(()),
            };
            let arguments = (value, index, buffer).into();

            match &scope {
                Some(scope) => callback.apply(*scope, arguments)?,
                None => callback.apply(buffer, arguments)?,
            };

            index += 1;
        }
    }

    #[napi]
    pub fn values(
        &self,
        env: Env,
        this: Reference<JsCircularBuffer>,
    ) -> Result<JsCircularBufferValues> {
        let source = this.share_with(env, |buffer| Ok(&buffer.inner))?;

        Ok(JsCircularBufferValues {
            cursor: CellCursor::open(source),
        })
    }

    #[napi]
    pub fn entries(
        &self,
        env: Env,
        this: Reference<JsCircularBuffer>,
    ) -> Result<JsCircularBufferEntries> {
        let source = this.share_with(env, |buffer| Ok(&buffer.inner))?;

        Ok(JsCircularBufferEntries {
            cursor: CellCursor::open(source),
            index: 0,
        })
    }

    /// `CircularBuffer.from(iterable, ArrayClass, capacity)`.
    ///
    /// Copies by index and assigns `size`, so it does **not** overwrite: an
    /// oversized iterable leaves `size > capacity` on the one class whose whole
    /// purpose is to prevent that. Upstream's behaviour, reproduced.
    #[napi(factory)]
    pub fn from(
        env: Env,
        iterable: Unknown,
        array_class: Unknown,
        capacity: Unknown,
    ) -> Result<Self> {
        let (class, capacity, values) =
            from_parts(&env, &iterable, &array_class, &capacity, &MESSAGES)?;

        CoreCircularBuffer::from_array_like(class.backing(), capacity, values)
            .map(|inner| Self {
                inner: RefCell::new(inner),
                class,
            })
            .map_err(raise)
    }
}

/// Surface a core error with upstream's own message.
fn raise(error: mnemonist_core::structures::circular_buffer::Error) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

/// The cursor `CircularBuffer.prototype.values()` hands out.
#[napi(iterator, js_name = "CircularBufferValues")]
pub struct JsCircularBufferValues {
    cursor: CellCursor<JsCircularBuffer, CoreCircularBuffer<JsSlot>>,
}

impl Generator for JsCircularBufferValues {
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

/// The cursor `CircularBuffer.prototype.entries()` hands out.
#[napi(iterator, js_name = "CircularBufferEntries")]
pub struct JsCircularBufferEntries {
    cursor: CellCursor<JsCircularBuffer, CoreCircularBuffer<JsSlot>>,
    index: u32,
}

impl Generator for JsCircularBufferEntries {
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

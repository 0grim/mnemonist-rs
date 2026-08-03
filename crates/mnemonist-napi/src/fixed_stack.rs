//! JS bridge for [`mnemonist_core::structures::fixed_stack`].
//!
//! Thin translation only: every behavioural decision lives in the core crate.
//! What the bridge has to *carry* is the `ArrayClass` argument, which core
//! reduces to two bits and which JavaScript needs back in full — see
//! [`crate::array_class`].
//!
//! Six things worth knowing about.
//!
//! 1. **The element type is [`JsSlot`]**, so a fixed stack holds anything
//!    upstream's does, and `push(o); pop() === o` holds for objects.
//! 2. **Values are coerced by the class itself** on the way in. `push(300)`
//!    into a `Uint8Array`-backed stack stores `44`, because the value is round
//!    tripped through a real one-element instance of the caller's class.
//! 3. **`toArray` returns an instance of that class.** `new ArrayClass(size)`,
//!    then only the present slots are written, so a missing one is a hole in an
//!    `Array` and the class zero in a typed array — which is exactly what
//!    upstream's `array[i] = undefined` produces.
//! 4. **`from` reproduces NOTES BUG-UTILS-ITERABLES-2.** `iterables.forEach` does not exist, so
//!    every iterable that is not array-like dies with a `TypeError` naming it.
//!    That is upstream, verified on Node 24.18.1, and it is the reason
//!    `FixedStack.from(new Set([1, 2, 3]), Array, 3)` throws.
//! 5. **`forEach` walks `items.length`** — BUG-FIXED-STACK-1 — and re-reads the array on
//!    every step, so a callback that mutates is visible to the reads after it.
//! 6. **The core structure is held in a [`RefCell`].** Not for interior
//!    mutability's sake but because a `&self` on a `Freeze` type is `noalias
//!    readonly` to LLVM, which hoisted a read out of exactly this loop once
//!    before. See `crate::cursor::CellCursor` (DIV-STACK-5, PORTBUG-1).
//!
//! `inspect` is not ported: a Node display convenience with no upstream
//! assertion.

use std::cell::RefCell;

use mnemonist_core::cursor::Step;
use mnemonist_core::structures::fixed_stack::{
    FixedStack as CoreFixedStack, BAD_CAPACITY, CANNOT_GUESS_LENGTH, MISSING_ARGUMENTS,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::array_class::{capacity_of, ArrayClass, Messages};
use crate::cursor::{yielded, CellCursor};
use crate::foreach;
use crate::iterables;
use crate::js_slot::JsSlot;

/// What `iterables.forEach(...)` raises, because `utils/iterables.js` exports
/// no `forEach`. Verbatim from Node 24.18.1; see NOTES BUG-UTILS-ITERABLES-2.
pub(crate) const NO_ITERABLES_FOREACH: &str = "iterables.forEach is not a function";

/// What V8 says when `FixedStack`'s `ArrayClass` is not a constructor.
///
/// `fixed-stack.js` allocates with `new this.ArrayClass(this.capacity)`, so the
/// message names the property. Its two siblings use the bare parameter and get
/// a different message; see [`crate::array_class::ArrayClass`].
const NOT_A_CONSTRUCTOR: &str = "this.ArrayClass is not a constructor";

/// This module's four upstream message strings.
const MESSAGES: Messages = Messages {
    cannot_guess: CANNOT_GUESS_LENGTH,
    missing: MISSING_ARGUMENTS,
    bad: BAD_CAPACITY,
    not_a_constructor: NOT_A_CONSTRUCTOR,
};

/// A LIFO stack of fixed capacity holding arbitrary JavaScript values.
#[napi(js_name = "FixedStack")]
pub struct JsFixedStack {
    inner: RefCell<CoreFixedStack<JsSlot>>,
    class: ArrayClass,
}

#[napi]
impl JsFixedStack {
    /// `new FixedStack(ArrayClass, capacity)`.
    ///
    /// Both parameters are [`Unknown`] rather than `Option<Unknown>` because
    /// napi maps a JS `null` to `None` and the original suite distinguishes the
    /// two: `new FixedStack(Array)` must throw about the *Array class* and
    /// `new FixedStack(Array, null)` about the *number*. napi passes
    /// `undefined` for a missing argument and does not enforce arity
    /// (`CallbackInfo::new(.., None, ..)`), so "absent" and "explicitly
    /// `undefined`" collapse — the one divergence, recorded as DIV-FIXED-STACK-3.
    #[napi(constructor)]
    pub fn new(env: Env, array_class: Unknown, capacity: Unknown) -> Result<Self> {
        // `arguments.length < 2`, as far as napi can see it.
        if array_class.get_type()? == ValueType::Undefined {
            return Err(Error::new(Status::InvalidArg, MISSING_ARGUMENTS));
        }

        let capacity = capacity_of(&env, &capacity, MISSING_ARGUMENTS, BAD_CAPACITY)?;
        let class = ArrayClass::probe(&env, &array_class, NOT_A_CONSTRUCTOR)?;

        CoreFixedStack::new(class.backing(), capacity)
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

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// Upstream returns the new size, and throws once the stack is full.
    #[napi]
    pub fn push(&self, env: Env, item: Unknown) -> Result<u32> {
        let slot = self.class.coerce(&env, &item)?;

        self.inner
            .borrow_mut()
            .push(slot)
            .map(|size| size as u32)
            .map_err(raise)
    }

    /// `undefined` on an empty stack, which is upstream's bare `return;`.
    #[napi]
    pub fn pop(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow_mut().pop().into()
    }

    /// `this.items[this.size - 1]` — `undefined` when empty, without an error.
    #[napi]
    pub fn peek(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow().peek().into()
    }

    /// `#.toArray`, newest first, as an instance of the stack's `ArrayClass`.
    #[napi(js_name = "toArray")]
    pub fn to_array<'env>(&self, env: &'env Env) -> Result<Unknown<'env>> {
        self.class.materialise(env, &self.inner.borrow().to_array())
    }

    /// `#.toJSON`, which upstream defines as `toArray`.
    #[napi(js_name = "toJSON")]
    pub fn to_json<'env>(&self, env: &'env Env) -> Result<Unknown<'env>> {
        self.to_array(env)
    }

    /// `#.toString`, which upstream defines as `toArray().join(',')`.
    ///
    /// `Array.prototype.join` renders `undefined` and holes as the empty
    /// string, which is what a missing slot becomes here.
    #[napi(js_name = "toString")]
    pub fn to_js_string(&self, env: Env) -> Result<String> {
        let slots: Vec<JsSlot> = self
            .inner
            .borrow()
            .to_array()
            .into_iter()
            .map(|slot| slot.unwrap_or(JsSlot::Undefined))
            .collect();

        foreach::join(&env, &slots)
    }

    /// `#.forEach` — **`items.length` iterations**, newest first, with
    /// `(value, index, stack)`.
    ///
    /// Two fidelity notes:
    ///
    /// * The bound is the array's length, not `this.size` (BUG-FIXED-STACK-1), so an
    ///   under-full stack hands the callback its unused slots first. The loop
    ///   bound is captured once and `this.items` is re-read every iteration, so
    ///   a callback that pushes or pops is visible to the reads that follow.
    /// * `scope` is upstream's `arguments.length > 1 ? scope : this`. napi's
    ///   typed signature cannot distinguish an omitted argument from an
    ///   explicit `undefined`, so `forEach(cb, undefined)` binds the stack here
    ///   where upstream binds `undefined`. The omitted case — the only one the
    ///   original suite uses — is exact.
    // The callback type is spelled out rather than aliased because napi's macro
    // reads the parameter type syntactically to generate the binding.
    #[allow(clippy::type_complexity)]
    #[napi]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(Either<JsSlot, Undefined>, u32, Object)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let stack = this.object;
        let length = self.inner.borrow().items_len();

        for index in 0..length {
            // The borrow ends with this statement, on purpose: the callback
            // below may re-enter and mutate, and upstream's re-read of
            // `this.items` is only reproduced if this read is genuinely a read.
            let value: Either<JsSlot, Undefined> =
                self.inner.borrow().lifo_slot(length, index).into();
            let arguments = (value, index as u32, stack).into();

            match &scope {
                Some(scope) => callback.apply(*scope, arguments)?,
                None => callback.apply(stack, arguments)?,
            };
        }

        Ok(())
    }

    /// A fresh cursor over the values, newest first — the *factory* half of
    /// DIV-STACK-2. `crate::cursor::install_iterator_factories` aliases
    /// `Symbol.iterator` onto this.
    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsFixedStack>) -> Result<JsFixedStackValues> {
        let source = this.share_with(env, |stack| Ok(&stack.inner))?;

        Ok(JsFixedStackValues {
            cursor: CellCursor::open(source),
        })
    }

    /// A fresh cursor over `[index, value]` pairs, newest first.
    ///
    /// The index is the cursor's own step counter: upstream's `[i++, value]`
    /// counts up while the value walks down.
    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsFixedStack>) -> Result<JsFixedStackEntries> {
        let source = this.share_with(env, |stack| Ok(&stack.inner))?;

        Ok(JsFixedStackEntries {
            cursor: CellCursor::open(source),
            index: 0,
        })
    }

    /// `FixedStack.from(iterable, ArrayClass, capacity)`.
    ///
    /// Reproduces both halves of upstream's static, including the half that
    /// cannot work: see [`from_parts`].
    #[napi(factory)]
    pub fn from(
        env: Env,
        iterable: Unknown,
        array_class: Unknown,
        capacity: Unknown,
    ) -> Result<Self> {
        let (class, capacity, values) =
            from_parts(&env, &iterable, &array_class, &capacity, &MESSAGES)?;

        CoreFixedStack::from_array_like(class.backing(), capacity, values)
            .map(|inner| Self {
                inner: RefCell::new(inner),
                class,
            })
            .map_err(raise)
    }
}

/// The shared body of the three `X.from(iterable, ArrayClass, capacity)`
/// statics, which are the same fourteen lines in all three upstream files.
///
/// ```js
/// if (arguments.length < 3) {
///   capacity = iterables.guessLength(iterable);
///   if (typeof capacity !== 'number') throw new Error(...);
/// }
///
/// var x = new X(ArrayClass, capacity);
///
/// if (iterables.isArrayLike(iterable)) { ...copy by index...; return x; }
///
/// iterables.forEach(iterable, function (value) { x.push(value); });
/// ```
///
/// The last line is **NOTES BUG-UTILS-ITERABLES-2**: `utils/iterables.js` exports `isArrayLike`,
/// `guessLength`, `toArray` and `toArrayWithIndices`, and no `forEach`. So the
/// branch that would handle a `Set`, a `Map`, a generator or a string is not a
/// slow path — it is a `TypeError`, and it has been one since the modules were
/// written. Verified on Node 24.18.1 for all three classes:
///
/// ```text
/// FixedStack.from(new Set([1,2,3]), Array, 3)
/// TypeError: iterables.forEach is not a function
/// ```
///
/// Reproduced rather than repaired (DIV-FIXED-STACK-6). A port that quietly made the branch
/// work would pass every upstream test and be a different library.
///
/// Note also the *order*: the structure is constructed — so a bad capacity
/// throws — **before** `isArrayLike` is consulted, and `guessLength` runs
/// before that. Three different errors, in a fixed order.
///
/// # The one divergence here: `size` when `.length` is not a number
///
/// `stack.size = l` assigns the *iterable's* `length` with no type check, and
/// `isArrayLike` is `Array.isArray || ArrayBuffer.isView` — which is true for a
/// **`DataView`**, and a `DataView` has `byteLength`, not `length`. Upstream
/// therefore produces a structure whose `size` is `undefined`:
///
/// ```text
/// FixedStack.from(new DataView(new ArrayBuffer(4)), Array, 3).size  // undefined
///                                             …and .toArray()      // [ undefined ]
/// ```
///
/// That is NOTES BUG-FIXED-STACK-2, and it is the one behaviour here the port does not
/// reproduce (DIV-FIXED-STACK-7): a `usize` cannot hold `undefined`, and every later method
/// would be arithmetic on `NaN`. [`iterables::array_like_values`] yields
/// nothing for such a target and `size` becomes `0` — "nothing was copied",
/// which is at least true. Reachable only with an explicit capacity; without
/// one `guessLength` throws first.
#[allow(clippy::type_complexity)]
pub(crate) fn from_parts(
    env: &Env,
    iterable: &Unknown,
    array_class: &Unknown,
    capacity: &Unknown,
    messages: &Messages,
) -> Result<(ArrayClass, usize, Vec<JsSlot>)> {
    // `arguments.length < 3`, as far as napi can see it (DIV-FIXED-STACK-3).
    let capacity = if capacity.get_type()? == ValueType::Undefined {
        match iterables::guess_length(env, iterable)? {
            Some(length) => length,
            None => return Err(Error::new(Status::InvalidArg, messages.cannot_guess)),
        }
    } else {
        return_capacity(env, capacity, messages.missing, messages.bad)?
    };

    // Upstream reaches this through `new X(ArrayClass, capacity)`, so the
    // capacity guards run here even when the number came from `guessLength`.
    let capacity_value = env.create_double(capacity)?;
    // SAFETY: a handle this call just created, in this scope.
    let capacity_value =
        unsafe { Unknown::from_raw_unchecked(env.raw(), napi::JsValue::raw(&capacity_value)) };
    let capacity = capacity_of(env, &capacity_value, messages.missing, messages.bad)?;

    let class = ArrayClass::probe(env, array_class, messages.not_a_constructor)?;

    if !iterables::is_array_like(env, iterable)? {
        return Err(crate::foreach::type_error(env, NO_ITERABLES_FOREACH));
    }

    let values = iterables::array_like_values(env, iterable)?
        .into_iter()
        .map(|slot| {
            let value = slot.get(env)?;

            class.coerce(env, &value)
        })
        .collect::<Result<Vec<JsSlot>>>()?;

    Ok((class, capacity, values))
}

/// An explicitly supplied capacity, as a JS number.
fn return_capacity(env: &Env, capacity: &Unknown, missing: &str, bad: &str) -> Result<f64> {
    if capacity.get_type()? == ValueType::Undefined {
        return Err(Error::new(Status::InvalidArg, missing.to_owned()));
    }

    if capacity.get_type()? != ValueType::Number {
        return Err(Error::new(Status::InvalidArg, bad.to_owned()));
    }

    crate::foreach::to_number(env, capacity)
}

/// Surface a core error with upstream's own message.
fn raise(error: mnemonist_core::structures::fixed_stack::Error) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

/// The cursor `FixedStack.prototype.values()` hands out.
#[napi(iterator, js_name = "FixedStackValues")]
pub struct JsFixedStackValues {
    cursor: CellCursor<JsFixedStack, CoreFixedStack<JsSlot>>,
}

impl Generator for JsFixedStackValues {
    /// `Either<_, Undefined>` rather than `Option<_>`: napi renders `None` as
    /// `null`, and the shrink window needs a real `undefined` (DIV-FIXED-STACK-1).
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

/// The cursor `FixedStack.prototype.entries()` hands out.
#[napi(iterator, js_name = "FixedStackEntries")]
pub struct JsFixedStackEntries {
    cursor: CellCursor<JsFixedStack, CoreFixedStack<JsSlot>>,
    /// `i` in upstream's `[i++, value]`, advanced only on a yielded step.
    index: u32,
}

impl Generator for JsFixedStackEntries {
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

//! JS bridge for [`mnemonist_core::structures::stack`].
//!
//! Thin translation only: every behavioural decision lives in the core crate.
//! What is worth knowing about is what the bridge has to *carry* rather than
//! decide.
//!
//! 1. **The element type is [`JsSlot`], so a stack holds anything.** Upstream
//!    pushes arbitrary JS values, and a bridge that narrowed them to numbers
//!    would pass the original suite while quietly being a different data
//!    structure. A slot is a refcounted handle, so `push(x); pop() === x` holds
//!    for objects, not just for primitives.
//! 2. **`from` goes through the real dispatch.** [`crate::foreach::collect`] is
//!    the five-branch coercion, unmodified; `Stack.from(new Map(...))` behaves
//!    the way upstream's does because it is the same code path, not a
//!    simplified one.
//! 3. **`of` is installed as JavaScript, deliberately.** See
//!    [`crate::statics`]: `Stack.of = function () { return Stack.from(arguments); }`
//!    is upstream's definition, and `arguments` has no Rust representation, so
//!    this is the only way `of` puts a real one through the real dispatch.
//! 4. **`forEach`'s `scope` argument.** Upstream keys off `arguments.length`,
//!    which napi's typed signature cannot see; same divergence, and same
//!    reasoning, as the `SparseSet` bridge.
//! 5. **`inspect` is not ported.** A Node display convenience with no upstream
//!    assertion and no Rust equivalent.
//! 6. **The core structure is held in a [`RefCell`].** Not for interior
//!    mutability's sake — every method below could take `&mut self` — but
//!    because a `&self` on a `Freeze` type is `noalias readonly` to LLVM, and
//!    JavaScript can and does mutate this object from inside a callback
//!    `forEach` is running. See `crate::cursor::CellCursor` for the measured
//!    failure. Every borrow is released before any JS call, so a re-entrant
//!    `push` never meets an outstanding one.

use std::cell::RefCell;

use mnemonist_core::cursor::Step;
use mnemonist_core::structures::stack::Stack as CoreStack;
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::cursor::{yielded, CellCursor};
use crate::foreach;
use crate::js_slot::JsSlot;

/// A LIFO stack of arbitrary JavaScript values.
#[napi(js_name = "Stack")]
pub struct JsStack {
    inner: RefCell<CoreStack<JsSlot>>,
}

#[napi]
impl JsStack {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(CoreStack::new()),
        }
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// Upstream returns the new size, not the instance.
    #[napi]
    pub fn push(&self, env: Env, item: Unknown) -> Result<u32> {
        let slot = JsSlot::new(&env, &item)?;

        Ok(self.inner.borrow_mut().push(slot) as u32)
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

    /// `#.toArray`, newest first.
    #[napi(js_name = "toArray")]
    pub fn to_array(&self) -> Vec<JsSlot> {
        self.inner.borrow().to_vec()
    }

    /// `#.toJSON`, which upstream defines as `toArray`.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Vec<JsSlot> {
        self.inner.borrow().to_vec()
    }

    /// `#.toString`, which upstream defines as `toArray().join(',')`.
    #[napi(js_name = "toString")]
    pub fn to_js_string(&self, env: Env) -> Result<String> {
        let slots = self.inner.borrow().to_vec();

        foreach::join(&env, &slots)
    }

    /// `#.forEach` — newest first, with `(value, index, stack)`.
    ///
    /// Two fidelity notes:
    ///
    /// * The loop bound is captured once but `this.items` is re-read on every
    ///   iteration, so a callback that calls `clear()` sends the remaining
    ///   reads into the *new* array and they arrive as `undefined`. That is
    ///   [`CoreStack::lifo_slot`], and it is a different read from the one
    ///   `values()` performs.
    /// * `scope` is upstream's `arguments.length > 1 ? scope : this`. napi's
    ///   typed signature cannot distinguish an omitted argument from an
    ///   explicit `undefined`, so `forEach(cb, undefined)` binds the stack here
    ///   where upstream binds `undefined`. The omitted case — the only one the
    ///   original suite uses — is exact.
    // The callback type is spelled out rather than aliased because napi's
    // macro reads the parameter type syntactically to generate the binding;
    // behind a `type` it stops recognising the `Function` shape.
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

    /// A fresh cursor over the values, newest first.
    ///
    /// The *factory* half of D-07: every call constructs a new cursor object,
    /// so `[...stack]` works repeatedly while each cursor is individually
    /// non-restartable. `crate::cursor::install_iterator_factories` aliases
    /// `Symbol.iterator` onto this, exactly as upstream's `stack.js` does on
    /// its last-but-one line.
    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsStack>) -> Result<JsStackValues> {
        let source = this.share_with(env, |stack| Ok(&stack.inner))?;

        Ok(JsStackValues {
            cursor: CellCursor::open(source),
        })
    }

    /// A fresh cursor over `[index, value]` pairs, newest first.
    ///
    /// The index is the cursor's own step counter, not a position in `items` —
    /// upstream's `[i++, value]` counts up while the value walks down.
    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsStack>) -> Result<JsStackEntries> {
        let source = this.share_with(env, |stack| Ok(&stack.inner))?;

        Ok(JsStackEntries {
            cursor: CellCursor::open(source),
            index: 0,
        })
    }

    /// `Stack.from(iterable)` — the five-branch coercion, then push in order.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown) -> Result<Self> {
        Ok(Self {
            inner: RefCell::new(foreach::collect(&env, iterable)?.into_iter().collect()),
        })
    }
}

impl Default for JsStack {
    fn default() -> Self {
        Self::new()
    }
}

/// The cursor `Stack.prototype.values()` hands out.
#[napi(iterator, js_name = "StackValues")]
pub struct JsStackValues {
    cursor: CellCursor<JsStack, CoreStack<JsSlot>>,
}

impl Generator for JsStackValues {
    /// `Either<_, Undefined>` rather than `Option<_>`: napi renders `None` as
    /// `null`, and the shrink window needs a real `undefined` (D-39).
    type Yield = Either<JsSlot, Undefined>;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        yielded(self.cursor.step())
    }

    /// Upstream cursors have no `return` method, so a `break` out of a `for…of`
    /// leaves the walk exactly where it stopped and a later `next()` resumes.
    /// napi's default `complete` is the same observable behaviour.
    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}

/// The cursor `Stack.prototype.entries()` hands out.
#[napi(iterator, js_name = "StackEntries")]
pub struct JsStackEntries {
    cursor: CellCursor<JsStack, CoreStack<JsSlot>>,
    /// `i` in upstream's `[i++, value]`, advanced only on a yielded step.
    index: u32,
}

impl Generator for JsStackEntries {
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

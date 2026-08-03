//! JS bridge for [`mnemonist_core::structures::bit_vector`].
//!
//! Thin translation only. The bit-level adaptations are the `bit_set` bridge's
//! and are documented there; two are specific to this module.
//!
//! 1. **The growth policy is a JS function called from Rust.** The core takes a
//!    `Box<dyn Fn(f64) -> Option<f64>>`, where `None` means upstream's
//!    "not a number". A JS policy can also *throw*, which no `Option` can
//!    express, so [`JsPolicy`] parks the exception in a `RefCell` and the
//!    calling method re-raises it in preference to whatever the core reported.
//!    Without that, a throwing policy would surface as
//!    "policy returned an invalid value", which is a different error from a
//!    different place.
//! 2. **`grow`, `push` and `set` can throw**, so they return `Result`. Every
//!    message is the core's [`fmt::Display`](std::fmt::Display), which is
//!    upstream's verbatim.
//!
//! The core structure is held in a [`RefCell`] for the same reason as in
//! [`crate::queue`] and [`crate::stack`]: `&self` on a `Freeze` type is
//! `noalias readonly`, so a `forEach` callback's mutation would be invisible.
//! Note that the pre-existing `thrown: Rc<RefCell<…>>` field did **not** buy
//! that — an `Rc` reaches its `UnsafeCell` through a pointer, so `Rc<RefCell<T>>`
//! is itself `Freeze`. The cell has to be inline. See
//! [`crate::cursor::CellCursor`] and PORTBUG-1.
//!
//! # This is the one module where the borrow cannot be kept off the JS call
//!
//! Everywhere else in the bridge, a `RefCell` borrow ends before any JavaScript
//! runs — `forEach` re-borrows per step, `DefaultMap::get` runs its factory
//! between the read and the write. Here it cannot: the **growth policy is a JS
//! function that `mnemonist-core` calls from inside `grow`**, so `push`, `set`,
//! `grow`, `resize`, `reallocate` and `apply_policy` all hold the vector while
//! a JS function runs, and a policy that re-enters meets an outstanding borrow.
//!
//! A `RefCell` panic inside a `#[napi]` method **aborts the process** — napi
//! 3.12 does not `catch_unwind` a sync call, and a panic unwinding out of an
//! `extern "C"` frame is an abort. Measured, not assumed. So every borrow here
//! is fallible and raises [`REENTRANT_POLICY`] instead.
//!
//! That is a stated divergence: upstream's policy may read and even mutate the
//! vector mid-growth, and gets whatever half-grown state it finds. This port
//! refuses instead, catchably. Refusing is a narrower behaviour than aborting,
//! and it is narrower than upstream's — recorded rather than hidden.

use std::cell::RefCell;
use std::rc::Rc;

use mnemonist_core::structures::bit_vector::{
    default_policy, BitVector as CoreVector, Error as CoreError,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::bit_set::clears;
use crate::cursor::BridgeBitCursor;

/// Raised when a growth policy calls back into the vector that is growing.
const REENTRANT_POLICY: &str = "mnemonist-rs/BitVector: the growth policy called back into the \
     vector while it was growing. Upstream serves such a call from a half-grown \
     vector; this port refuses it, because the vector is mid-operation and cannot \
     answer honestly. See the module docs and PORTBUG-1.";

/// A growable bit vector over a `Uint32Array`.
#[napi(js_name = "BitVector")]
pub struct JsBitVector {
    inner: RefCell<CoreVector>,
    /// Where [`JsPolicy`] leaves an exception thrown by the JS policy.
    thrown: Rc<RefCell<Option<Error>>>,
}

#[napi]
impl JsBitVector {
    /// `new BitVector(initialLengthOrOptions)`.
    ///
    /// The options object reads `initialLength || initialCapacity || 0`, so an
    /// `initialCapacity` becomes the **length**. That is upstream, it is what
    /// upstream's own "custom policy" test depends on, and it is resolved here
    /// so the core sees only a length.
    #[napi(constructor)]
    pub fn new(env: Env, initial_length_or_options: Option<Either<f64, Object>>) -> Result<Self> {
        let thrown = Rc::new(RefCell::new(None));

        let (initial_length, policy) = match initial_length_or_options {
            None => (0usize, None),
            // `var initialLength = initialLengthOrOptions || 0`.
            Some(Either::A(length)) => (count(length), None),
            Some(Either::B(options)) => {
                let length = options
                    .get::<f64>("initialLength")?
                    .map(count)
                    .filter(|value| *value != 0)
                    .or_else(|| {
                        options
                            .get::<f64>("initialCapacity")
                            .ok()
                            .flatten()
                            .map(count)
                    })
                    .unwrap_or(0);

                (
                    length,
                    options.get::<Function<f64, Unknown<'static>>>("policy")?,
                )
            }
        };

        let inner = match policy {
            None => CoreVector::new(initial_length),
            Some(policy) => {
                let policy = JsPolicy {
                    // A JS function reference has to outlive the call that
                    // created it, so it is promoted to a `FunctionRef`.
                    callable: policy.create_ref()?,
                    env,
                    thrown: Rc::clone(&thrown),
                };

                CoreVector::with_policy(
                    initial_length,
                    Box::new(move |capacity| policy.call(capacity)),
                )
            }
        };

        Ok(Self {
            inner: RefCell::new(inner),
            thrown,
        })
    }

    #[napi(getter)]
    pub fn length(&self) -> Result<u32> {
        Ok(self.read()?.length() as u32)
    }

    /// Upstream's `size` counter. Signed; see BUG-SPARSE-QUEUE-SET-2 and the `push`/`pop` defects.
    #[napi(getter)]
    pub fn size(&self) -> Result<i64> {
        Ok(self.read()?.size())
    }

    #[napi(getter)]
    pub fn capacity(&self) -> Result<u32> {
        Ok(self.read()?.capacity() as u32)
    }

    /// The backing `Uint32Array`. A **copy**; the original test reads
    /// `vector.array.length`, so it has to exist.
    #[napi(getter)]
    pub fn array(&self) -> Result<Uint32Array> {
        Ok(Uint32Array::new(self.read()?.words().to_vec()))
    }

    #[napi]
    pub fn set<'a>(&self, this: This<'a>, index: i64, value: Option<Unknown>) -> Result<This<'a>> {
        let outcome = self.write()?.set(index, !clears(value)?);

        outcome.map_err(|error| self.raise(error))?;

        Ok(this)
    }

    #[napi]
    pub fn reset<'a>(&self, this: This<'a>, index: i64) -> Result<This<'a>> {
        self.write()?.reset(index);

        Ok(this)
    }

    #[napi]
    pub fn flip<'a>(&self, this: This<'a>, index: i64) -> Result<This<'a>> {
        self.write()?.flip(index);

        Ok(this)
    }

    /// `undefined` strictly past `length`; `index == length` reads the capacity
    /// region. `Either<_, Undefined>` rather than `Option` — DIV-FIXED-STACK-1.
    #[napi]
    pub fn get(&self, index: i64) -> Result<Either<u32, Undefined>> {
        Ok(match self.read()?.get(index) {
            Some(bit) => Either::A(bit),
            None => Either::B(()),
        })
    }

    #[napi]
    pub fn test(&self, index: i64) -> Result<bool> {
        Ok(self.read()?.test(index))
    }

    #[napi]
    pub fn rank(&self, i: i64) -> Result<i64> {
        Ok(self.read()?.rank(i))
    }

    #[napi]
    pub fn select(&self, r: i64) -> Result<Either<i64, Undefined>> {
        Ok(match self.read()?.select(r) {
            Some(position) => Either::A(position),
            None => Either::B(()),
        })
    }

    #[napi]
    pub fn apply_policy(&self, override_capacity: Option<f64>) -> Result<u32> {
        // The borrow ends before `raise`, which borrows `thrown` and — for a
        // policy that re-enters — must not find the vector locked either.
        let outcome = self.read()?.apply_policy(override_capacity.map(count));

        outcome
            .map(|capacity| capacity as u32)
            .map_err(|error| self.raise(error))
    }

    #[napi]
    pub fn reallocate<'a>(&self, this: This<'a>, capacity: f64) -> Result<This<'a>> {
        self.write()?.reallocate(count(capacity));

        Ok(this)
    }

    #[napi]
    pub fn grow<'a>(&self, this: This<'a>, capacity: Option<f64>) -> Result<This<'a>> {
        let outcome = self.write()?.grow(capacity.map(count));

        outcome.map_err(|error| self.raise(error))?;

        Ok(this)
    }

    #[napi]
    pub fn resize<'a>(&self, this: This<'a>, length: f64) -> Result<This<'a>> {
        self.write()?.resize(count(length));

        Ok(this)
    }

    /// Returns the new length. Does **not** clear the slot for a falsy value;
    /// see the core docs.
    #[napi]
    pub fn push(&self, value: Option<Unknown>) -> Result<u32> {
        let value = !clears(value)?;
        let outcome = self.write()?.push(value);

        outcome
            .map(|length| length as u32)
            .map_err(|error| self.raise(error))
    }

    #[napi]
    pub fn pop(&self) -> Result<Either<u32, Undefined>> {
        Ok(match self.write()?.pop() {
            Some(bit) => Either::A(bit),
            None => Either::B(()),
        })
    }

    /// `forEach(callback, scope)`; same `scope` caveat as the other bridges.
    #[napi]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(u32, u32)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let (word_count, length) = {
            let inner = self.read()?;

            (inner.words().word_count(), inner.length())
        };

        for index in 0..word_count {
            // Re-borrowed and dropped per word, before any callback runs:
            // upstream's `byte = this.array[i]` is a fresh read of the live
            // array each time round the outer loop, and a callback that
            // `set`s or `push`es must not meet an outstanding borrow.
            let word = self.read()?.words().word(index).unwrap_or(0);
            let bits = mnemonist_core::structures::bits::bits_in_word(index, word_count, length);

            for bit in 0..bits {
                let value = (((word as i32) >> bit) & 1) as u32;
                let position = (index * 32 + bit) as u32;

                match &scope {
                    Some(scope) => callback.apply(*scope, (value, position).into())?,
                    None => callback.apply(this, (value, position).into())?,
                };
            }
        }

        Ok(())
    }

    #[napi]
    pub fn values(&self) -> Result<JsBitVectorValues> {
        Ok(JsBitVectorValues {
            cursor: BridgeBitCursor::new(self.read()?.values()),
        })
    }

    #[napi]
    pub fn entries(&self) -> Result<JsBitVectorEntries> {
        Ok(JsBitVectorEntries {
            cursor: BridgeBitCursor::new(self.read()?.values()),
        })
    }

    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Result<Vec<u32>> {
        Ok(self.read()?.to_json())
    }

    /// A shared borrow, or the re-entrancy error.
    ///
    /// Never `borrow()`: see the module docs for why a panic here would take
    /// the process down rather than reach JavaScript.
    fn read(&self) -> Result<std::cell::Ref<'_, CoreVector>> {
        self.inner
            .try_borrow()
            .map_err(|_| Error::new(Status::GenericFailure, REENTRANT_POLICY))
    }

    /// A mutable borrow, or the re-entrancy error.
    fn write(&self) -> Result<std::cell::RefMut<'_, CoreVector>> {
        self.inner
            .try_borrow_mut()
            .map_err(|_| Error::new(Status::GenericFailure, REENTRANT_POLICY))
    }

    /// Prefer an exception thrown *by the JS policy* over the core's
    /// classification of its result.
    ///
    /// Upstream would propagate the policy's own error straight out of
    /// `applyPolicy`; the core can only report "not a number", because a
    /// `Box<dyn Fn>` returning `Option` has nowhere to put an exception.
    fn raise(&self, error: CoreError) -> Error {
        if let Some(thrown) = self.thrown.borrow_mut().take() {
            return thrown;
        }

        Error::new(Status::GenericFailure, error.to_string())
    }
}

/// A JS growth policy, callable from the core's `Box<dyn Fn>`.
struct JsPolicy {
    callable: FunctionRef<f64, Unknown<'static>>,
    env: Env,
    thrown: Rc<RefCell<Option<Error>>>,
}

impl JsPolicy {
    /// `None` for "not a number" *and* for "threw" — the latter with the
    /// exception parked in `thrown` for [`JsBitVector::raise`] to re-raise.
    fn call(&self, capacity: f64) -> Option<f64> {
        let result = self
            .callable
            .borrow_back(&self.env)
            .and_then(|callable| callable.call(capacity));

        match result {
            Err(error) => {
                *self.thrown.borrow_mut() = Some(error);
                None
            }
            // `typeof newCapacity !== 'number'` -- a policy returning a string
            // or an object lands here, and the core turns it into upstream's
            // "invalid value" throw.
            Ok(value) => match value.get_type() {
                Ok(ValueType::Number) => f64::from_unknown(value).ok(),
                _ => None,
            },
        }
    }
}

/// `BitVector.prototype.values()`.
#[napi(iterator, js_name = "BitVectorValues")]
pub struct JsBitVectorValues {
    cursor: BridgeBitCursor,
}

impl Generator for JsBitVectorValues {
    type Yield = u32;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<u32> {
        self.cursor.next_bit()
    }
}

/// `BitVector.prototype.entries()`, yielding `[index, bit]`.
#[napi(iterator, js_name = "BitVectorEntries")]
pub struct JsBitVectorEntries {
    cursor: BridgeBitCursor,
}

impl Generator for JsBitVectorEntries {
    type Yield = Vec<u32>;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Vec<u32>> {
        self.cursor.next_entry()
    }
}

/// A JS number used as a length or capacity. Same treatment as the
/// `HashedArrayTree` bridge: truncate, and clamp what a `usize` cannot hold.
fn count(value: f64) -> usize {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }

    value.trunc() as usize
}

/// Re-exported so the default policy is reachable from JS-side tooling without
/// reaching into the core crate.
#[napi(js_name = "bitVectorDefaultPolicy")]
pub fn bit_vector_default_policy(capacity: f64) -> Either<f64, Undefined> {
    match default_policy(capacity) {
        Some(value) => Either::A(value),
        None => Either::B(()),
    }
}

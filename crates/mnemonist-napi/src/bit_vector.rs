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

use std::cell::RefCell;
use std::rc::Rc;

use mnemonist_core::structures::bit_vector::{
    default_policy, BitVector as CoreVector, Error as CoreError,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::bit_set::clears;
use crate::cursor::BridgeBitCursor;

/// A growable bit vector over a `Uint32Array`.
#[napi(js_name = "BitVector")]
pub struct JsBitVector {
    inner: CoreVector,
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

        Ok(Self { inner, thrown })
    }

    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.length() as u32
    }

    /// Upstream's `size` counter. Signed; see B-13 and the `push`/`pop` defects.
    #[napi(getter)]
    pub fn size(&self) -> i64 {
        self.inner.size()
    }

    #[napi(getter)]
    pub fn capacity(&self) -> u32 {
        self.inner.capacity() as u32
    }

    /// The backing `Uint32Array`. A **copy**; the original test reads
    /// `vector.array.length`, so it has to exist.
    #[napi(getter)]
    pub fn array(&self) -> Uint32Array {
        Uint32Array::new(self.inner.words().to_vec())
    }

    #[napi]
    pub fn set<'a>(
        &mut self,
        this: This<'a>,
        index: i64,
        value: Option<Unknown>,
    ) -> Result<This<'a>> {
        self.inner
            .set(index, !clears(value)?)
            .map_err(|error| self.raise(error))?;

        Ok(this)
    }

    #[napi]
    pub fn reset<'a>(&mut self, this: This<'a>, index: i64) -> This<'a> {
        self.inner.reset(index);

        this
    }

    #[napi]
    pub fn flip<'a>(&mut self, this: This<'a>, index: i64) -> This<'a> {
        self.inner.flip(index);

        this
    }

    /// `undefined` strictly past `length`; `index == length` reads the capacity
    /// region. `Either<_, Undefined>` rather than `Option` — D-39.
    #[napi]
    pub fn get(&self, index: i64) -> Either<u32, Undefined> {
        match self.inner.get(index) {
            Some(bit) => Either::A(bit),
            None => Either::B(()),
        }
    }

    #[napi]
    pub fn test(&self, index: i64) -> bool {
        self.inner.test(index)
    }

    #[napi]
    pub fn rank(&self, i: i64) -> i64 {
        self.inner.rank(i)
    }

    #[napi]
    pub fn select(&self, r: i64) -> Either<i64, Undefined> {
        match self.inner.select(r) {
            Some(position) => Either::A(position),
            None => Either::B(()),
        }
    }

    #[napi]
    pub fn apply_policy(&self, override_capacity: Option<f64>) -> Result<u32> {
        self.inner
            .apply_policy(override_capacity.map(count))
            .map(|capacity| capacity as u32)
            .map_err(|error| self.raise(error))
    }

    #[napi]
    pub fn reallocate<'a>(&mut self, this: This<'a>, capacity: f64) -> This<'a> {
        self.inner.reallocate(count(capacity));

        this
    }

    #[napi]
    pub fn grow<'a>(&mut self, this: This<'a>, capacity: Option<f64>) -> Result<This<'a>> {
        self.inner
            .grow(capacity.map(count))
            .map_err(|error| self.raise(error))?;

        Ok(this)
    }

    #[napi]
    pub fn resize<'a>(&mut self, this: This<'a>, length: f64) -> This<'a> {
        self.inner.resize(count(length));

        this
    }

    /// Returns the new length. Does **not** clear the slot for a falsy value;
    /// see the core docs.
    #[napi]
    pub fn push(&mut self, value: Option<Unknown>) -> Result<u32> {
        let value = !clears(value)?;

        self.inner
            .push(value)
            .map(|length| length as u32)
            .map_err(|error| self.raise(error))
    }

    #[napi]
    pub fn pop(&mut self) -> Either<u32, Undefined> {
        match self.inner.pop() {
            Some(bit) => Either::A(bit),
            None => Either::B(()),
        }
    }

    /// `forEach(callback, scope)`; same `scope` caveat as the other bridges.
    #[napi]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(u32, u32)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let word_count = self.inner.words().word_count();
        let length = self.inner.length();

        for index in 0..word_count {
            let word = self.inner.words().word(index).unwrap_or(0);
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
    pub fn values(&self) -> JsBitVectorValues {
        JsBitVectorValues {
            cursor: BridgeBitCursor::new(self.inner.values()),
        }
    }

    #[napi]
    pub fn entries(&self) -> JsBitVectorEntries {
        JsBitVectorEntries {
            cursor: BridgeBitCursor::new(self.inner.values()),
        }
    }

    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Vec<u32> {
        self.inner.to_json()
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

//! The `ArrayClass` constructor argument, at the boundary.
//!
//! `FixedStack`, `FixedDeque` and `CircularBuffer` are all
//! `new X(ArrayClass, capacity)`, and the class is a **JavaScript value** — a
//! constructor function the caller supplies. The original test files use
//! `Array`, `Uint8Array` and `Float64Array`; nothing stops a caller from
//! passing `Int16Array`, `BigInt64Array` or a class of their own.
//!
//! `mnemonist-core` reduces all of that to two bits
//! ([`Backing`](mnemonist_core::structures::backing::Backing)): what an
//! unwritten slot reads as, and what a store past the end does. This module is
//! how those two bits — and the *element coercion*, which core deliberately
//! does not model — are obtained from a real JS constructor.
//!
//! # Coercion by round trip, rather than by a table of classes
//!
//! A `Uint8Array` stores `300` as `44`, an `Int8Array` stores it as `44` too
//! but `200` as `-56`, a `Float64Array` stores `1.5` unchanged, and an `Array`
//! stores whatever it is given. The `hashed-array-tree` bridge handles the same
//! problem with a name whitelist and refuses everything outside it; that costs
//! a divergence for nine of the twelve built-in typed arrays and for anything
//! user-defined.
//!
//! Here the class does the conversion itself. [`ArrayClass`] keeps a
//! **one-element instance** of the class and coerces by
//!
//! ```js
//! scratch[0] = value;
//! return scratch[0];
//! ```
//!
//! which is, definitionally, what `this.items[i] = item` would have done. It
//! is exact for every array class that exists, including ones that did not
//! exist when this was written, and it needs no table.
//!
//! # The two bits, probed rather than assumed
//!
//! `0 in new ArrayClass(1)` distinguishes the two backings: a `new Array(1)`
//! has no own `0` (it is a hole), while every typed array is zero-filled and
//! does. That single read decides both bits, because **every** array class
//! pairs them the same way — hole-filled classes grow on an out-of-range store
//! and zero-filled ones drop it. Probing the growth bit directly would mean
//! *writing* to a caller-supplied object during construction, which is a
//! visible side effect for a class with setters; the read is not.
//!
//! # What this costs, stated
//!
//! Two extra constructions of the caller's class per structure (the scratch and
//! the probe, both of length one) that upstream does not perform. Invisible for
//! `Array` and the typed arrays; observable for a constructor with side
//! effects. Recorded as D-63.

use std::ptr;

use mnemonist_core::structures::backing::Backing;
use napi::bindgen_prelude::*;
use napi::sys;

use crate::foreach::{check, coerce_to_object};
use crate::js_slot::JsSlot;

/// A JS array constructor, plus everything derived from it once.
pub struct ArrayClass {
    /// `this.ArrayClass`. Held as a slot because a `napi_value` does not
    /// survive the call that produced it.
    constructor: JsSlot,
    /// A one-element instance, used to run element stores through the class.
    scratch: JsSlot,
    /// The two bits `mnemonist-core` needs.
    backing: Backing<JsSlot>,
}

impl ArrayClass {
    /// Identify a caller-supplied constructor.
    ///
    /// Errors surface whatever the class's own constructor threw — a
    /// non-constructor value dies here with V8's `TypeError`, which is roughly
    /// where upstream's `new this.ArrayClass(this.capacity)` would have died.
    pub fn probe(env: &Env, constructor: &Unknown) -> Result<Self> {
        let scratch = instantiate(env, constructor, 1)?;
        let probe = instantiate(env, constructor, 1)?;
        let probe = coerce_to_object(env, &probe)?;

        // `0 in probe`: present for a zero-filled typed array, absent for the
        // hole a `new Array(1)` leaves behind.
        let backing = if probe.has_own_property("0")? {
            let zero: Unknown = probe.get_element(0)?;

            Backing::Filled(JsSlot::new(env, &zero)?)
        } else {
            Backing::Holes
        };

        Ok(Self {
            constructor: JsSlot::new(env, constructor)?,
            scratch: JsSlot::new(env, &scratch)?,
            backing,
        })
    }

    /// The two bits, for [`mnemonist_core`].
    pub fn backing(&self) -> Backing<JsSlot> {
        self.backing.clone()
    }

    /// `items[i] = value`, run through the class and read back.
    ///
    /// The identity for a plain `Array`; the narrowing store for a typed one.
    pub fn coerce(&self, env: &Env, value: &Unknown) -> Result<JsSlot> {
        let mut scratch = coerce_to_object(env, &self.scratch.get(env)?)?;

        scratch.set_element(0, *value)?;

        let stored: Unknown = scratch.get_element(0)?;

        JsSlot::new(env, &stored)
    }

    /// `new ArrayClass(length)`, filled from `slots`.
    ///
    /// A `None` slot is left **unwritten**, which is exactly upstream's
    /// `array[i] = undefined`: a hole in a fresh `Array`, and the class zero in
    /// a fresh typed array, without this code having to know which it is
    /// holding.
    pub fn materialise<'env>(
        &self,
        env: &'env Env,
        slots: &[Option<JsSlot>],
    ) -> Result<Unknown<'env>> {
        let constructor = self.constructor.get(env)?;
        let instance = instantiate(env, &constructor, slots.len())?;
        let mut object = coerce_to_object(env, &instance)?;

        for (index, slot) in slots.iter().enumerate() {
            let Some(slot) = slot else { continue };

            object.set_element(index as u32, slot.get(env)?)?;
        }

        // SAFETY: `object` is the instance constructed above, in this scope.
        Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), object.raw()) })
    }
}

/// `new constructor(length)`.
fn instantiate<'env>(
    env: &'env Env,
    constructor: &Unknown,
    length: usize,
) -> Result<Unknown<'env>> {
    let argument = env.create_double(length as f64)?;
    let arguments = [argument.raw()];
    let mut instance = ptr::null_mut();

    // SAFETY: `arguments` outlives the call and holds one live handle from
    // `env`; `constructor` is a handle from the same environment. A
    // non-constructor value is refused by N-API, not UB.
    let status = unsafe {
        sys::napi_new_instance(
            env.raw(),
            constructor.raw(),
            1,
            arguments.as_ptr(),
            &mut instance,
        )
    };

    // Two different failures, and they must not be merged. A constructor that
    // *threw* leaves a pending JS exception, and `check` propagates it
    // untouched — `new Uint8Array(1e30)` must still surface as its own
    // `RangeError`. A value that is not a constructor at all is refused by
    // N-API with no exception pending, and upstream's `new this.ArrayClass(…)`
    // reports that as V8 words it, naming the property:
    //
    //     TypeError: this.ArrayClass is not a constructor
    if status != sys::Status::napi_ok && status != sys::Status::napi_pending_exception {
        return Err(crate::foreach::type_error(
            env,
            "this.ArrayClass is not a constructor",
        ));
    }

    check(status, "napi_new_instance")?;

    // SAFETY: `instance` is the object N-API just constructed.
    Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), instance) })
}

/// A JS number used as a fixed structure's `capacity`.
///
/// Three outcomes, and the order between them is upstream's:
///
/// 1. an **omitted** argument — `undefined`, since napi does not check arity —
///    is `arguments.length < 2`, so `missing` is raised;
/// 2. a non-number, or a number `<= 0`, is upstream's second guard, so `bad`;
/// 3. anything else has to survive `new ArrayClass(capacity)`.
///
/// (3) is where this diverges, and it is stated rather than hidden. Upstream
/// passes the raw number to the class and lets it decide: `new Array(2.5)`
/// throws `RangeError: Invalid array length`, while `new Uint8Array(2.5)`
/// **truncates**, leaving `this.capacity === 2.5` against an `items.length` of
/// 2 — after which the deque's wrap arithmetic compares indices against 2.5.
/// The port requires an integral capacity and raises the `Array` form of the
/// error for every class. See D-62.
pub fn capacity_of(
    env: &Env,
    value: &Unknown,
    missing: &'static str,
    bad: &'static str,
) -> Result<usize> {
    if value.get_type()? == ValueType::Undefined {
        return Err(Error::new(Status::InvalidArg, missing));
    }

    if value.get_type()? != ValueType::Number {
        return Err(Error::new(Status::InvalidArg, bad));
    }

    let capacity = crate::foreach::to_number(env, value)?;

    // `capacity <= 0`. NaN fails this comparison in both languages and falls
    // through to the length check below, exactly as it does upstream.
    if capacity <= 0.0 {
        return Err(Error::new(Status::InvalidArg, bad));
    }

    if !capacity.is_finite() || capacity.fract() != 0.0 || capacity >= 4_294_967_296.0 {
        return Err(crate::foreach::range_error(env, "Invalid array length"));
    }

    Ok(capacity as usize)
}

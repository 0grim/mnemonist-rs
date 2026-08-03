//! `mnemonist/utils/iterables.js` (93 LOC), ported to the boundary.
//!
//! Four functions — `isArrayLike`, `guessLength`, `toArray`,
//! `toArrayWithIndices` — and every one of them is a *JavaScript-value*
//! question. `isArrayLike` asks `Array.isArray || ArrayBuffer.isView`;
//! `guessLength` reads two properties and checks their `typeof`; `toArray`
//! preallocates a JS array and drives `obliterator/foreach`. None of that has a
//! Rust meaning, so by DESIGN.md §3.5 it lives here rather than in
//! `mnemonist-core`, exactly as `forEach` does — and it is built **on**
//! [`crate::foreach`], not on a second copy of the dispatch.
//!
//! # `guessLength` trusts, and `toArray` preallocates on the trust (NOTES BUG-UTILS-ITERABLES-1)
//!
//! ```js
//! function guessLength(target) {
//!   if (typeof target.length === 'number') return target.length;
//!   if (typeof target.size === 'number') return target.size;
//!   return;
//! }
//!
//! function toArray(target) {
//!   var l = guessLength(target);
//!   var array = typeof l === 'number' ? new Array(l) : [];
//!   var i = 0;
//!   forEach(target, function (value) { array[i++] = value; });
//!   return array;
//! }
//! ```
//!
//! Nothing checks the guess against what `forEach` actually yields, and nothing
//! checks that it is a *valid array length*. Three consequences, all measured
//! on Node 24.18.1:
//!
//! * **An overstated length leaves holes.** A target claiming `length: 5` that
//!   yields two values gives `[1, 2, <3 empty items>]` — `length === 5`, and
//!   `2 in array === false`. A hole is not `undefined`: it is distinguishable
//!   by `in`, by `hasOwnProperty`, and by `Array.prototype.map`, which skips it.
//! * **An understated length is silently exceeded**, because `array[i++] = v`
//!   grows the array. `{length: 1}` yielding three values gives `[1, 2, 3]`.
//! * **An invalid length throws from the allocation**, not from a guard:
//!   `toArray({length: -1, …})` and `toArray({length: 3.5, …})` both die with
//!   `RangeError: Invalid array length`.
//!
//! This is reproduced rather than fixed (DIV-FIXED-STACK-2, resolving DIV-PROJ-18): the array
//! really is allocated by calling the running realm's `Array` constructor, so
//! the `RangeError` is V8's own and the holes are real holes.
//!
//! The sharpest form of BUG-UTILS-ITERABLES-1, and the reason it is filed as a bug rather than a
//! quirk, is that `toArray({length: 5})` reaches `forEach`'s **plain-object**
//! branch, which enumerates own properties — *including `length` itself* — so
//! the array's first element is the number 5.
//!
//! # `toArrayWithIndices` picks its index width from the same untrusted number
//!
//! `getPointerArray(l)` is `mnemonist-core`'s
//! [`get_pointer_array`](mnemonist_core::utils::typed_arrays::get_pointer_array),
//! the one part of this file that *is* pure computation. It runs **before** the
//! `new Array(l)`, so for a hostile `l` its throw wins over the `RangeError`;
//! that ordering is upstream's and is preserved.

use std::cell::Cell;
use std::ptr;
use std::rc::Rc;

use mnemonist_core::utils::typed_arrays::{
    get_pointer_array, PointerWidth, POINTER_ARRAY_TOO_LARGE,
};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::foreach::{self, check, coerce_to_object, is_array, is_array_buffer_view, type_error};
use crate::js_slot::JsSlot;

/// `Array.isArray(target) || typed.isTypedArray(target)`.
///
/// `isTypedArray` is `ArrayBuffer.isView`, so a `DataView` counts and a string
/// does not — which is why `FixedStack.from('abc', Array)` takes the *other*
/// branch of `from` and dies in BUG-UTILS-ITERABLES-2, while `FixedStack.from([1,2,3], Array)`
/// works.
#[napi(js_name = "isArrayLike")]
pub fn js_is_array_like(env: Env, target: Unknown) -> Result<bool> {
    is_array_like(&env, &target)
}

pub fn is_array_like(env: &Env, target: &Unknown) -> Result<bool> {
    Ok(is_array(env, target)? || is_array_buffer_view(env, target)?)
}

/// `guessLength(target)` — `.length`, then `.size`, then `undefined`.
///
/// `Either<f64, Undefined>` rather than `Option<f64>`: napi renders `None` as
/// `null` and upstream returns a bare `undefined` (DIV-FIXED-STACK-1).
#[napi(js_name = "guessLength")]
pub fn js_guess_length(env: Env, target: Unknown) -> Result<Either<f64, Undefined>> {
    Ok(match guess_length(&env, &target)? {
        Some(length) => Either::A(length),
        None => Either::B(()),
    })
}

/// The Rust-callable form, used by the three `from` statics.
///
/// Upstream reads `target.length` with no guard at all, so `null` and
/// `undefined` throw from the property access rather than from the function.
/// That is V8's `TypeError`, with V8's wording, and it is what a caller of
/// `FixedStack.from(null)` sees — so it is raised here rather than replaced
/// with a tidier message.
pub fn guess_length(env: &Env, target: &Unknown) -> Result<Option<f64>> {
    let value_type = target.get_type()?;

    if matches!(value_type, ValueType::Null | ValueType::Undefined) {
        let what = if value_type == ValueType::Null {
            "null"
        } else {
            "undefined"
        };

        return Err(type_error(
            env,
            &format!("Cannot read properties of {what} (reading 'length')"),
        ));
    }

    // `x.length` on a primitive boxes it, which N-API does not do implicitly.
    let object = coerce_to_object(env, target)?;

    for name in ["length", "size"] {
        let candidate: Unknown = object.get_named_property_unchecked(name)?;

        if candidate.get_type()? == ValueType::Number {
            return Ok(Some(foreach::to_number(env, &candidate)?));
        }
    }

    Ok(None)
}

/// `toArray(target)` — preallocate on the guess, then fill.
#[napi(js_name = "toArray")]
pub fn js_to_array<'env>(env: &'env Env, target: Unknown<'env>) -> Result<Unknown<'env>> {
    let length = guess_length(env, &target)?;
    // `new Array(l)` when the guess is a number, `[]` otherwise. Really
    // allocated through the realm's own constructor, so an invalid length
    // throws V8's `RangeError` rather than being clamped here.
    let array = allocate(env, "Array", length)?;

    fill(env, &array, target, None)?;

    Ok(array)
}

/// `toArrayWithIndices(target)` — the same, plus a parallel index array whose
/// element width is chosen by `getPointerArray`.
#[napi(js_name = "toArrayWithIndices")]
pub fn js_to_array_with_indices<'env>(
    env: &'env Env,
    target: Unknown<'env>,
) -> Result<Unknown<'env>> {
    let length = guess_length(env, &target)?;
    // Upstream computes `IndexArray` FIRST, so on a hostile guess the
    // `getPointerArray` throw wins over `new Array(l)`'s `RangeError`.
    let index_class = match length {
        None => "Array",
        Some(length) => match get_pointer_array(length) {
            Ok(PointerWidth::U8) => "Uint8Array",
            Ok(PointerWidth::U16) => "Uint16Array",
            Ok(PointerWidth::U32) => "Uint32Array",
            Err(_) => {
                return Err(Error::new(
                    Status::GenericFailure,
                    POINTER_ARRAY_TOO_LARGE.to_owned(),
                ))
            }
        },
    };

    let array = allocate(env, "Array", length)?;
    let indices = allocate(env, index_class, length)?;

    fill(env, &array, target, Some(&indices))?;

    // A real JS array, not a plain object with "0"/"1" — upstream returns
    // `[array, indices]` and callers destructure it.
    let mut pair = ptr::null_mut();

    // SAFETY: a live `env`; the out-pointer is written before it is read.
    check(
        unsafe { sys::napi_create_array_with_length(env.raw(), 2, &mut pair) },
        "napi_create_array_with_length",
    )?;

    // SAFETY: `pair` is the array N-API just produced.
    let mut pair = unsafe { Object::from_napi_value(env.raw(), pair)? };

    pair.set_element(0, array)?;
    pair.set_element(1, indices)?;

    // SAFETY: a handle from this environment and scope.
    Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), pair.raw()) })
}

/// `new Class(length)`, or `new Class()` when the guess was not a number.
///
/// Constructed by calling the realm's own constructor rather than by
/// `napi_create_array_with_length`, because the two differ exactly where this
/// module is interesting: `new Array(-1)` throws `RangeError: Invalid array
/// length` and the N-API call does not.
fn allocate<'env>(env: &'env Env, class: &str, length: Option<f64>) -> Result<Unknown<'env>> {
    let global = env.get_global()?;
    let constructor: Unknown = global.get_named_property_unchecked(class)?;
    let mut instance = ptr::null_mut();

    match length {
        None => {
            // SAFETY: `constructor` is a live handle from `env`; a zero-argument
            // construction needs no argument vector.
            check(
                unsafe {
                    sys::napi_new_instance(
                        env.raw(),
                        constructor.raw(),
                        0,
                        ptr::null(),
                        &mut instance,
                    )
                },
                "napi_new_instance",
            )?;
        }
        Some(length) => {
            let argument = env.create_double(length)?;
            let arguments = [argument.raw()];

            // SAFETY: `arguments` outlives the call and holds exactly one live
            // handle from `env`.
            check(
                unsafe {
                    sys::napi_new_instance(
                        env.raw(),
                        constructor.raw(),
                        1,
                        arguments.as_ptr(),
                        &mut instance,
                    )
                },
                "napi_new_instance",
            )?;
        }
    }

    // SAFETY: `instance` is the object N-API just constructed.
    Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), instance) })
}

/// `forEach(target, function (value) { array[i++] = value; })`, with the
/// optional index array of `toArrayWithIndices` written alongside.
///
/// Upstream's second body is `array[i] = value; indices[i] = i++;` — the
/// member expression is evaluated before the postfix increment, so both stores
/// land at the same index and `indices` ends up as the identity. Written out
/// here because reading it as `indices[i + 1] = i` is the obvious misreading.
fn fill(env: &Env, array: &Unknown, target: Unknown, indices: Option<&Unknown>) -> Result<()> {
    // The two sinks travel as `JsSlot`s rather than as `Object<'env>`s because
    // napi requires the closure to be `'static` and a handle is scope-bound; a
    // slot is a real `napi_ref`, which is exactly the "survives between calls"
    // property the closure needs (see `crate::js_slot`).
    let sink = JsSlot::new(env, array)?;
    let index_sink = indices
        .map(|indices| JsSlot::new(env, indices))
        .transpose()?;
    let position = Rc::new(Cell::new(0u32));
    let cursor = Rc::clone(&position);

    // A JS function, so `forEach`'s branch 2 hands a host `forEach` exactly the
    // kind of callback it expects. Same reasoning as `foreach::collect`: the
    // dispatch is run, never simulated.
    let collector: Function<'_, Unknown, ()> =
        env.create_function_from_closure("toArray", move |context| {
            let value: Unknown = match context.length() {
                0 => foreach::undefined(context.env)?,
                _ => context.get(0)?,
            };
            let index = cursor.get();
            let mut sink = coerce_to_object(context.env, &sink.get(context.env)?)?;

            sink.set_element(index, value)?;

            if let Some(index_sink) = index_sink.as_ref() {
                let mut index_sink = coerce_to_object(context.env, &index_sink.get(context.env)?)?;

                index_sink.set_element(index, context.env.create_uint32(index)?)?;
            }

            cursor.set(index + 1);

            Ok(())
        })?;

    // SAFETY: `collector` is a JS function this call just created.
    let collector = unsafe { Unknown::from_raw_unchecked(env.raw(), collector.raw()) };

    foreach::for_each(env, target, collector)
}

/// Every value an array-like target holds, in index order.
///
/// This is the loop the three `from` statics run when `isArrayLike` says yes:
///
/// ```js
/// for (i = 0, l = iterable.length; i < l; i++)
///   structure.items[i] = iterable[i];
/// ```
///
/// `length` is read **once** and compared numerically, exactly as
/// [`crate::foreach`]'s branch 1 does — note that it is `.length` here, not
/// `guessLength`, so a typed array's `.length` is used and `.size` is never
/// consulted.
pub fn array_like_values(env: &Env, target: &Unknown) -> Result<Vec<JsSlot>> {
    let object = coerce_to_object(env, target)?;
    let length: Unknown = object.get_named_property_unchecked("length")?;
    let length = foreach::to_number(env, &length)?;
    let mut values = Vec::new();
    let mut index = 0u32;

    while f64::from(index) < length {
        let value: Unknown = object.get_element(index)?;

        values.push(JsSlot::new(env, &value)?);

        index = match index.checked_add(1) {
            Some(next) => next,
            None => break,
        };
    }

    Ok(values)
}

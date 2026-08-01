//! `obliterator/foreach`, ported to the boundary (DESIGN.md §3.4, §3.5, D-03).
//!
//! Thirty of the forty-four upstream modules import this one function, and a
//! grep of every call site shows all of them are `forEach(iterable, cb)` inside
//! a `.from()` static or an iterable-accepting constructor, operating on the
//! **user-supplied argument**. Not one iterates a structure's own data. So this
//! is a JavaScript-value coercion, it belongs in the crate that has JavaScript
//! values in it, and `mnemonist-core` never hears about it — core structures
//! take an `IntoIterator` and a Rust caller never meets any of the below.
//!
//! # The dispatch, and why the order is observable
//!
//! | # | test | callback's 2nd argument |
//! |---|---|---|
//! | 1 | `Array.isArray` ∥ `ArrayBuffer.isView` ∥ `typeof === 'string'` ∥ `toString() === '[object Arguments]'` | the index, a **number** |
//! | 2 | `typeof iterable.forEach === 'function'` → **delegate** | whatever the host passes |
//! | 3 | `Symbol.iterator in iterable && typeof iterable.next !== 'function'` → coerce | — |
//! | 4 | `typeof iterable.next === 'function'` → drain | an own counter, a **number** |
//! | 5 | plain object → `for…in` + `hasOwnProperty` | the key, a **string** |
//!
//! Three traps, all of them load-bearing (D-10, D-11, D-12):
//!
//! * **Branch 2 preempts 3 and 4.** A JS `Map` owns a `.forEach`, so it never
//!   reaches the iterator path — and a host `forEach` passes `(value, key)`.
//!   The second callback argument therefore changes *type* by branch: number,
//!   string, or host-defined.
//! * **The falsy guard is `if (!iterable) throw`**, not a null check. So
//!   `forEach('', cb)` throws while `forEach('a', cb)` iterates; likewise `0`,
//!   `false`, `NaN` and `0n`.
//! * **`toString()` is invoked on an arbitrary user value during type
//!   dispatch** (NOTES B-5). It can throw, and it can return
//!   `'[object Arguments]'` and hijack branch 1. Both are reproduced.
//!
//! And one behaviour that is not in the table because it is not in the comments
//! either — see [`for_each`] and NOTES B-30: a *truthy primitive* reaches the
//! `in` operator in branch 3, and `in` requires an object, so `forEach(5, cb)`
//! dies with a `TypeError` from V8 rather than with obliterator's own guard.
//!
//! # One implementation, two callers
//!
//! [`for_each`] drives a **JavaScript** callback, because branch 2 hands the
//! callback to a host `forEach` and there is no way to make that faithful with
//! a Rust closure. [`collect`], which is what every `.from()` bridge actually
//! calls, therefore does not reimplement anything: it builds a JS collector
//! function and runs the same dispatch. Splitting them would mean two copies of
//! the five branches and one of them going stale.

use std::cell::RefCell;
use std::ptr;
use std::rc::Rc;

use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::js_slot::JsSlot;

/// Verbatim, and asserted verbatim by `tests/boundary/foreach.js` (D-14).
const INVALID_ITERABLE: &str = "obliterator/forEach: invalid iterable.";
const EXPECTING_CALLBACK: &str = "obliterator/forEach: expecting a callback.";

/// The tag `Object.prototype.toString` produces for an `arguments` object.
const ARGUMENTS_TAG: &str = "[object Arguments]";

/// `obliterator/foreach`, exported so it can be tested as the standalone
/// function it is upstream.
///
/// The bridges do not go through this — they call [`collect`] — but the two
/// share every line of dispatch below, so testing this tests them.
#[napi(js_name = "forEach")]
pub fn js_for_each(env: Env, iterable: Unknown, callback: Unknown) -> Result<()> {
    for_each(&env, iterable, callback)
}

/// Visit every value the upstream function would visit, in the same order,
/// with the same second argument.
///
/// `callback` is taken as an [`Unknown`] rather than a typed [`Function`] on
/// purpose: upstream checks `typeof callback !== 'function'` itself and throws
/// its own message, and letting napi reject the argument first would replace
/// that message with napi's.
pub fn for_each<'env>(env: &Env, iterable: Unknown<'env>, callback: Unknown<'env>) -> Result<()> {
    // `if (!iterable) throw` — JS truthiness, not a null check (D-12).
    if !is_truthy(env, &iterable)? {
        return Err(Error::new(Status::GenericFailure, INVALID_ITERABLE));
    }

    if callback.get_type()? != ValueType::Function {
        return Err(Error::new(Status::GenericFailure, EXPECTING_CALLBACK));
    }

    let value_type = iterable.get_type()?;

    // ---- branch 1: indexed sequences -------------------------------------
    //
    // The four tests are `||`-chained upstream, so `toString()` runs only when
    // the first three have failed. That ordering is what keeps a hostile
    // `toString` off the fast paths, and it is reproduced rather than tidied.
    if value_type == ValueType::String {
        return each_string(env, &iterable, &callback);
    }

    if is_array(env, &iterable)? || is_array_buffer_view(env, &iterable)? {
        return each_indexed(env, &iterable, &callback);
    }

    // `iterable.toString()`. NOTES B-5: an arbitrary user value is being asked
    // for a string in the middle of type dispatch.
    if has_arguments_tag(env, &iterable)? {
        return each_indexed(env, &iterable, &callback);
    }

    // Everything from here reads properties. JS boxes primitives for that;
    // N-API does not, so the boxing is explicit. `napi_coerce_to_object` is the
    // identity on objects and functions, so this costs nothing in the common
    // case.
    let target = coerce_to_object(env, &iterable)?;

    // ---- branch 2: the host owns a #.forEach, so delegate to it ------------
    //
    // Before the iterator branches, deliberately. A `Map` never reaches
    // branch 3, and the callback it receives is invoked by the host with
    // `(value, key)` — the polymorphic second argument of D-11, at its most
    // visible.
    let host_for_each: Unknown = target.get_named_property_unchecked("forEach")?;

    if host_for_each.get_type()? == ValueType::Function {
        // SAFETY: the type check above is exactly `typeof === 'function'`.
        let host_for_each: Function<'_, Unknown, Unknown> = unsafe { host_for_each.cast()? };

        host_for_each.apply(target, callback)?;

        return Ok(());
    }

    // ---- branch 3: iterable, but not already an iterator -------------------
    //
    // `Symbol.iterator in iterable` — and `in` demands an object. A truthy
    // primitive (a number, a boolean, a symbol, a bigint) has already survived
    // the falsy guard and branch 1, so it arrives here and V8 throws. That is
    // upstream behaviour, not ours: obliterator has no guard for it, and the
    // error a caller sees names the `in` operator rather than the library.
    // NOTES B-30.
    if !matches!(value_type, ValueType::Object | ValueType::Function) {
        return Err(type_error(
            env,
            &format!(
                "Cannot use 'in' operator to search for 'Symbol(Symbol.iterator)' in {}",
                display(env, &iterable)?
            ),
        ));
    }

    let iterator_symbol = well_known_iterator(env)?;
    // `iterable` is *reassigned* upstream — `iterable = iterable[Symbol.iterator]()`
    // — so branches 4 and 5 operate on whatever this leaves behind, not on the
    // original argument. Only the boxed form is needed from here on.
    let mut drained_object = target;

    if target.has_property_js(iterator_symbol)? {
        let next: Unknown = drained_object.get_named_property_unchecked("next")?;

        if next.get_type()? != ValueType::Function {
            let factory: Unknown = drained_object.get_property(iterator_symbol)?;
            // Upstream does not check that this is callable; it just calls it,
            // and a non-callable value throws from the call site.
            let factory: Function<'_, (), Unknown> = unsafe { factory.cast()? };
            let drained = factory.apply(drained_object, ())?;

            drained_object = coerce_to_object(env, &drained)?;
        }
    }

    // ---- branch 4: an iterator, drained with its own counter ---------------
    let next: Unknown = drained_object.get_named_property_unchecked("next")?;

    if next.get_type()? == ValueType::Function {
        // SAFETY: `typeof === 'function'`, checked above.
        let next: Function<'_, (), Unknown> = unsafe { next.cast()? };
        // SAFETY: `callback` was type-checked at the top of this function.
        let callback: Function<'_, FnArgs<(Unknown, u32)>, Unknown> = unsafe { callback.cast()? };
        let mut index = 0u32;

        // `while (((s = iterator.next()), s.done !== true))` — note the strict
        // `!== true`, so a step reporting `done: 1` keeps going.
        loop {
            let step = next.apply(drained_object, ())?;
            let step = coerce_to_object(env, &step)?;
            let done: Unknown = step.get_named_property_unchecked("done")?;

            if is_strictly_true(env, &done)? {
                return Ok(());
            }

            let value: Unknown = step.get_named_property_unchecked("value")?;

            callback.call((value, index).into())?;
            index += 1;
        }
    }

    // ---- branch 5: a plain object, by key ----------------------------------
    //
    // `for (k in iterable) if (iterable.hasOwnProperty(k))`. Own, enumerable,
    // string-keyed — which is precisely what `napi_get_all_property_names`
    // returns under these filters, **in the engine's own order**: integer-like
    // keys ascending, then the rest in insertion order. Reimplementing that
    // order in Rust would be pure downside risk (D-15).
    let keys = drained_object.get_all_property_names(
        KeyCollectionMode::OwnOnly,
        KeyFilter::AllProperties,
        KeyConversion::NumbersToStrings,
    )?;
    let count = keys.get_array_length()?;
    // SAFETY: `callback` was type-checked at the top of this function.
    let callback: Function<'_, FnArgs<(Unknown, Unknown)>, Unknown> = unsafe { callback.cast()? };

    for index in 0..count {
        let key: Unknown = keys.get_element(index)?;

        // The enumerability filter is applied here rather than through
        // `KeyFilter::Enumerable`, whose N-API semantics differ across
        // versions; `napi_get_property` on a key we just enumerated cannot
        // fail, and a non-enumerable one is skipped exactly as `for…in` skips
        // it.
        if !is_enumerable(env, &drained_object, &key)? {
            continue;
        }

        let value: Unknown = drained_object.get_property(key)?;

        callback.call((value, key).into())?;
    }

    Ok(())
}

/// Everything [`for_each`] would visit, as values a Rust structure can hold.
///
/// This is the `.from()` shape, and the only thing the `Stack`/`Queue` bridges
/// use. It runs the dispatch above rather than a second copy of it: the
/// callback is a JS function built here, so branch 2 hands the host exactly the
/// kind of value it expects and nothing about the delegation is simulated.
pub fn collect(env: &Env, iterable: Unknown) -> Result<Vec<JsSlot>> {
    let sink = Rc::new(RefCell::new(Vec::<JsSlot>::new()));
    let collected = Rc::clone(&sink);

    // The declared argument type is one `Unknown`, but nothing enforces arity
    // on the JS side: a host `forEach` will invoke this with three, and
    // `FunctionCallContext` reads whichever of them exist.
    let collector: Function<'_, Unknown, ()> =
        env.create_function_from_closure("collect", move |context| {
            // A host `forEach` that invokes its callback with no arguments would
            // give upstream's `function (value) { stack.push(value); }` an
            // `undefined` to push, not nothing.
            let value: Unknown = match context.length() {
                0 => undefined(context.env)?,
                _ => context.get(0)?,
            };

            collected
                .borrow_mut()
                .push(JsSlot::new(context.env, &value)?);

            Ok(())
        })?;

    // SAFETY: `collector` is a JS function this call just created.
    let collector = unsafe { Unknown::from_raw_unchecked(env.raw(), collector.raw()) };

    for_each(env, iterable, collector)?;

    let slots = std::mem::take(&mut *sink.borrow_mut());

    Ok(slots)
}

// ---------------------------------------------------------------------------
// Branch 1, the two shapes an "indexed sequence" comes in.
// ---------------------------------------------------------------------------

/// A string, walked by UTF-16 code unit.
///
/// `'ab'[0]` is `'a'`, and JS indexes strings in code units, not code points or
/// bytes — so a surrogate pair yields two halves and a lone surrogate survives.
/// Going through Rust `char`s would silently repair both.
fn each_string(env: &Env, iterable: &Unknown, callback: &Unknown) -> Result<()> {
    let units = crate::js_slot::read_utf16(env, iterable)?;
    // SAFETY: `callback` is type-checked by the caller.
    let callback: Function<'_, FnArgs<(Unknown, u32)>, Unknown> = unsafe { callback.cast()? };

    for (index, unit) in units.iter().enumerate() {
        let character = env.create_string_utf16([*unit])?;
        // SAFETY: a handle from this same environment and scope.
        let character = unsafe { Unknown::from_raw_unchecked(env.raw(), character.raw()) };

        callback.call((character, index as u32).into())?;
    }

    Ok(())
}

/// An array, a typed array, an `arguments` object — or anything whose
/// `toString()` claimed to be one.
///
/// `for (i = 0, l = iterable.length; i < l; i++)`: the length is read **once**,
/// the elements lazily, and `i < l` is JS's numeric comparison, so a `length`
/// of `undefined` gives zero iterations and a `length` of `3.5` gives four.
fn each_indexed(env: &Env, iterable: &Unknown, callback: &Unknown) -> Result<()> {
    let target = coerce_to_object(env, iterable)?;
    let length: Unknown = target.get_named_property_unchecked("length")?;
    let length = to_number(env, &length)?;
    // SAFETY: `callback` is type-checked by the caller.
    let callback: Function<'_, FnArgs<(Unknown, u32)>, Unknown> = unsafe { callback.cast()? };

    let mut index = 0u32;

    while f64::from(index) < length {
        let value: Unknown = target.get_element(index)?;

        callback.call((value, index).into())?;

        index = match index.checked_add(1) {
            Some(next) => next,
            // `length` past 2^32 is unreachable for any real sequence, and JS
            // array indices stop there anyway.
            None => return Ok(()),
        };
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// JS primitives that N-API does not hand over ready-made.
// ---------------------------------------------------------------------------

/// `!!value`, spelled out because Rust has no truthiness.
///
/// Only the empty string, `0`/`-0`/`NaN`, `false`, `0n`, `null` and `undefined`
/// are falsy; every object is truthy, `[]` and `{}` included.
fn is_truthy(env: &Env, value: &Unknown) -> Result<bool> {
    let mut result = false;

    // `napi_coerce_to_bool` is ToBoolean, which is the definition. Doing it by
    // hand per type is how the `'0'`/`''` and `[]`/`0` pairs get confused.
    let mut coerced = ptr::null_mut();
    check(
        // SAFETY: a live handle from `env`.
        unsafe { sys::napi_coerce_to_bool(env.raw(), value.raw(), &mut coerced) },
        "napi_coerce_to_bool",
    )?;
    check(
        // SAFETY: `coerced` is the boolean N-API just produced.
        unsafe { sys::napi_get_value_bool(env.raw(), coerced, &mut result) },
        "napi_get_value_bool",
    )?;

    Ok(result)
}

/// `value === true`, which is not `is_truthy`: `s.done !== true` keeps
/// draining when `done` is `1` or `'yes'`.
fn is_strictly_true(env: &Env, value: &Unknown) -> Result<bool> {
    if value.get_type()? != ValueType::Boolean {
        return Ok(false);
    }

    let mut result = false;

    // SAFETY: the type check above guarantees a boolean.
    check(
        unsafe { sys::napi_get_value_bool(env.raw(), value.raw(), &mut result) },
        "napi_get_value_bool",
    )?;

    Ok(result)
}

pub(crate) fn is_array(env: &Env, value: &Unknown) -> Result<bool> {
    let mut result = false;

    // SAFETY: a live handle from `env`.
    check(
        unsafe { sys::napi_is_array(env.raw(), value.raw(), &mut result) },
        "napi_is_array",
    )?;

    Ok(result)
}

/// `ArrayBuffer.isView`, which is true for every typed array **and** for a
/// `DataView`. N-API splits the two, so both are asked.
pub(crate) fn is_array_buffer_view(env: &Env, value: &Unknown) -> Result<bool> {
    let mut typed_array = false;
    let mut data_view = false;

    // SAFETY: a live handle from `env`.
    unsafe {
        check(
            sys::napi_is_typedarray(env.raw(), value.raw(), &mut typed_array),
            "napi_is_typedarray",
        )?;
        check(
            sys::napi_is_dataview(env.raw(), value.raw(), &mut data_view),
            "napi_is_dataview",
        )?;
    }

    Ok(typed_array || data_view)
}

/// `iterable.toString() === '[object Arguments]'`.
///
/// Faithful down to the failure modes (NOTES B-5): a missing or non-callable
/// `toString` throws the `TypeError` V8 throws, naming obliterator's own
/// variable, and a `toString` that returns the tag hijacks branch 1.
fn has_arguments_tag(env: &Env, value: &Unknown) -> Result<bool> {
    let target = coerce_to_object(env, value)?;
    let to_string: Unknown = target.get_named_property_unchecked("toString")?;

    if to_string.get_type()? != ValueType::Function {
        return Err(type_error(env, "iterable.toString is not a function"));
    }

    // SAFETY: `typeof === 'function'`, checked above.
    let to_string: Function<'_, (), Unknown> = unsafe { to_string.cast()? };
    let tag = to_string.apply(*value, ())?;

    if tag.get_type()? != ValueType::String {
        return Ok(false);
    }

    // SAFETY: the type check above guarantees a string.
    let tag: String = unsafe { tag.cast()? };

    Ok(tag == ARGUMENTS_TAG)
}

/// `Symbol.iterator`, fetched from the running realm rather than reconstructed.
fn well_known_iterator<'env>(env: &'env Env) -> Result<Unknown<'env>> {
    let global = env.get_global()?;
    let symbol: Object = global.get_named_property_unchecked("Symbol")?;

    symbol.get_named_property_unchecked("iterator")
}

/// Whether `key` is an own **enumerable** property, which is the filter
/// `for…in` applies before `hasOwnProperty` ever runs.
fn is_enumerable(env: &Env, target: &Object, key: &Unknown) -> Result<bool> {
    let global = env.get_global()?;
    let object_ctor: Object = global.get_named_property_unchecked("Object")?;
    let descriptor_of: Function<'_, FnArgs<(Unknown, Unknown)>, Unknown> =
        object_ctor.get_named_property("getOwnPropertyDescriptor")?;

    // SAFETY: an object handle from this environment.
    let target = unsafe { Unknown::from_raw_unchecked(env.raw(), target.raw()) };
    let descriptor = descriptor_of.call((target, *key).into())?;

    if descriptor.get_type()? != ValueType::Object {
        return Ok(false);
    }

    // SAFETY: the type check above guarantees an object.
    let descriptor: Object = unsafe { descriptor.cast()? };
    let enumerable: Unknown = descriptor.get_named_property_unchecked("enumerable")?;

    is_strictly_true(env, &enumerable)
}

/// `ToObject`, i.e. the boxing JS performs implicitly for `x.y` on a primitive.
pub(crate) fn coerce_to_object<'env>(env: &'env Env, value: &Unknown) -> Result<Object<'env>> {
    let mut object = ptr::null_mut();

    // SAFETY: a live handle from `env`.
    check(
        unsafe { sys::napi_coerce_to_object(env.raw(), value.raw(), &mut object) },
        "napi_coerce_to_object",
    )?;

    // SAFETY: `object` is the object N-API just produced.
    unsafe { Object::from_napi_value(env.raw(), object) }
}

/// `ToNumber`, which is what `i < l` applies to a non-numeric `length`.
pub(crate) fn to_number(env: &Env, value: &Unknown) -> Result<f64> {
    let mut number = ptr::null_mut();
    let mut result = 0.0;

    // SAFETY: a live handle from `env`; `napi_coerce_to_number` may throw for a
    // symbol, which surfaces as a pending exception and a non-ok status.
    unsafe {
        check(
            sys::napi_coerce_to_number(env.raw(), value.raw(), &mut number),
            "napi_coerce_to_number",
        )?;
        check(
            sys::napi_get_value_double(env.raw(), number, &mut result),
            "napi_get_value_double",
        )?;
    }

    Ok(result)
}

/// `array.join(',')`, which several structures define `toString` as.
///
/// Not `Vec<String>::join`: `Array.prototype.join` renders `null` and
/// `undefined` as the empty string and everything else through `String(v)`, so
/// `[1, null, 2].join(',')` is `"1,,2"` and not `"1,null,2"`.
pub(crate) fn join(env: &Env, slots: &[JsSlot]) -> Result<String> {
    let mut parts = Vec::with_capacity(slots.len());

    for slot in slots {
        let value = slot.get(env)?;

        parts.push(match value.get_type()? {
            ValueType::Null | ValueType::Undefined => String::new(),
            _ => display(env, &value)?,
        });
    }

    Ok(parts.join(","))
}

/// `String(value)`, used to build the `in`-operator `TypeError`'s text and to
/// render elements for [`join`].
pub(crate) fn display(env: &Env, value: &Unknown) -> Result<String> {
    let global = env.get_global()?;
    let string_ctor: Function<'_, Unknown, String> = global.get_named_property("String")?;

    string_ctor.call(*value)
}

pub(crate) fn undefined<'env>(env: &'env Env) -> Result<Unknown<'env>> {
    // SAFETY: `()` is napi's `Undefined`; the produced handle belongs to `env`.
    let raw = unsafe { ToNapiValue::to_napi_value(env.raw(), ())? };

    Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), raw) })
}

/// Throw a real JS `TypeError`, not napi's generic `Error`.
///
/// napi builds an `Error` from a status and a reason, which is right for
/// obliterator's own two messages but wrong for the ones V8 raises. Throwing
/// first and returning `PendingException` leaves the already-thrown value
/// alone: napi's error path re-uses a pending exception rather than replacing
/// it.
pub(crate) fn type_error(env: &Env, message: &str) -> Error {
    if env.throw_type_error(message, None).is_err() {
        return Error::new(Status::GenericFailure, message.to_owned());
    }

    Error::new(Status::PendingException, message.to_owned())
}

/// As [`type_error`], for the `RangeError` a bad array length produces.
///
/// `new Array(-1)` and `new Array(2.5)` both throw `RangeError: Invalid array
/// length`, and the fixed-capacity structures reach it through their
/// `new this.ArrayClass(this.capacity)`. A napi `Error` would arrive in JS as a
/// plain `Error`, which `assert.throws(fn, RangeError)` would not accept.
pub(crate) fn range_error(env: &Env, message: &str) -> Error {
    if env.throw_range_error(message, None).is_err() {
        return Error::new(Status::GenericFailure, message.to_owned());
    }

    Error::new(Status::PendingException, message.to_owned())
}

pub(crate) fn check(status: sys::napi_status, call: &str) -> Result<()> {
    if status == sys::Status::napi_ok {
        return Ok(());
    }

    if status == sys::Status::napi_pending_exception {
        return Err(Error::new(
            Status::PendingException,
            format!("{call} raised a JavaScript exception"),
        ));
    }

    Err(Error::new(
        Status::GenericFailure,
        format!("{call} failed with status {status}"),
    ))
}

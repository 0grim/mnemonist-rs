//! A JavaScript value a Rust collection can hold across calls.
//!
//! `Stack` and `Queue` store *anything* upstream — numbers, strings, objects,
//! `undefined`, functions. The core structures are generic in `T`, so the only
//! question is what `T` the bridge instantiates them at, and the answer has to
//! satisfy three constraints at once:
//!
//! 1. **It must survive between calls.** A `napi_value` is a handle in the
//!    current handle scope and is invalid the moment the call that produced it
//!    returns. Anything stored in a `#[napi]` class must be a *reference*.
//! 2. **It must be [`Clone`].** [`Sequence::slot`](mnemonist_core::cursor::Sequence::slot)
//!    returns `Option<Self::Item>` by value, and `Queue::dequeue` returns a
//!    clone because upstream leaves the element in the array.
//! 3. **It must free itself.** A leaked `napi_ref` keeps its value alive for
//!    the life of the process.
//!
//! # Why this is an enum and not simply a reference
//!
//! The obvious implementation — one `napi_ref` per value — **fails at
//! runtime**. `napi_create_reference` rejects primitives with
//! `napi_invalid_arg` for any module below Node-API 10, and napi-rs 3.12 does
//! not export `node_api_module_get_api_version_v1`, so an addon built with it
//! *is* a version-8 module however its Cargo features are set. Measured, not
//! guessed: with a reference-only slot, `new Stack().push(1)` fails with
//! `napi_create_reference failed with status 1`, and moving the crate from the
//! `napi9` feature to `napi10` changes nothing.
//!
//! So references are used for the types that accept them — object, function,
//! symbol — and primitives are stored **by value** and rebuilt on the way out.
//! That is observationally exact, because primitives are immutable and compared
//! by value: `assert.strictEqual` cannot tell a rebuilt `5` from the original,
//! and `Object.is` cannot tell a rebuilt `-0` or `NaN` either, both of which
//! survive an `f64` round trip intact. Strings are kept as UTF-16 code units
//! rather than as Rust `String`s for the same reason `forEach` walks them that
//! way: a lone surrogate is a legal JS string and `String::from_utf16` would
//! refuse it. BigInts keep their raw words, so arbitrary precision survives.
//!
//! The one thing this design would get wrong is a *mutable* primitive, and
//! JavaScript has none.
//!
//! # Why the `unsafe` is here and not in `mnemonist-core`
//!
//! The crate split doing its job (D-02). `mnemonist-core` keeps
//! `#![forbid(unsafe_code)]` and never hears about any of this. Sharing is
//! [`Rc`], so the reference counting Rust already has does that work, and the
//! only hand-written lifetime rule is [`Handle`]'s `Drop` — N-API reference
//! calls that are documented as safe from a finalizer, which is exactly where
//! they run when a whole collection is collected.

use std::fmt;
use std::ptr;
use std::rc::Rc;

use napi::bindgen_prelude::*;
use napi::sys;

/// A JavaScript value held across calls.
///
/// [`Clone`] shares: a cloned slot denotes the *same* object, so object
/// identity survives `Stack.from([o]).pop() === o`.
#[derive(Clone)]
pub enum JsSlot {
    Undefined,
    Null,
    Boolean(bool),
    /// `-0` and `NaN` both survive this round trip, which is what `Object.is`
    /// would notice if they did not.
    Number(f64),
    /// UTF-16 code units, so a lone surrogate is preserved.
    String(Rc<Vec<u16>>),
    /// `(sign_bit, words)`, the shape `napi_create_bigint_words` takes back.
    BigInt(Rc<(bool, Vec<u64>)>),
    /// Object, function or symbol: the types `napi_create_reference` accepts,
    /// and the ones whose identity a copy would destroy.
    Referenced(Rc<Handle>),
}

impl JsSlot {
    /// Capture `value`, by reference where that is possible and by value where
    /// it is not.
    pub fn new<'env, V: JsValue<'env>>(env: &Env, value: &V) -> Result<Self> {
        // SAFETY: a live handle from `env`; `Unknown` asserts nothing about the
        // value's type, and `get_type` is what finds it out.
        let unknown = unsafe { Unknown::from_raw_unchecked(env.raw(), value.raw()) };

        Ok(match unknown.get_type()? {
            ValueType::Undefined => Self::Undefined,
            ValueType::Null => Self::Null,
            ValueType::Boolean => Self::Boolean(read_bool(env, &unknown)?),
            ValueType::Number => Self::Number(read_double(env, &unknown)?),
            ValueType::String => Self::String(Rc::new(read_utf16(env, &unknown)?)),
            ValueType::BigInt => Self::BigInt(Rc::new(read_bigint(env, &unknown)?)),
            // Object, Function, Symbol, External — everything with an identity.
            _ => Self::Referenced(Rc::new(Handle::new(env, &unknown)?)),
        })
    }

    /// Resolve back to a handle in the caller's scope.
    ///
    /// The returned [`Unknown`] borrows `env`, so it cannot outlive the call it
    /// was resolved in — which is the invariant that made this type necessary.
    pub fn get<'env>(&self, env: &'env Env) -> Result<Unknown<'env>> {
        // SAFETY: `to_napi_value` produces a handle in `env`'s current scope.
        let raw = unsafe { ToNapiValue::to_napi_value(env.raw(), self.clone())? };

        Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), raw) })
    }
}

/// An owning `napi_ref`, released exactly once when the last [`Rc`] drops.
pub struct Handle {
    /// The environment the reference belongs to.
    ///
    /// Stored because [`Drop`] has no other way to reach one. A `napi_env` is
    /// stable for the life of the environment, and every drop site — a method
    /// call, or the class finalizer that runs before the environment is torn
    /// down — is inside that life.
    env: sys::napi_env,
    reference: sys::napi_ref,
}

impl Handle {
    pub(crate) fn new(env: &Env, value: &Unknown) -> Result<Self> {
        let mut reference = ptr::null_mut();

        // SAFETY: `env` is live and `value` is a handle from it. The caller has
        // already established that the value is of a referenceable type.
        let status =
            unsafe { sys::napi_create_reference(env.raw(), value.raw(), 1, &mut reference) };

        check(status, "napi_create_reference")?;

        Ok(Self {
            env: env.raw(),
            reference,
        })
    }

    pub(crate) fn value(&self, env: sys::napi_env) -> Result<sys::napi_value> {
        let mut value = ptr::null_mut();

        // SAFETY: `self.reference` is live until `Drop` releases it.
        let status = unsafe { sys::napi_get_reference_value(env, self.reference, &mut value) };

        check(status, "napi_get_reference_value")?;

        Ok(value)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let mut count = 0;

        // SAFETY: `napi_reference_unref` and `napi_delete_reference` are both
        // permitted from a finalizer, which is where this runs when a whole
        // collection is collected. `Rc` guarantees it happens once.
        unsafe {
            sys::napi_reference_unref(self.env, self.reference, &mut count);
            sys::napi_delete_reference(self.env, self.reference);
        }
    }
}

/// Hand the value back to JavaScript.
///
/// Primitives are rebuilt; referenced values are resolved. Both produce a
/// handle in the caller's scope, so the value stays reachable for the rest of
/// the call even when this slot was the last one holding it.
impl ToNapiValue for JsSlot {
    unsafe fn to_napi_value(env: sys::napi_env, slot: Self) -> Result<sys::napi_value> {
        let mut value = ptr::null_mut();

        // SAFETY: every arm writes `value` through the matching N-API
        // constructor, and the status is checked before it is returned.
        let status = unsafe {
            match &slot {
                Self::Undefined => sys::napi_get_undefined(env, &mut value),
                Self::Null => sys::napi_get_null(env, &mut value),
                Self::Boolean(boolean) => sys::napi_get_boolean(env, *boolean, &mut value),
                Self::Number(number) => sys::napi_create_double(env, *number, &mut value),
                Self::String(units) => sys::napi_create_string_utf16(
                    env,
                    units.as_ptr(),
                    units.len() as isize,
                    &mut value,
                ),
                Self::BigInt(parts) => sys::napi_create_bigint_words(
                    env,
                    i32::from(parts.0),
                    parts.1.len(),
                    parts.1.as_ptr(),
                    &mut value,
                ),
                Self::Referenced(handle) => return handle.value(env),
            }
        };

        check(status, "JsSlot::to_napi_value")?;

        Ok(value)
    }
}

/// Only for `#[napi]`'s TypeScript emission; a slot is an arbitrary JS value
/// and validates nothing.
impl TypeName for JsSlot {
    fn type_name() -> &'static str {
        "unknown"
    }

    fn value_type() -> ValueType {
        ValueType::Unknown
    }
}

impl fmt::Debug for JsSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undefined => formatter.write_str("undefined"),
            Self::Null => formatter.write_str("null"),
            Self::Boolean(boolean) => write!(formatter, "{boolean}"),
            Self::Number(number) => write!(formatter, "{number}"),
            Self::String(units) => write!(formatter, "{:?}", String::from_utf16_lossy(units)),
            Self::BigInt(_) => formatter.write_str("<bigint>"),
            Self::Referenced(_) => formatter.write_str("<reference>"),
        }
    }
}

fn read_bool(env: &Env, value: &Unknown) -> Result<bool> {
    let mut result = false;

    // SAFETY: the caller checked `ValueType::Boolean`.
    check(
        unsafe { sys::napi_get_value_bool(env.raw(), value.raw(), &mut result) },
        "napi_get_value_bool",
    )?;

    Ok(result)
}

fn read_double(env: &Env, value: &Unknown) -> Result<f64> {
    let mut result = 0.0;

    // SAFETY: the caller checked `ValueType::Number`.
    check(
        unsafe { sys::napi_get_value_double(env.raw(), value.raw(), &mut result) },
        "napi_get_value_double",
    )?;

    Ok(result)
}

/// The UTF-16 code units of a JS string, which is how JS indexes one.
///
/// Shared with `crate::foreach`, whose branch-1 string walk needs the same
/// view: `'ab'[0]` is a code unit, not a `char`.
pub(crate) fn read_utf16(env: &Env, value: &Unknown) -> Result<Vec<u16>> {
    let mut length = 0;

    // SAFETY: the caller checked `ValueType::String`. A null buffer asks N-API
    // for the length in code units.
    check(
        unsafe {
            sys::napi_get_value_string_utf16(
                env.raw(),
                value.raw(),
                ptr::null_mut(),
                0,
                &mut length,
            )
        },
        "napi_get_value_string_utf16",
    )?;

    // The API writes a trailing NUL, so the buffer needs one more slot than the
    // string has code units; `written` then excludes it.
    let mut units = vec![0u16; length + 1];
    let mut written = 0;

    // SAFETY: `units` has room for `length` code units plus the terminator.
    check(
        unsafe {
            sys::napi_get_value_string_utf16(
                env.raw(),
                value.raw(),
                units.as_mut_ptr(),
                units.len(),
                &mut written,
            )
        },
        "napi_get_value_string_utf16",
    )?;

    units.truncate(written);

    Ok(units)
}

/// A BigInt as `(sign_bit, little-endian 64-bit words)` — the exact shape
/// `napi_create_bigint_words` takes back, so the round trip is lossless at
/// arbitrary precision.
fn read_bigint(env: &Env, value: &Unknown) -> Result<(bool, Vec<u64>)> {
    let mut sign_bit = 0;
    let mut word_count = 0;

    // SAFETY: the caller checked `ValueType::BigInt`. Node's implementation
    // accepts a query call only when `sign_bit` AND `words` are both null; a
    // null `words` with a non-null `sign_bit` is rejected as `napi_invalid_arg`.
    // Measured, after that exact mistake cost a run.
    check(
        unsafe {
            sys::napi_get_value_bigint_words(
                env.raw(),
                value.raw(),
                ptr::null_mut(),
                &mut word_count,
                ptr::null_mut(),
            )
        },
        "napi_get_value_bigint_words",
    )?;

    let mut words = vec![0u64; word_count];

    // SAFETY: `words` has room for exactly the count N-API just reported.
    check(
        unsafe {
            sys::napi_get_value_bigint_words(
                env.raw(),
                value.raw(),
                &mut sign_bit,
                &mut word_count,
                words.as_mut_ptr(),
            )
        },
        "napi_get_value_bigint_words",
    )?;

    words.truncate(word_count);

    Ok((sign_bit != 0, words))
}

fn check(status: sys::napi_status, call: &str) -> Result<()> {
    if status == sys::Status::napi_ok {
        return Ok(());
    }

    Err(Error::new(
        Status::GenericFailure,
        format!("{call} failed with status {status}"),
    ))
}

//! Holding arbitrary JavaScript **values** across calls.
//!
//! T3's key problem is `Map` semantics; its *value* problem is separate and,
//! for `default-map`, larger. The upstream test file is built on this idiom:
//!
//! ```js
//! map.get('one').push(1);
//! assert.deepStrictEqual(map.get('one'), [1]);
//! ```
//!
//! The array the factory made is stored, handed back, mutated **in place** by
//! the caller, and must still be that same array on the next read. So a stored
//! value cannot be a Rust copy of anything; it has to be a handle to the very
//! JS object, kept alive for as long as the map holds it. The same requirement
//! recurs across the family: `lru-cache` asserts `assert.equal(ret, arr)`
//! against an array and an object, which is reference equality, and
//! `multi-map` asserts `map.get('one') instanceof Set`.
//!
//! # …but only objects need that
//!
//! [`Retained`] is therefore two things, split on the one question that
//! matters: **does this value have an identity a caller could observe?**
//!
//! | value | stored as | why |
//! |---|---|---|
//! | object, function, symbol, bigint | a counted `napi_ref` | identity is observable; the value must be kept alive and handed back as *itself* |
//! | null, boolean, number, string | [`JsPrimitive`], by value | a JS primitive has no identity, so a copy is indistinguishable from the original |
//!
//! That split is not an optimisation bolted onto a reference-everything
//! design; it is forced twice over.
//!
//! * **It is required.** `napi_create_reference` rejects a number at
//!   `NAPI_VERSION` 9, which is what this addon declares. Storing
//!   `map.set('one', 1)` as a reference fails outright — measured, not
//!   assumed: it is what made two of the seven upstream assertions fail on the
//!   first run of this bridge.
//! * **It is also right.** A `napi_ref` is a V8 global handle. One per stored
//!   value would mean a million global handles for a million-entry
//!   `lru-cache`, against upstream's inline SMIs — a benchmark result that
//!   would say more about the bridge than about the port.
//!
//! Nothing is lost by copying a primitive, because JavaScript itself cannot
//! tell: `0 === 0` and `'a' === 'a'` regardless of provenance. `-0` and `NaN`
//! survive too, because [`JsPrimitive::Number`] holds the `f64` verbatim —
//! unlike [`crate::js_key::JsKey`], which normalises both, because *keys* are
//! compared with SameValueZero and *values* are not compared at all.
//!
//! Strings are `Rc<str>`, so cloning one out of the map to hand back is a
//! refcount bump rather than a copy of the text.
//!
//! # Three types, because the boundary has three directions
//!
//! | | direction | why it exists |
//! |---|---|---|
//! | [`Received`] | JS → Rust | resolves `undefined` to `None` **and nothing else**, unlike napi's `Option<T>` |
//! | [`Retained`] | stored | owns the reference, when there is one; must be [`released`](Retained::release) |
//! | [`Loaned`] | Rust → JS | an owned, cheap-to-clone handle that converts back |
//!
//! [`Received`] exists because napi's own `FromNapiValue for Option<T>` maps
//! **both** `undefined` and `null` to `None`. That is wrong here: `null` is a
//! perfectly good `Map` value that has to round-trip, and `lru-cache`'s falsy
//! sweep asserts exactly that — `cache.set('nul', null)` then
//! `assert.strictEqual(ret, null)`. Only `undefined` is absence.
//!
//! # Releasing, and why `Drop` cannot do it
//!
//! `napi_delete_reference` needs an `Env`, and `Drop` has none. So every path
//! that removes a value from a map calls [`Retained::release`] explicitly, and
//! `#[napi(custom_finalize)]` covers the last one: when the map itself is
//! collected, `ObjectFinalize::finalize` runs *with* an env and drains what is
//! left.
//!
//! Forgetting one is not silent. [`Retained`]'s own `Drop` prints to stderr,
//! the same way napi's `UnknownRef` does — loud, in the test output, where it
//! belongs.

use std::ptr;
use std::rc::Rc;

use napi::bindgen_prelude::*;
use napi::{check_status, sys};

/// A JavaScript primitive, stored by value.
///
/// `undefined` is absent from this enum on purpose: absence is core's `None`,
/// one level up.
///
/// [`JsPrimitive::Number`] is a raw `f64` with **no** normalisation, unlike
/// [`crate::js_key::JsKey::Number`]. A value of `-0` must come back as `-0`;
/// only keys are folded, and only because SameValueZero folds them.
#[derive(Debug, Clone)]
pub enum JsPrimitive {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A double, unnormalised: `-0` and each `NaN` bit pattern survive.
    Number(f64),
    /// A string, shared by refcount rather than copied on every hand-out.
    String(Rc<str>),
}

impl JsPrimitive {
    /// Classify a JS value, or report that it is not a primitive this can hold.
    fn from_unknown(value: &Unknown, kind: ValueType) -> Result<Option<Self>> {
        Ok(Some(match kind {
            ValueType::Null => Self::Null,
            // SAFETY (x3): `kind` came from `get_type` on this exact value,
            // which is the precondition `Unknown::cast` documents.
            ValueType::Boolean => Self::Bool(unsafe { value.cast::<bool>()? }),
            ValueType::Number => Self::Number(unsafe { value.cast::<f64>()? }),
            ValueType::String => Self::String(Rc::from(unsafe { value.cast::<String>()? })),
            _ => return Ok(None),
        }))
    }
}

impl ToNapiValue for JsPrimitive {
    /// Every arm delegates to napi's own conversion for the corresponding
    /// Rust type, so nothing here constructs a JS value by hand.
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        unsafe {
            match val {
                Self::Null => ToNapiValue::to_napi_value(env, Null),
                Self::Bool(value) => ToNapiValue::to_napi_value(env, value),
                Self::Number(value) => ToNapiValue::to_napi_value(env, value),
                Self::String(value) => ToNapiValue::to_napi_value(env, &*value),
            }
        }
    }
}

/// A **defined** JavaScript value the map owns.
///
/// Never holds `undefined`: absence is core's `None`, one level up. See
/// [`Received`].
#[derive(Debug)]
pub enum Retained {
    /// No identity to preserve, so the value itself is kept.
    Primitive(JsPrimitive),
    /// Identity is observable, so a counted reference is kept.
    ///
    /// Null once released. A released handle is inert rather than dangling, so
    /// a double release is a no-op instead of a crash.
    Reference(sys::napi_ref),
}

impl Retained {
    /// Take ownership of `value`, by copy or by reference as its type demands.
    pub fn new(value: &Unknown) -> Result<Self> {
        let kind = value.get_type()?;

        if let Some(primitive) = JsPrimitive::from_unknown(value, kind)? {
            return Ok(Self::Primitive(primitive));
        }

        let mut reference = ptr::null_mut();
        check_status!(
            unsafe {
                sys::napi_create_reference(value.value().env, value.raw(), 1, &mut reference)
            },
            "mnemonist-rs: failed to retain a map value of type {kind:?}"
        )?;

        Ok(Self::Reference(reference))
    }

    /// A handle for passing the value back to JavaScript.
    ///
    /// Cheap: a primitive is cloned, and its only heap case is an `Rc<str>`.
    pub fn loan(&self) -> Loaned {
        match self {
            Self::Primitive(primitive) => Loaned::Primitive(primitive.clone()),
            Self::Reference(reference) => Loaned::Reference(*reference),
        }
    }

    /// Drop the reference, if there is one, letting the collector reclaim the
    /// value.
    ///
    /// Idempotent, so a finalizer can run over slots a `delete` already
    /// cleared without having to know which. A primitive owns nothing, so this
    /// is a no-op for one.
    pub fn release(&mut self, env: &Env) -> Result<()> {
        let Self::Reference(slot) = self else {
            return Ok(());
        };

        if slot.is_null() {
            return Ok(());
        }

        let reference = std::mem::replace(slot, ptr::null_mut());

        check_status!(
            unsafe { sys::napi_reference_unref(env.raw(), reference, &mut 0) },
            "mnemonist-rs: failed to unref a map value"
        )?;
        check_status!(
            unsafe { sys::napi_delete_reference(env.raw(), reference) },
            "mnemonist-rs: failed to delete a map value's reference"
        )?;

        Ok(())
    }
}

impl Drop for Retained {
    /// Cannot release — there is no `Env` here — so it reports instead.
    ///
    /// Reaching this means a removal path in the bridge forgot to call
    /// [`Retained::release`], which leaks one JS value. Printing rather than
    /// panicking matches what napi's own `UnknownRef` does, and a panic in a
    /// `Drop` during finalization would take the process with it.
    fn drop(&mut self) {
        if matches!(self, Self::Reference(reference) if !reference.is_null()) {
            eprintln!(
                "mnemonist-rs: a retained JavaScript value was dropped without being released. \
                 This leaks one value; see crates/mnemonist-napi/src/js_value.rs."
            );
        }
    }
}

/// Release a whole core slot — `Option<Retained>`, where `None` is `undefined`
/// and owns nothing.
pub fn release_slot(slot: &mut Option<Retained>, env: &Env) -> Result<()> {
    match slot {
        Some(retained) => retained.release(env),
        None => Ok(()),
    }
}

/// A JavaScript value arriving from a caller, resolved into core's slot shape.
///
/// `undefined` becomes `None`; **`null` does not**. That distinction is the
/// entire reason this type exists rather than napi's `Option<T>`.
pub struct Received(Option<Retained>);

impl Received {
    /// The slot to hand to core.
    pub fn into_slot(self) -> Option<Retained> {
        self.0
    }
}

impl TypeName for Received {
    fn type_name() -> &'static str {
        "unknown"
    }

    fn value_type() -> ValueType {
        ValueType::Unknown
    }
}

impl ValidateNapiValue for Received {}

impl FromNapiValue for Received {
    unsafe fn from_napi_value(env: sys::napi_env, value: sys::napi_value) -> Result<Self> {
        let unknown = unsafe { Unknown::from_napi_value(env, value)? };

        if unknown.get_type()? == ValueType::Undefined {
            return Ok(Self(None));
        }

        Ok(Self(Some(Retained::new(&unknown)?)))
    }
}

/// A value on its way back to JavaScript.
///
/// Owned, because it is what an iterator's `Yield` has to be, and cheap,
/// because the only heap case is an `Rc<str>` bump.
///
/// [`Loaned::Reference`] carries the `napi_ref` itself and does **not** touch
/// its count: the map holds the only count, and the loan is created and
/// converted inside a single napi call, during which no JavaScript can run and
/// therefore nothing can release the map's reference.
pub enum Loaned {
    /// A missing key and a stored `undefined` are the same thing from
    /// JavaScript, and this is both.
    Undefined,
    /// A primitive, copied out — cheap, since the only heap case is an
    /// `Rc<str>` refcount bump.
    Primitive(JsPrimitive),
    /// The stored `napi_ref` itself, borrowed without touching its count.
    /// Valid only for the duration of the napi call that produced it.
    Reference(sys::napi_ref),
}

impl Loaned {
    /// The loan for a core slot, or for a key that was not there at all.
    pub fn of(slot: Option<&Retained>) -> Self {
        match slot {
            Some(retained) => retained.loan(),
            None => Self::Undefined,
        }
    }
}

impl ToNapiValue for Loaned {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        match val {
            Loaned::Undefined => unsafe { ToNapiValue::to_napi_value(env, ()) },
            Loaned::Primitive(primitive) => unsafe { ToNapiValue::to_napi_value(env, primitive) },
            Loaned::Reference(reference) => {
                let mut result = ptr::null_mut();
                check_status!(
                    unsafe { sys::napi_get_reference_value(env, reference, &mut result) },
                    "mnemonist-rs: failed to read back a retained value"
                )?;

                Ok(result)
            }
        }
    }
}

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
//!    clone because upstream leaves the element in the array. A clone here has
//!    to be the *same JS object*, not a copy of it, or object identity — which
//!    `assert.strictEqual` sees — would break.
//! 3. **It must free itself.** A leaked `napi_ref` keeps its value alive for
//!    the life of the process.
//!
//! napi-rs ships `Ref<T>`, `ObjectRef` and `UnknownRef`, and none of the three
//! satisfies all of (1)–(3): they are not `Clone`, and their `Drop` cannot free
//! anything because it has no `Env` to free it with — `UnknownRef`'s literally
//! prints "considered as a memory leak" and gives up. So this type stores the
//! raw `napi_env` alongside the reference and uses N-API's own refcount, which
//! is exactly the mechanism the three constraints describe.
//!
//! # Why the `unsafe` is here and not in `mnemonist-core`
//!
//! This is the crate split doing its job (D-02). `mnemonist-core` keeps
//! `#![forbid(unsafe_code)]` and never hears about any of this; the FFI crate,
//! where `unsafe` is expected and sanctioned, owns the one place a JS handle is
//! kept alive by hand. Every block below is a single N-API reference-management
//! call, all four of which are documented as safe to invoke from a finalizer —
//! which matters, because dropping a collection is exactly when they run.

use std::fmt;
use std::ptr;

use napi::bindgen_prelude::*;
use napi::sys;

/// A refcounted handle to an arbitrary JavaScript value.
///
/// [`Clone`] shares the referent; [`Drop`] releases one share and deletes the
/// reference when the last one goes.
pub struct JsSlot {
    /// The environment the reference belongs to.
    ///
    /// Stored because [`Drop`] has no other way to reach one. A `napi_env` is
    /// stable for the life of the environment, and every drop site — a method
    /// call, or the class finalizer that runs before the environment is torn
    /// down — is inside that life.
    env: sys::napi_env,
    reference: sys::napi_ref,
}

impl JsSlot {
    /// Take a reference to `value`, with an initial count of one.
    pub fn new<'env, V: JsValue<'env>>(env: &Env, value: &V) -> Result<Self> {
        let mut reference = ptr::null_mut();

        // SAFETY: `env` is live for the duration of the call that produced it,
        // and `value.raw()` is a handle obtained from that same environment.
        let status =
            unsafe { sys::napi_create_reference(env.raw(), value.raw(), 1, &mut reference) };

        check(status, "napi_create_reference")?;

        Ok(Self {
            env: env.raw(),
            reference,
        })
    }

    /// Resolve back to a handle in the caller's scope.
    ///
    /// The returned [`Unknown`] borrows `env`, so it cannot outlive the call it
    /// was resolved in — which is the invariant that made this type necessary
    /// in the first place.
    pub fn get<'env>(&self, env: &'env Env) -> Result<Unknown<'env>> {
        let mut value = ptr::null_mut();

        // SAFETY: `self.reference` is live until `Drop` deletes it, and this is
        // not a `Drop`.
        let status =
            unsafe { sys::napi_get_reference_value(env.raw(), self.reference, &mut value) };

        check(status, "napi_get_reference_value")?;

        // SAFETY: `value` is the handle N-API just wrote, in `env`'s current
        // scope; `Unknown` asserts nothing about its type.
        Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), value) })
    }
}

/// Sharing, not copying: both handles denote the same JS value, so
/// `assert.strictEqual` on a cloned slot still passes.
impl Clone for JsSlot {
    fn clone(&self) -> Self {
        let mut count = 0;

        // SAFETY: same environment and reference this value was built from.
        // A failure here can only be a wrong-env/dead-ref programming error,
        // and `Clone` cannot report it; the count simply stays put, which
        // degrades to the leak-free behaviour of a missed increment rather
        // than to a dangling reference.
        unsafe {
            sys::napi_reference_ref(self.env, self.reference, &mut count);
        }

        Self {
            env: self.env,
            reference: self.reference,
        }
    }
}

impl Drop for JsSlot {
    fn drop(&mut self) {
        let mut count = 0;

        // SAFETY: `napi_reference_unref` and `napi_delete_reference` are both
        // explicitly permitted from a finalizer, which is where this runs when
        // a whole collection is collected. Deleting only at zero is what makes
        // `Clone` sound: the last share frees, the others do not.
        unsafe {
            if sys::napi_reference_unref(self.env, self.reference, &mut count)
                != sys::Status::napi_ok
            {
                return;
            }

            if count == 0 {
                sys::napi_delete_reference(self.env, self.reference);
            }
        }
    }
}

/// Hand the value back to JavaScript, releasing this share.
///
/// The handle is written into the caller's scope *before* the release, so the
/// value stays reachable for the rest of the call even when this was the last
/// share — which is exactly the `pop()` case.
impl ToNapiValue for JsSlot {
    unsafe fn to_napi_value(env: sys::napi_env, slot: Self) -> Result<sys::napi_value> {
        let mut value = ptr::null_mut();

        // SAFETY: `slot.reference` is live; `slot` has not been dropped yet.
        let status = unsafe { sys::napi_get_reference_value(env, slot.reference, &mut value) };

        check(status, "napi_get_reference_value")?;

        drop(slot);

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
        formatter.debug_struct("JsSlot").finish_non_exhaustive()
    }
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

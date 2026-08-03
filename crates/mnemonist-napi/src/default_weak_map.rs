//! JS bridge for [`mnemonist_core::structures::default_weak_map`].
//!
//! Read `mnemonist_core::structures::default_weak_map`'s module docs first —
//! this file is where the two things they defer to "the bridge" actually
//! happen: **object identity** and **weak reachability**, neither of which
//! `mnemonist-core` can express without knowing JavaScript exists.
//!
//! # Identity: [`WeakKey`] is a genuinely weak `napi_ref`
//!
//! A real `WeakMap` compares keys by reference and holds them **weakly** —
//! an entry does not keep its key alive, and V8 may reclaim it the moment
//! nothing else does. [`WeakKey`] reproduces exactly that, at the one place
//! this port can: `sys::napi_create_reference(env, value, 0, &mut r)` with an
//! **initial reference count of zero**. Per the Node-API contract (not
//! independently re-derived here, since forcing garbage collection
//! deterministically from a test would be exactly the non-determinism this
//! module's own docs say to design around rather than average out):
//! a zero-count reference does not itself keep the referent alive, and
//! `napi_get_reference_value` on one returns `NULL` once the referent has
//! actually been collected. [`WeakKey::upgrade`] is that read; [`WeakKey::
//! matches`] then compares the live object (if any) against an incoming
//! candidate with `napi_strict_equals` — the same O(n) linear-scan trade-off
//! `crates/mnemonist-napi/src/js_key.rs` documents and declines for `Map`
//! keys (no T3 test needs an object key there); unavoidable here, because an
//! object key is the entire reason a `WeakMap` exists.
//!
//! # Scope: object keys only, not function/symbol
//!
//! A real `WeakMap` also accepts a function or an (unregistered) symbol as a
//! key. This port accepts **plain objects only** — `ValueType::Object` — and
//! rejects everything else, including functions and symbols, with a stated
//! message. `test/default-weak-map.js` never constructs a key any way but
//! `{}`; implementing napi's function/symbol reference paths for a
//! distinction nothing here exercises would be unverifiable scope, the same
//! judgement call `js_key.rs` makes for object keys in the `Map` family.
//!
//! # Reachability: a collected key can never be presented again, so a permanently-dead entry is inert, not wrong
//!
//! Once a key's `WeakKey` fails to upgrade, [`WeakKey::matches`] reports
//! `false` against *any* candidate, forever — which is the correct answer,
//! not a workaround: if the object were still reachable enough for a caller
//! to pass it as an argument again, it would not have been collected. So a
//! dead entry is simply never matched again by anything upstream could still
//! call this with. Its *value*, though, is never released early: this port
//! does not register a finalizer per key to notice the exact moment of
//! collection (see the module docs on why that is a disclosed simplification,
//! not a fix pending), so a collected key's stored value stays retained,
//! taking up one arena-style slot, until the whole [`JsDefaultWeakMap`] is
//! finalized. Nothing observable depends on when that release actually
//! happens — nothing upstream exposes can tell — so this is a memory-shape
//! divergence rather than a behavioural one. See `docs/modules/default-weak-map.md`.
//!
//! # The factory, and why `get` still cannot hold a borrow across it
//!
//! Same discipline as `crates/mnemonist-napi/src/default_map.rs`: the
//! factory is a JS callback that can call back into this very map (PORTBUG-1), so
//! every borrow of `inner` ends before it runs. [`DefaultWeakMap::peek`] is
//! read-only and cheap to call twice; running the factory does not require
//! constructing a [`WeakKey`] until AFTER it returns, on the miss path only,
//! which is also what keeps a re-triggered factory (BUG-DEFAULT-WEAK-MAP-1) from allocating a
//! fresh weak reference on every read of a key that keeps returning
//! `undefined`.

use std::cell::RefCell;
use std::ptr;

use mnemonist_core::structures::default_weak_map::DefaultWeakMap as CoreMap;
use napi::bindgen_prelude::*;
use napi::sys;
use napi::{check_status, JsValue};
use napi_derive::napi;

use crate::js_slot::JsSlot;
use crate::js_value::{release_slot, Loaned, Received, Retained};

/// What a rejected key type is told — `set`'s own path, and `get`'s when the
/// factory has already run and the write it triggers is the step that fails.
/// Matches the wording a real `WeakMap.prototype.set` uses for a primitive
/// key (`TypeError: Invalid value used as weak map key`), verified against
/// Node 24.18.1, but stated in this port's own voice rather than copied,
/// since this port's rejected set (function, symbol, bigint too) is wider.
const UNSUPPORTED: &str = "mnemonist-rs: this port's DefaultWeakMap supports object keys only -- \
     function, symbol and primitive keys need identity comparison this port \
     does not implement -- see docs/modules/default-weak-map.md.";

/// Upstream's constructor message, matched by the original test's
/// `/function/`.
const NOT_A_FUNCTION: &str = "mnemonist/DefaultWeakMap.constructor: expecting a function.";

/// A genuinely weak reference to a JS object used as a key. See the module
/// docs.
struct WeakKey {
    env: sys::napi_env,
    weak_ref: sys::napi_ref,
}

impl WeakKey {
    /// `value` must already be confirmed `ValueType::Object` by the caller —
    /// this never runs the type check itself, so that every call site is
    /// forced to decide up front whether a miss is a throw (`set`, `get`'s
    /// write) or a quiet "never matches" (`peek`/`has`/`delete`).
    fn new(env: &Env, value: &Object) -> Result<Self> {
        let mut weak_ref = ptr::null_mut();

        // SAFETY: `env` is live and `value` is a handle from it. An initial
        // reference count of 0 is what makes this a WEAK reference -- see
        // the module docs.
        check_status!(
            unsafe { sys::napi_create_reference(env.raw(), value.raw(), 0, &mut weak_ref) },
            "mnemonist-rs: failed to create a weak map key reference"
        )?;

        Ok(Self {
            env: env.raw(),
            weak_ref,
        })
    }

    /// The live object, or `None` if it has been collected.
    fn upgrade(&self) -> Result<Option<sys::napi_value>> {
        let mut result = ptr::null_mut();

        // SAFETY: `self.weak_ref` is live until `release` deletes it; a
        // zero-refcount reference legitimately reports a NULL value once its
        // referent is collected, which is not an error.
        check_status!(
            unsafe { sys::napi_get_reference_value(self.env, self.weak_ref, &mut result) },
            "mnemonist-rs: failed to read a weak map key reference"
        )?;

        Ok((!result.is_null()).then_some(result))
    }

    /// Whether this key IS `candidate`, by strict equality of the *live*
    /// object. A collected key never matches anything again -- see the
    /// module docs on why that is the correct answer, not an approximation.
    fn matches(&self, env: &Env, candidate: sys::napi_value) -> bool {
        let Ok(Some(live)) = self.upgrade() else {
            return false;
        };
        let mut equal = false;

        // SAFETY: `live` came from this call's own `upgrade`, and `candidate`
        // is a handle from the same `env`.
        check_status!(
            unsafe { sys::napi_strict_equals(env.raw(), live, candidate, &mut equal) },
            "mnemonist-rs: failed to compare a weak map key"
        )
        .expect("napi_strict_equals should not fail against two live handles in the same env");

        equal
    }

    /// Delete the underlying reference bookkeeping. Idempotent.
    fn release(&mut self) {
        if self.weak_ref.is_null() {
            return;
        }

        let reference = std::mem::replace(&mut self.weak_ref, ptr::null_mut());

        // SAFETY: `napi_delete_reference` is documented safe to call at any
        // time the reference is still live, including from a finalizer.
        unsafe {
            sys::napi_delete_reference(self.env, reference);
        }
    }
}

impl Drop for WeakKey {
    /// Reaching this without a prior [`WeakKey::release`] leaks the
    /// reference's small bookkeeping node (never the referent -- a weak
    /// reference does not hold that). Reported the same way
    /// `crate::js_value::Retained`'s `Drop` reports an unreleased value, for
    /// the same reason: every removal path in this file calls `release`
    /// explicitly, and reaching this means one of them forgot to.
    fn drop(&mut self) {
        if !self.weak_ref.is_null() {
            eprintln!(
                "mnemonist-rs: a weak map key reference was dropped without being released. \
                 This leaks its reference bookkeeping (not the object); see \
                 crates/mnemonist-napi/src/default_weak_map.rs."
            );
        }
    }
}

/// The map as core sees it: an identity-matched key, and a JS value that may
/// be `None` (`undefined`).
type Core = CoreMap<WeakKey, Retained>;

/// The factory: `(key) -> value`, upstream's one-argument signature — no
/// `size` to pass, unlike `DefaultMap`'s two-argument factory, because a
/// `WeakMap` has no `size`. The argument is [`JsSlot`], not `Object`,
/// following `bk_tree.rs`'s `Distance` and `default_map.rs`'s own `Factory`:
/// a `FunctionRef`'s argument type must be lifetime-free to be called back
/// with a value built from a *later* call's `Env`, and `JsSlot` is exactly
/// this port's "arbitrary JS value that survives between calls" type.
type Factory = FunctionRef<FnArgs<(JsSlot,)>, Received>;

#[napi(js_name = "DefaultWeakMap", custom_finalize)]
pub struct JsDefaultWeakMap {
    inner: RefCell<Core>,
    factory: Factory,
}

/// `key.get_type()? == ValueType::Object`, cast if so. Every call site uses
/// this rather than repeating the check, so "what counts as a key" is
/// decided in exactly one place.
fn as_object<'env>(key: &Unknown<'env>) -> Result<Option<Object<'env>>> {
    if key.get_type()? != ValueType::Object {
        return Ok(None);
    }

    // SAFETY: `get_type` has just reported `Object`, which is the
    // precondition `Unknown::cast` documents.
    Ok(Some(unsafe { key.cast::<Object>()? }))
}

#[napi]
impl JsDefaultWeakMap {
    /// `new DefaultWeakMap(factory)`.
    #[napi(constructor)]
    pub fn new(factory: Unknown) -> Result<Self> {
        if factory.get_type()? != ValueType::Function {
            return Err(Error::new(Status::InvalidArg, NOT_A_FUNCTION));
        }

        // SAFETY: `get_type` has just reported `Function`.
        let function = unsafe { factory.cast::<Function<FnArgs<(JsSlot,)>, Received>>()? };

        Ok(Self {
            inner: RefCell::new(Core::new()),
            factory: function.create_ref()?,
        })
    }

    /// Upstream's `clear`: `this.items = new WeakMap();`.
    #[napi]
    pub fn clear(&self, env: Env) -> Result<()> {
        let mut inner = self.inner.borrow_mut();

        for (key, value) in inner.entries_mut() {
            release_slot(value, &env)?;
            key.release();
        }

        inner.clear();

        Ok(())
    }

    /// Upstream's `get`: a mutating read that manufactures and stores a value
    /// when the stored one is `undefined` — reproducing BUG-DEFAULT-WEAK-MAP-1, the same
    /// "tests the value, not the key" defect `default-map.js` has as BUG-DEFAULT-MAP-1,
    /// minus the `size` drift a `WeakMap` has no counter to exhibit.
    ///
    /// # A disclosed ordering divergence for a non-object key
    ///
    /// Upstream's `get` runs the factory FIRST and only fails at the
    /// following `this.items.set(key, value)` — a real `WeakMap.set` — which
    /// throws `TypeError: Invalid value used as weak map key` for anything
    /// but an object. Verified against Node 24.18.1: `get(1)` on a fresh
    /// `DefaultWeakMap` calls the factory (with whatever side effects it
    /// has) and only THEN throws. This port instead rejects a non-object key
    /// immediately, before the factory runs, because reproducing upstream's
    /// exact order would require calling this port's factory with a
    /// non-object argument its own typed signature refuses to carry — a
    /// distinction `test/default-weak-map.js` never exercises (no block
    /// calls `get` with a non-object key at all). See
    /// `docs/modules/default-weak-map.md`'s "Deliberate divergences" for the
    /// full statement; every other observable behaviour of `get` — including
    /// the eventual `TypeError` itself, and the "factory never ran" case for
    /// a hit — matches.
    #[napi]
    pub fn get(&self, env: Env, key: Unknown) -> Result<Loaned> {
        let Some(object) = as_object(&key)? else {
            return Err(Error::new(Status::InvalidArg, UNSUPPORTED));
        };
        let candidate = object.raw();

        // Upstream's `this.items.get(key)`.
        if let Some(loaned) = self
            .inner
            .borrow()
            .peek(|stored: &WeakKey| stored.matches(&env, candidate))
            .map(|value| value.loan())
        {
            return Ok(loaned);
        }

        // The factory runs with nothing borrowed, exactly as upstream's own
        // `this.factory(key)` runs between the read and the write -- and
        // exactly as `default_map.rs`'s `get` does, for the identical PORTBUG-1
        // reason: a `RefCell` panic inside a `#[napi]` method does not unwind
        // into a JS exception, it aborts the process.
        let manufacture = self.factory.borrow_back(&env)?;
        let key_slot = JsSlot::new(&env, &object)?;
        let value = manufacture
            .call((key_slot,).into())
            .map(Received::into_slot)?;

        Ok(Loaned::of(self.inner.borrow_mut().write_from_factory(
            |stored: &WeakKey| stored.matches(&env, candidate),
            || WeakKey::new(&env, &object).expect("key type already validated as Object"),
            value,
        )))
    }

    /// Upstream's `peek`: no factory call, and no throw for any key type —
    /// a real `WeakMap.get` on a non-object argument simply misses.
    #[napi]
    pub fn peek(&self, env: Env, key: Unknown) -> Result<Loaned> {
        let Some(object) = as_object(&key)? else {
            return Ok(Loaned::Undefined);
        };

        Ok(Loaned::of(self.inner.borrow().peek(|stored: &WeakKey| {
            stored.matches(&env, object.raw())
        })))
    }

    /// Upstream's `set`, which returns `this` for chaining.
    ///
    /// Unlike `get`, this throws IMMEDIATELY for a non-object key, before
    /// anything else happens — matching a real `WeakMap.set`, which has no
    /// factory step to run first.
    #[napi]
    pub fn set<'a>(
        &self,
        this: This<'a>,
        env: Env,
        key: Unknown,
        value: Received,
    ) -> Result<This<'a>> {
        let Some(object) = as_object(&key)? else {
            return Err(Error::new(Status::InvalidArg, UNSUPPORTED));
        };
        let candidate = object.raw();

        let displaced = self.inner.borrow_mut().set(
            |stored: &WeakKey| stored.matches(&env, candidate),
            || WeakKey::new(&env, &object).expect("key type already validated as Object"),
            value.into_slot(),
        );

        if let Some(mut displaced) = displaced {
            displaced.release(&env)?;
        }

        Ok(this)
    }

    /// Upstream's `has`: no throw for any key type, same reasoning as `peek`.
    #[napi]
    pub fn has(&self, env: Env, key: Unknown) -> Result<bool> {
        let Some(object) = as_object(&key)? else {
            return Ok(false);
        };

        Ok(self
            .inner
            .borrow()
            .has(|stored: &WeakKey| stored.matches(&env, object.raw())))
    }

    /// Upstream's `delete`: no throw for any key type either.
    #[napi(js_name = "delete")]
    pub fn delete(&self, env: Env, key: Unknown) -> Result<bool> {
        let Some(object) = as_object(&key)? else {
            return Ok(false);
        };

        let removed = self
            .inner
            .borrow_mut()
            .delete(|stored: &WeakKey| stored.matches(&env, object.raw()));

        match removed {
            None => Ok(false),
            Some((mut removed_key, mut slot)) => {
                release_slot(&mut slot, &env)?;
                removed_key.release();

                Ok(true)
            }
        }
    }
}

impl ObjectFinalize for JsDefaultWeakMap {
    /// The last release: every entry still in the map when the map itself is
    /// collected is unreferenced here.
    fn finalize(self, env: Env) -> Result<()> {
        let mut inner = self.inner.into_inner();

        for (key, value) in inner.entries_mut() {
            release_slot(value, &env)?;
            key.release();
        }

        Ok(())
    }
}

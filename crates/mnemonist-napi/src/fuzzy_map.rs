//! JS bridge for [`mnemonist_core::structures::fuzzy_map`].
//!
//! Inherits `default-map`'s T3 value handling (`Received`/`Retained`/`Loaned`)
//! almost verbatim — the only difference is there is no factory, so a miss is
//! simply `undefined` rather than something manufactured. What is genuinely
//! new here is the **hash functions**: `writeHashFunction`/`readHashFunction`
//! are JS callbacks, applied to an arbitrary value (an `add`ed item, or a
//! `set`/`get`/`has` key) *before* core ever sees a key. Their result is what
//! becomes the `JsKey`.
//!
//! # Why the hash functions can't be `FunctionRef<FnArgs<(JsKey, ...)>, _>`
//!
//! `default-map`'s stored factory takes typed, owned arguments (`JsKey`,
//! `f64`) precisely so the `FunctionRef` never has to name a borrowed
//! lifetime. A hash function's argument cannot be typed that narrowly: `add`
//! hashes the item itself, which upstream's own test passes as an arbitrary
//! object (`{title: 'Hello'}`) — the hash function's *input* is unconstrained
//! even though its *output* must become a `JsKey`. `crates/mnemonist-napi/src/bit_vector.rs`
//! already establishes the pattern this needs: `FunctionRef<f64, Unknown<'static>>`
//! for a stored callback whose *return* type is an unclassified JS value. This
//! module is the same idea on the *argument* side — `FunctionRef<Unknown<'static>,
//! Unknown<'static>>` — reconstructed from a real, live `Unknown` at every call
//! site via `Unknown::from_raw_unchecked`, the same escape hatch `crate::foreach`
//! uses throughout to build a handle in the caller's own scope. The `'static` is
//! never actually relied on past the single `.call(...)` it is used in.
//!
//! # `identity`, folded into "no hash function" rather than installed as one
//!
//! ```js
//! if (!this.writeHashFunction) this.writeHashFunction = identity;
//! if (!this.readHashFunction) this.readHashFunction = identity;
//! ```
//!
//! Upstream substitutes `identity` for a **falsy** descriptor slot — not just
//! a missing one; `new FuzzyMap([null, hash])` is legal and leaves
//! `writeHashFunction` as `identity`. Rather than build a real JS closure for
//! that (and pay a `FunctionRef` + a round trip through JS for what is a
//! no-op), [`HashFn`] is `Option<Function>`: `None` means "hash by classifying
//! the value directly", which is observably identical because `identity(x)`
//! returns `x` verbatim and `JsKey::from_unknown` is exactly what would
//! happen to that return value next.
//!
//! # Re-entrancy
//!
//! Every method's `RefCell` borrow ends before the hash function call — the
//! same discipline `default-map::get`'s factory split enforces — so a hash
//! function that reads back into the same `FuzzyMap` (however contrived)
//! never meets an outstanding borrow.

use std::cell::RefCell;
use std::rc::Rc;

use mnemonist_core::structures::fuzzy_map::FuzzyMap as CoreMap;
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::foreach;
use crate::js_key::JsKey;
use crate::js_value::{release_slot, Loaned, Retained};
use crate::map_cursor::CellMapCursor;

/// The map as core sees it: hashed `JsKey`s, `None` for a stored `undefined`.
type Core = CoreMap<JsKey, Retained>;

/// The cursor `.values()` hands out.
type Cursor = CellMapCursor<JsFuzzyMap, Core, JsKey, Option<Retained>>;

/// A stored hash function, or `None` for upstream's `identity` substitution.
/// See the module docs for why the argument and return are both an
/// unclassified `Unknown<'static>`.
type HashFn = FunctionRef<Unknown<'static>, Unknown<'static>>;

const INVALID_HASH: &str = "mnemonist/FuzzyMap.constructor: invalid hash function given.";

#[napi(js_name = "FuzzyMap", custom_finalize)]
pub struct JsFuzzyMap {
    inner: RefCell<Core>,
    write_hash: Option<HashFn>,
    read_hash: Option<HashFn>,
}

#[napi]
impl JsFuzzyMap {
    /// `new FuzzyMap(descriptor)`.
    ///
    /// `descriptor` is an `[write, read]` pair when it is an array, and both
    /// directions' hash function otherwise — including when it is omitted
    /// entirely, which upstream's own JSDoc does not advertise but its
    /// falsy-substitution logic accepts (`new FuzzyMap()` hashes by
    /// `identity` both ways).
    #[napi(constructor)]
    pub fn new(env: Env, descriptor: Option<Unknown>) -> Result<Self> {
        let (write_candidate, read_candidate) = match &descriptor {
            Some(value) if foreach::is_array(&env, value)? => {
                // SAFETY: `is_array` just reported this exact shape.
                let array = unsafe { value.cast::<Array>()? };

                (array.get::<Unknown>(0)?, array.get::<Unknown>(1)?)
            }
            Some(value) => (Some(*value), Some(*value)),
            None => (None, None),
        };

        Ok(Self {
            inner: RefCell::new(Core::new()),
            write_hash: resolve_hash(&env, write_candidate)?,
            read_hash: resolve_hash(&env, read_candidate)?,
        })
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    #[napi]
    pub fn clear(&self, env: Env) -> Result<()> {
        let mut inner = self.inner.borrow_mut();

        for slot in inner.values_mut() {
            release_slot(slot, &env)?;
        }

        inner.clear();

        Ok(())
    }

    /// Upstream's `add`: hash the *item itself* with `writeHashFunction`, then
    /// store the item under that key.
    #[napi]
    pub fn add<'a>(&self, this: This<'a>, env: Env, item: Unknown) -> Result<This<'a>> {
        let key = apply_hash(&env, &self.write_hash, item)?;
        let slot = retain(&item)?;

        self.store(&env, key, slot)?;

        Ok(this)
    }

    /// Upstream's `set`: hash the *given key* with `writeHashFunction`; the
    /// item is stored as-is.
    #[napi]
    pub fn set<'a>(
        &self,
        this: This<'a>,
        env: Env,
        key: Unknown,
        item: Unknown,
    ) -> Result<This<'a>> {
        let hashed = apply_hash(&env, &self.write_hash, key)?;
        let slot = retain(&item)?;

        self.store(&env, hashed, slot)?;

        Ok(this)
    }

    /// Upstream's `get`: hash `key` with `readHashFunction`.
    #[napi]
    pub fn get(&self, env: Env, key: Unknown) -> Result<Loaned> {
        let hashed = apply_hash(&env, &self.read_hash, key)?;

        Ok(Loaned::of(self.inner.borrow().get(&hashed)))
    }

    /// Upstream's `has`: hash `key` with `readHashFunction`.
    #[napi]
    pub fn has(&self, env: Env, key: Unknown) -> Result<bool> {
        let hashed = apply_hash(&env, &self.read_hash, key)?;

        Ok(self.inner.borrow().has(&hashed))
    }

    /// Upstream's `forEach`: `this.items.forEach(function(value) {
    /// callback.call(scope, value, value); })` — the value twice, never the
    /// key, and no third "collection" argument at all.
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(Loaned, Loaned)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let mut cursor = self.inner.borrow().cursor();

        let mut step = || {
            let inner = self.inner.borrow();

            cursor
                .step(inner.items())
                .map(|(_, value)| (Loaned::of(value.as_ref()), Loaned::of(value.as_ref())))
        };

        while let Some((first, second)) = step() {
            let args = FnArgs::from((first, second));

            match &scope {
                Some(scope) => callback.apply(*scope, args)?,
                None => callback.apply(this, args)?,
            };
        }

        Ok(())
    }

    /// Upstream's `values`, and its `Symbol.iterator` — the only iteration
    /// method this module has.
    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsFuzzyMap>) -> Result<JsFuzzyMapValues> {
        Ok(JsFuzzyMapValues {
            cursor: CellMapCursor::open(this.share_with(env, |map| Ok(&map.inner))?, Core::items),
        })
    }

    /// `FuzzyMap.from(iterable, descriptor, useSet)`:
    /// `forEach(iterable, function(value, key) { if (useSet) map.set(key,
    /// value); else map.add(value); });`
    ///
    /// Collects every `(value, key)` pair the dispatch visits **before**
    /// hashing or storing any of them — see [`collect_pairs`] for why: the
    /// collector closure `for_each` drives must be `'static`, and `map`'s hash
    /// functions are not.
    #[napi(factory)]
    pub fn from(
        env: Env,
        iterable: Unknown,
        descriptor: Option<Unknown>,
        use_set: Option<bool>,
    ) -> Result<Self> {
        let pairs = collect_pairs(&env, iterable)?;
        let map = Self::new(env, descriptor)?;
        let use_set = use_set.unwrap_or(false);

        for (value, key) in pairs {
            if use_set {
                let key = resolve(&env, &key)?;
                let hashed = apply_hash(&env, &map.write_hash, key)?;

                map.store(&env, hashed, value)?;
            } else {
                let item = resolve(&env, &value)?;
                let hashed = apply_hash(&env, &map.write_hash, item)?;

                map.store(&env, hashed, value)?;
            }
        }

        Ok(map)
    }

    fn store(&self, env: &Env, key: JsKey, slot: Option<Retained>) -> Result<()> {
        let displaced = self.inner.borrow_mut().set(key, slot);

        if let Some(mut displaced) = displaced {
            displaced.release(env)?;
        }

        Ok(())
    }
}

impl ObjectFinalize for JsFuzzyMap {
    fn finalize(self, env: Env) -> Result<()> {
        for slot in self.inner.borrow_mut().values_mut() {
            release_slot(slot, &env)?;
        }

        Ok(())
    }
}

/// Resolve one descriptor slot to a stored hash function.
///
/// `None` for a **falsy** value — matching upstream's `if
/// (!this.writeHashFunction) this.writeHashFunction = identity;`, which is a
/// truthiness test, not a null check (`0`, `''`, `false` all fall through to
/// `identity` too, exactly as `false`/`0` would as a hash function be
/// nonsensical but legal input). A truthy non-function is upstream's
/// `INVALID_HASH` throw.
fn resolve_hash(env: &Env, candidate: Option<Unknown>) -> Result<Option<HashFn>> {
    let Some(candidate) = candidate else {
        return Ok(None);
    };

    if !is_truthy(env, &candidate)? {
        return Ok(None);
    }

    if candidate.get_type()? != ValueType::Function {
        return Err(Error::new(Status::InvalidArg, INVALID_HASH));
    }

    // SAFETY: the type check above is exactly `typeof === 'function'`.
    let function = unsafe { candidate.cast::<Function<Unknown<'static>, Unknown<'static>>>()? };

    Ok(Some(function.create_ref()?))
}

/// `!!value`, upstream's truthiness test for a descriptor slot. A private
/// copy of `crate::foreach`'s (module-private there), rather than widening
/// that function's visibility for one caller outside it.
fn is_truthy(env: &Env, value: &Unknown) -> Result<bool> {
    let mut coerced = std::ptr::null_mut();
    let mut result = false;

    napi::check_status!(
        unsafe { sys::napi_coerce_to_bool(env.raw(), value.raw(), &mut coerced) },
        "napi_coerce_to_bool"
    )?;
    napi::check_status!(
        unsafe { sys::napi_get_value_bool(env.raw(), coerced, &mut result) },
        "napi_get_value_bool"
    )?;

    Ok(result)
}

/// Hash `value`: `identity` (i.e. classify it directly) when `hash` is
/// `None`, otherwise the stored JS function's return value.
fn apply_hash(env: &Env, hash: &Option<HashFn>, value: Unknown) -> Result<JsKey> {
    match hash {
        None => JsKey::from_unknown(&value),
        Some(function) => {
            let callable = function.borrow_back(env)?;
            // SAFETY: `value` is a live handle from `env`, the same
            // environment the call below runs in; the reconstructed
            // `Unknown<'static>` is used only for the duration of this one
            // `.call`, never stored. Same escape hatch as
            // `crate::foreach`'s throughout.
            let argument = unsafe { Unknown::from_raw_unchecked(env.raw(), value.raw()) };
            let result = callable.call(argument)?;

            JsKey::from_unknown(&result)
        }
    }
}

/// Take ownership of `value`, the same way [`JsFuzzyMap::store`] does — `None`
/// for `undefined`, `Some` otherwise. A small shared helper rather than
/// inlining it at both call sites (`add`/`set` directly, and
/// [`collect_pairs`]'s collector).
fn retain(value: &Unknown) -> Result<Option<Retained>> {
    match value.get_type()? {
        ValueType::Undefined => Ok(None),
        _ => Ok(Some(Retained::new(value)?)),
    }
}

/// The inverse of [`retain`]: a handle in `env`'s current scope, for a slot
/// collected earlier in the same call. `None` becomes a real `undefined`
/// value rather than napi's `null`.
fn resolve<'env>(env: &'env Env, slot: &Option<Retained>) -> Result<Unknown<'env>> {
    let Some(retained) = slot else {
        return foreach::undefined(env);
    };

    // SAFETY: `to_napi_value` produces a handle in `env`'s current scope,
    // exactly as `JsSlot::get` does for the same reason.
    let raw = unsafe { ToNapiValue::to_napi_value(env.raw(), retained.loan())? };

    Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), raw) })
}

/// Everything [`crate::foreach::for_each`] would visit, as `(value, key)`
/// pairs already lifted out of the current call — the shape `FuzzyMap.from`'s
/// dispatch (`add`/`set`, chosen by `useSet`) needs.
///
/// Collected **before** any hashing happens, for a reason specific to this
/// module: the collector closure `for_each` drives has to satisfy
/// `Fn(...) + 'static` (a JS function may, in general, escape the call that
/// created it), but hashing needs `map`'s `write_hash`/`read_hash`, which are
/// neither `'static`-free to capture by reference nor cheaply cloned. Every
/// other T3 `.from` avoids the question because its collector needs no
/// borrowed state at all (`bi_map`'s classifies straight to `JsKey`); this one
/// defers the part that does, exactly as `[JsFuzzyMap::from]` shows.
fn collect_pairs(
    env: &Env,
    iterable: Unknown,
) -> Result<Vec<(Option<Retained>, Option<Retained>)>> {
    let sink = Rc::new(RefCell::new(
        Vec::<(Option<Retained>, Option<Retained>)>::new(),
    ));
    let collected = Rc::clone(&sink);

    let collector: Function<'_, Unknown, ()> =
        env.create_function_from_closure("collect_pairs", move |context| {
            let value: Unknown = match context.length() {
                0 => foreach::undefined(context.env)?,
                _ => context.get(0)?,
            };
            let key: Unknown = match context.length() {
                len if len > 1 => context.get(1)?,
                _ => foreach::undefined(context.env)?,
            };

            let value = retain(&value)?;
            let key = retain(&key)?;

            collected.borrow_mut().push((value, key));

            Ok(())
        })?;

    // SAFETY: `collector` is a JS function this call just created.
    let collector = unsafe { Unknown::from_raw_unchecked(env.raw(), collector.raw()) };

    foreach::for_each(env, iterable, collector)?;

    let pairs = std::mem::take(&mut *sink.borrow_mut());

    Ok(pairs)
}

/// The cursor `FuzzyMap.prototype.values()` hands out.
#[napi(iterator, js_name = "FuzzyMapValues")]
pub struct JsFuzzyMapValues {
    cursor: Cursor,
}

impl Generator for JsFuzzyMapValues {
    type Yield = Loaned;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Loaned> {
        self.cursor.step(|_, value| Loaned::of(value.as_ref()))
    }

    /// A native `Map` iterator has no `return` method — see
    /// `default_map::JsDefaultMapEntries::complete` for the same note.
    fn complete(&mut self, _value: Option<()>) -> Option<Loaned> {
        None
    }
}

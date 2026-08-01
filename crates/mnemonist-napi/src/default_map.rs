//! JS bridge for [`mnemonist_core::structures::default_map`].
//!
//! The pilot for bridge tier T3. Thin, as every bridge should be — but three
//! things happen here that have no equivalent in T0/T1, and all three are
//! inherited by the ten remaining T3 modules.
//!
//! 1. **Keys become [`JsKey`]**, which is where SameValueZero lives.
//! 2. **Values stay JavaScript.** [`Received`]/[`Retained`]/[`Loaned`] keep the
//!    caller's actual object alive across calls, because the upstream test
//!    mutates a returned array in place and reads it back.
//!    `#[napi(custom_finalize)]` is what eventually releases them.
//! 3. **The factory is a JS callback**, held as a [`FunctionRef`] and called
//!    from inside core's `try_get_or_insert_with`, so that a factory which
//!    throws leaves the map untouched exactly as upstream's does.
//!
//! Four fidelity notes, in descending order of how likely they are to matter:
//!
//! * **`forEach`'s third callback argument.** Upstream delegates to
//!   `this.items.forEach(...)`, so the native `Map` passes *itself* — the
//!   inner map, not the `DefaultMap`. There is no inner JS `Map` object in
//!   this port, so the `DefaultMap` is passed instead. The upstream test's
//!   callback declares two parameters.
//! * **`forEach`'s `scope`.** Upstream is `arguments.length > 1 ? scope : this`
//!   and napi's typed signature cannot see `arguments.length`; identical to
//!   `SparseSet::for_each`, and recorded the same way.
//! * **`inspect()` is not ported.** It returns the inner `Map`, which does not
//!   exist here, and nothing asserts on it.
//! * **A re-entrant factory or `forEach` callback** — one that calls back into
//!   the same map — is not supported. Upstream allows it. See
//!   `docs/modules/default-map.md`.

use std::cell::Cell;

use mnemonist_core::structures::default_map::DefaultMap as CoreMap;
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::js_key::JsKey;
use crate::js_value::{release_slot, Loaned, Received, Retained};
use crate::map_cursor::MapBridgeCursor;

/// The map as core sees it: JS keys, JS values, `None` for `undefined`.
type Core = CoreMap<JsKey, Retained>;

/// The cursor every one of the three iterators holds.
///
/// Core stores `Option<V>`, where `None` is `undefined`; the three iterators
/// differ only in what they project out of each step.
type Cursor = MapBridgeCursor<JsDefaultMap, JsKey, Option<Retained>>;

/// The factory's JS signature: `(key, size) -> value`.
///
/// The second argument is `f64`, not an integer, for two reasons: JavaScript
/// numbers are doubles, and `size` here is upstream's *drifting* counter
/// (B-11), which is not bounded by the entry count.
type Factory = FunctionRef<FnArgs<(JsKey, f64)>, Received>;

/// Upstream's constructor message, matched by the original test's `/function/`.
const NOT_A_FUNCTION: &str = "mnemonist/DefaultMap.constructor: expecting a function.";

#[napi(js_name = "DefaultMap", custom_finalize)]
pub struct JsDefaultMap {
    inner: Core,
    factory: Factory,
}

#[napi]
impl JsDefaultMap {
    /// `new DefaultMap(factory)`.
    ///
    /// The `typeof factory !== 'function'` check is upstream's, kept verbatim
    /// including its message, and it lives here rather than in core because it
    /// is a JavaScript type test.
    #[napi(constructor)]
    pub fn new(factory: Unknown) -> Result<Self> {
        if factory.get_type()? != ValueType::Function {
            return Err(Error::new(Status::InvalidArg, NOT_A_FUNCTION));
        }

        // SAFETY: `get_type` has just reported `Function`, which is the
        // precondition `Unknown::cast` documents.
        let function = unsafe { factory.cast::<Function<FnArgs<(JsKey, f64)>, Received>>()? };

        Ok(Self {
            inner: Core::new(),
            factory: function.create_ref()?,
        })
    }

    /// Upstream's `size` **property**.
    ///
    /// A drifting counter, not the entry count. See
    /// `mnemonist_core::structures::default_map` and B-11.
    #[napi(getter)]
    pub fn size(&self) -> f64 {
        self.inner.size() as f64
    }

    #[napi]
    pub fn clear(&mut self, env: Env) -> Result<()> {
        for slot in self.inner.values_mut() {
            release_slot(slot, &env)?;
        }

        self.inner.clear();

        Ok(())
    }

    /// Upstream's `get`: a **mutating** read that manufactures and stores a
    /// value when the stored one is `undefined`.
    #[napi]
    pub fn get(&mut self, env: Env, key: JsKey) -> Result<Loaned> {
        // Split the borrow: the factory is read while the map is written.
        let Self { inner, factory } = self;
        let manufacture = factory.borrow_back(&env)?;

        let value = inner.try_get_or_insert_with(key, |key, size| {
            manufacture
                .call((key.clone(), size as f64).into())
                .map(Received::into_slot)
        })?;

        Ok(Loaned::of(value))
    }

    /// Upstream's `peek`: no factory, no counter change.
    ///
    /// `undefined` for a missing key *and* for a key holding `undefined`,
    /// because upstream's caller cannot tell those apart either.
    #[napi]
    pub fn peek(&self, key: JsKey) -> Loaned {
        Loaned::of(self.inner.peek(&key))
    }

    /// Upstream's `set`, which returns `this` for chaining.
    #[napi]
    pub fn set<'a>(
        &mut self,
        this: This<'a>,
        env: Env,
        key: JsKey,
        value: Received,
    ) -> Result<This<'a>> {
        if let Some(mut displaced) = self.inner.set(key, value.into_slot()) {
            displaced.release(&env)?;
        }

        Ok(this)
    }

    /// Upstream's `has`, which asks about the **key**.
    ///
    /// So it is `true` for a key holding `undefined`, even though `get` on
    /// that key will run the factory again.
    #[napi]
    pub fn has(&self, key: JsKey) -> bool {
        self.inner.has(&key)
    }

    #[napi(js_name = "delete")]
    pub fn delete(&mut self, env: Env, key: JsKey) -> Result<bool> {
        match self.inner.delete(&key) {
            None => Ok(false),
            Some(mut slot) => {
                release_slot(&mut slot, &env)?;

                Ok(true)
            }
        }
    }

    /// Upstream's `forEach`, which is the backing `Map`'s.
    ///
    /// Live, like every `Map` walk: an entry the callback adds **is** visited,
    /// one it deletes ahead of the cursor is **not**. That falls out of using
    /// the same cursor the iterators use, rather than being arranged for.
    ///
    /// See the module docs for the two divergences — the third callback
    /// argument, and `scope` under `arguments.length`.
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(Loaned, JsKey, Object)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let mut cursor = self.inner.cursor();

        while let Some((key, value)) = cursor.step(self.inner.items()) {
            let args = FnArgs::from((Loaned::of(value.as_ref()), key.clone(), this.object));

            match &scope {
                Some(scope) => callback.apply(*scope, args)?,
                None => callback.apply(this, args)?,
            };
        }

        Ok(())
    }

    /// Upstream's `entries`, and its `Symbol.iterator`.
    ///
    /// `crate::cursor::install_iterator_factories` aliases `Symbol.iterator`
    /// onto this method, exactly as upstream's last line does — and note that
    /// upstream aliases `entries`, not `values`, so a spread of a `DefaultMap`
    /// yields `[key, value]` pairs.
    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsDefaultMap>) -> Result<JsDefaultMapEntries> {
        Ok(JsDefaultMapEntries {
            cursor: MapBridgeCursor::open(this.share_with(env, |map| Ok(map.inner.items()))?),
        })
    }

    #[napi]
    pub fn keys(&self, env: Env, this: Reference<JsDefaultMap>) -> Result<JsDefaultMapKeys> {
        Ok(JsDefaultMapKeys {
            cursor: MapBridgeCursor::open(this.share_with(env, |map| Ok(map.inner.items()))?),
        })
    }

    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsDefaultMap>) -> Result<JsDefaultMapValues> {
        Ok(JsDefaultMapValues {
            cursor: MapBridgeCursor::open(this.share_with(env, |map| Ok(map.inner.items()))?),
        })
    }

    /// `DefaultMap.autoIncrement()` — a static that returns a *stateful*
    /// closure, one counter per call.
    ///
    /// Built as a real JS function rather than a Rust type, because upstream's
    /// is one and callers pass it straight to the constructor. The counter is a
    /// [`Cell`] because napi requires `Fn`, not `FnMut`; single-threaded by
    /// construction, since a napi callback only ever runs on its own JS thread.
    #[napi(js_name = "autoIncrement")]
    pub fn auto_increment(env: Env) -> Result<Function<'static, (), f64>> {
        let counter = Cell::new(0f64);

        let function: Function<(), f64> =
            env.create_function_from_closure("autoIncrement", move |_| {
                let next = counter.get();
                counter.set(next + 1.0);

                Ok(next)
            })?;

        // `Function`'s lifetime is phantom -- it holds an env pointer and a
        // value pointer -- and `create_function_from_closure` ties it to the
        // borrow of `env`, which is a local here. Re-adopting the same handle
        // frees the lifetime.
        //
        // SAFETY: both pointers came from napi in this call and are converted
        // to a napi value by the generated wrapper before anything else runs.
        unsafe { Function::from_napi_value(env.raw(), function.raw()) }
    }
}

impl ObjectFinalize for JsDefaultMap {
    /// The last release. Everything still in the map when the map itself is
    /// collected is unreferenced here, which is the one removal path that
    /// cannot be reached from a method call.
    fn finalize(mut self, env: Env) -> Result<()> {
        for slot in self.inner.values_mut() {
            release_slot(slot, &env)?;
        }

        Ok(())
    }
}

/// One `[key, value]` pair, as `Map.prototype.entries` yields it.
///
/// A dedicated type rather than a tuple because napi has no tuple-to-array
/// conversion, and the array has to be built with the env that only exists at
/// conversion time.
pub struct Pair(JsKey, Loaned);

impl ToNapiValue for Pair {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let key = unsafe { ToNapiValue::to_napi_value(env, val.0)? };
        let value = unsafe { ToNapiValue::to_napi_value(env, val.1)? };

        let mut pair = std::ptr::null_mut();
        napi::check_status!(
            unsafe { sys::napi_create_array_with_length(env, 2, &mut pair) },
            "mnemonist-rs: failed to build a [key, value] pair"
        )?;
        napi::check_status!(
            unsafe { sys::napi_set_element(env, pair, 0, key) },
            "mnemonist-rs: failed to set a pair's key"
        )?;
        napi::check_status!(
            unsafe { sys::napi_set_element(env, pair, 1, value) },
            "mnemonist-rs: failed to set a pair's value"
        )?;

        Ok(pair)
    }
}

/// The cursor `DefaultMap.prototype.entries()` hands out.
///
/// `#[napi(iterator)]` supplies the identity `Symbol.iterator` — the cursor
/// returns itself, so it is non-restartable — which is what a native `Map`
/// iterator does too.
#[napi(iterator, js_name = "DefaultMapEntries")]
pub struct JsDefaultMapEntries {
    cursor: Cursor,
}

impl Generator for JsDefaultMapEntries {
    type Yield = Pair;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Pair> {
        let (key, value) = self.cursor.step()?;

        Some(Pair(key.clone(), Loaned::of(value.as_ref())))
    }

    /// `Generator.return()`, which a `break` out of a `for…of` calls.
    ///
    /// A native `Map` iterator has **no** `return` method at all — verified
    /// against Node 24.18.1 — so `break` leaves it where it stopped and a later
    /// `next()` resumes. napi's default `complete` returns `None` without
    /// touching the walk, which is the same observable behaviour.
    fn complete(&mut self, _value: Option<()>) -> Option<Pair> {
        None
    }
}

#[napi(iterator, js_name = "DefaultMapKeys")]
pub struct JsDefaultMapKeys {
    cursor: Cursor,
}

impl Generator for JsDefaultMapKeys {
    type Yield = JsKey;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<JsKey> {
        let (key, _) = self.cursor.step()?;

        Some(key.clone())
    }

    fn complete(&mut self, _value: Option<()>) -> Option<JsKey> {
        None
    }
}

#[napi(iterator, js_name = "DefaultMapValues")]
pub struct JsDefaultMapValues {
    cursor: Cursor,
}

impl Generator for JsDefaultMapValues {
    type Yield = Loaned;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Loaned> {
        let (_, value) = self.cursor.step()?;

        Some(Loaned::of(value.as_ref()))
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Loaned> {
        None
    }
}

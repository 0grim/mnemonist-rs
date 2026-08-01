//! JS bridge for [`mnemonist_core::structures::bi_map`].
//!
//! Inherits the T3 pilot's machinery directly: [`JsKey`] for SameValueZero,
//! [`CellMapCursor`] for a live cursor over a JS-owned instance. Nothing here
//! needs [`crate::js_value`]'s `Received`/`Retained`/`Loaned` split at all —
//! unlike `default-map`, a `BiMap`'s *values* are themselves `Map` keys
//! somewhere (in `.inverse`), so both directions are `JsKey`, and `JsKey` is
//! plain data with no napi reference to release. That is also why this
//! struct needs no `#[napi(custom_finalize)]`: there is nothing to clean up.
//!
//! # `.inverse` is a live view, not a snapshot
//!
//! Upstream's `BiMap` holds a real second object, `this.inverse`, whose six
//! delegating methods (`has`, `get`, `forEach`, `keys`, `values`, `entries`)
//! plus `set`/`delete`/`clear` read and write the *same* two `Map`s the
//! `BiMap` itself does. [`JsBiMapInverse`] reproduces that by holding a
//! [`SharedReference`] to the **same** `RefCell<CoreBiMap>` [`JsBiMap`] owns —
//! obtained through `share_with`, the identical mechanism the T3 cursors use
//! to reach a JS-owned parent — rather than by copying any state. A method on
//! either object therefore observes every write the other one makes.
//!
//! # Re-entrancy
//!
//! Every method here is a single, self-contained `OrderedMap` operation with
//! no user callback in the middle except `forEach`, whose borrow is taken and
//! released per step exactly as `default-map`'s is (B-31). There is no
//! `try_borrow` anywhere in this file: nothing here ever calls back into
//! JavaScript while the map is locked, so an ordinary `borrow`/`borrow_mut`
//! is the same guarantee `RefCell` gives every other T3 bridge.

use std::cell::RefCell;
use std::rc::Rc;

use mnemonist_core::map::OrderedMap;
use mnemonist_core::structures::bi_map::BiMap as CoreBiMap;
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::foreach;
use crate::js_key::JsKey;
use crate::map_cursor::CellMapCursor;

/// The map as core sees it: both directions are `JsKey`. See the module docs.
type Core = CoreBiMap<JsKey>;

/// The cursor every one of the six iterators (three per view) holds.
type Cursor<Owner> = CellMapCursor<Owner, Core, JsKey, JsKey>;

fn items(map: &Core) -> &OrderedMap<JsKey, JsKey> {
    map.items()
}

fn inverse_items(map: &Core) -> &OrderedMap<JsKey, JsKey> {
    map.inverse()
}

/// `undefined` for a missing key, never `null` — see [`JsBiMap::get`].
fn into_either(value: Option<JsKey>) -> Either<JsKey, Undefined> {
    match value {
        Some(key) => Either::A(key),
        None => Either::B(()),
    }
}

#[napi(js_name = "BiMap")]
pub struct JsBiMap {
    inner: RefCell<Core>,
}

#[napi]
impl JsBiMap {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(Core::new()),
        }
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// Upstream's `set`, which returns `this` for chaining.
    #[napi]
    pub fn set<'a>(&self, this: This<'a>, key: JsKey, value: JsKey) -> This<'a> {
        self.inner.borrow_mut().set(key, value);

        this
    }

    #[napi(js_name = "delete")]
    pub fn delete(&self, key: JsKey) -> bool {
        self.inner.borrow_mut().delete(&key).is_some()
    }

    /// `undefined` for a missing key, not `null` — napi's `Option` renders
    /// `None` as the latter, and `deepStrictEqual(map.get('missing'),
    /// undefined)` would fail against it. `Either<_, Undefined>` is the same
    /// fix `Stack`/`Queue`'s `pop`/`peek` use.
    #[napi]
    pub fn get(&self, key: JsKey) -> Either<JsKey, Undefined> {
        into_either(self.inner.borrow().get(&key).cloned())
    }

    #[napi]
    pub fn has(&self, key: JsKey) -> bool {
        self.inner.borrow().has(&key)
    }

    /// Upstream's `forEach`, delegated to `Map.prototype.forEach.apply(this.items,
    /// arguments)`. The third callback argument is upstream's inner `Map`,
    /// which has no equivalent here — see `docs/modules/default-map.md` for
    /// the same divergence, made once for the whole T3 family — so this
    /// bridge object is passed instead, exactly as `DefaultMap::for_each`
    /// does.
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(JsKey, JsKey, Object)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        walk(&self.inner, items, this, callback, scope)
    }

    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsBiMap>) -> Result<JsBiMapEntries> {
        Ok(JsBiMapEntries {
            cursor: CellMapCursor::open(this.share_with(env, |map| Ok(&map.inner))?, items),
        })
    }

    #[napi]
    pub fn keys(&self, env: Env, this: Reference<JsBiMap>) -> Result<JsBiMapKeys> {
        Ok(JsBiMapKeys {
            cursor: CellMapCursor::open(this.share_with(env, |map| Ok(&map.inner))?, items),
        })
    }

    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsBiMap>) -> Result<JsBiMapValues> {
        Ok(JsBiMapValues {
            cursor: CellMapCursor::open(this.share_with(env, |map| Ok(&map.inner))?, items),
        })
    }

    /// `map.inverse` — a live companion object over the *same* underlying
    /// `RefCell`. See the module docs.
    #[napi(getter)]
    pub fn inverse(&self, env: Env, this: Reference<JsBiMap>) -> Result<JsBiMapInverse> {
        Ok(JsBiMapInverse {
            source: this.share_with(env, |map| Ok(&map.inner))?,
        })
    }

    /// `BiMap.from(iterable)`: `forEach(iterable, function(value, key) {
    /// bimap.set(key, value); })`.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown) -> Result<Self> {
        let pairs = collect_pairs(&env, iterable)?;
        let map = Self::new();

        {
            let mut inner = map.inner.borrow_mut();

            for (key, value) in pairs {
                inner.set(key, value);
            }
        }

        Ok(map)
    }
}

impl Default for JsBiMap {
    fn default() -> Self {
        Self::new()
    }
}

/// The live companion `map.inverse` hands out.
///
/// Reads and writes the exact same [`Core`] as the [`JsBiMap`] it was made
/// from, through the shared `RefCell`. Every method here is
/// `InverseMap.prototype.<name>`, upstream's *same function* as `BiMap`'s,
/// called with `items`/`inverse` swapped.
#[napi(js_name = "BiMapInverse")]
pub struct JsBiMapInverse {
    source: SharedReference<JsBiMap, &'static RefCell<Core>>,
}

#[napi]
impl JsBiMapInverse {
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.source.borrow().inverse_size() as u32
    }

    /// Same upstream function as `BiMap::clear`, called with `this` being the
    /// inverse view: both `this.items` and `this.inverse.items` are cleared
    /// regardless of which view called it, but only `this.size` (i.e.
    /// `inverse.size` from the outer `BiMap`'s perspective) is reset — B-120.
    /// `Core::clear_reverse` is the direction-aware half of that; calling
    /// `Core::clear` here would zero the wrong counter.
    #[napi]
    pub fn clear(&self) {
        self.source.borrow_mut().clear_reverse();
    }

    #[napi]
    pub fn set<'a>(&self, this: This<'a>, value: JsKey, key: JsKey) -> This<'a> {
        self.source.borrow_mut().set_reverse(value, key);

        this
    }

    #[napi(js_name = "delete")]
    pub fn delete(&self, value: JsKey) -> bool {
        self.source.borrow_mut().delete_reverse(&value).is_some()
    }

    #[napi]
    pub fn get(&self, value: JsKey) -> Either<JsKey, Undefined> {
        into_either(self.source.borrow().get_reverse(&value).cloned())
    }

    #[napi]
    pub fn has(&self, value: JsKey) -> bool {
        self.source.borrow().has_reverse(&value)
    }

    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(JsKey, JsKey, Object)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        walk(*self.source, inverse_items, this, callback, scope)
    }

    #[napi]
    pub fn entries(
        &self,
        env: Env,
        this: Reference<JsBiMapInverse>,
    ) -> Result<JsBiMapInverseEntries> {
        Ok(JsBiMapInverseEntries {
            cursor: CellMapCursor::open(
                this.share_with(env, |inv| Ok(*inv.source))?,
                inverse_items,
            ),
        })
    }

    #[napi]
    pub fn keys(&self, env: Env, this: Reference<JsBiMapInverse>) -> Result<JsBiMapInverseKeys> {
        Ok(JsBiMapInverseKeys {
            cursor: CellMapCursor::open(
                this.share_with(env, |inv| Ok(*inv.source))?,
                inverse_items,
            ),
        })
    }

    #[napi]
    pub fn values(
        &self,
        env: Env,
        this: Reference<JsBiMapInverse>,
    ) -> Result<JsBiMapInverseValues> {
        Ok(JsBiMapInverseValues {
            cursor: CellMapCursor::open(
                this.share_with(env, |inv| Ok(*inv.source))?,
                inverse_items,
            ),
        })
    }
}

/// The `forEach` body shared by [`JsBiMap`] and [`JsBiMapInverse`].
///
/// `project` picks which of the two `OrderedMap`s to walk; everything else —
/// the per-step borrow, the `scope` divergence — is identical, because
/// upstream's `forEach` is the same `Map.prototype.forEach.apply` call in
/// both cases.
fn walk(
    inner: &RefCell<Core>,
    project: fn(&Core) -> &OrderedMap<JsKey, JsKey>,
    this: This,
    callback: Function<FnArgs<(JsKey, JsKey, Object)>, Unknown>,
    scope: Option<Unknown>,
) -> Result<()> {
    let mut cursor = mnemonist_core::map::MapCursor::open();

    let mut step = || {
        let borrowed = inner.borrow();

        cursor
            .step(project(&borrowed))
            .map(|(key, value)| (key.clone(), value.clone()))
    };

    while let Some((key, value)) = step() {
        let args = FnArgs::from((value, key, this.object));

        match &scope {
            Some(scope) => callback.apply(*scope, args)?,
            None => callback.apply(this, args)?,
        };
    }

    Ok(())
}

/// Everything [`crate::foreach::for_each`] would visit, as `(key, value)`
/// pairs already classified into [`JsKey`] — the shape `BiMap.from`'s
/// collector needs.
///
/// A local, minimal collector rather than a change to
/// [`crate::foreach::collect`]: that helper returns `Vec<JsSlot>` for a
/// single-value walk (`Stack`/`Queue`'s `.from`), and `BiMap.from` needs both
/// the value *and* the key the dispatch hands over —
/// `forEach(iterable, function(value, key) { bimap.set(key, value); })`. It
/// still runs the same five-branch `for_each` dispatch underneath, so
/// `BiMap.from(new Map(...))` takes the exact branch-2 delegation path a real
/// `Map.forEach` would, and a non-primitive key or value is rejected exactly
/// as [`JsKey::from_unknown`] rejects one anywhere else.
fn collect_pairs(env: &Env, iterable: Unknown) -> Result<Vec<(JsKey, JsKey)>> {
    let sink = Rc::new(RefCell::new(Vec::<(JsKey, JsKey)>::new()));
    let collected = Rc::clone(&sink);

    // The declared argument type is one `Unknown`, matching
    // `foreach::collect`: nothing enforces arity on the JS side, and
    // `FunctionCallContext` reads whichever of `value`/`key` actually exist.
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

            let key = JsKey::from_unknown(&key)?;
            let value = JsKey::from_unknown(&value)?;

            collected.borrow_mut().push((key, value));

            Ok(())
        })?;

    // SAFETY: `collector` is a JS function this call just created.
    let collector = unsafe { Unknown::from_raw_unchecked(env.raw(), collector.raw()) };

    foreach::for_each(env, iterable, collector)?;

    let pairs = std::mem::take(&mut *sink.borrow_mut());

    Ok(pairs)
}

/// One `[key, value]` pair, as `Map.prototype.entries` yields it.
///
/// A dedicated type because napi has no tuple-to-array conversion and the
/// array has to be built with the env that only exists at conversion time —
/// same reasoning as `default_map::Pair`, kept separate because this one's
/// two fields are both `JsKey` rather than a `JsKey` and a `Loaned`.
pub struct Pair(JsKey, JsKey);

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

macro_rules! map_iterators {
    ($owner:ty, $entries:ident, $keys:ident, $values:ident, $entries_name:literal, $keys_name:literal, $values_name:literal) => {
        #[napi(iterator, js_name = $entries_name)]
        pub struct $entries {
            cursor: Cursor<$owner>,
        }

        impl Generator for $entries {
            type Yield = Pair;
            type Next = ();
            type Return = ();

            fn next(&mut self, _value: Option<()>) -> Option<Pair> {
                self.cursor
                    .step(|key, value| Pair(key.clone(), value.clone()))
            }

            fn complete(&mut self, _value: Option<()>) -> Option<Pair> {
                None
            }
        }

        #[napi(iterator, js_name = $keys_name)]
        pub struct $keys {
            cursor: Cursor<$owner>,
        }

        impl Generator for $keys {
            type Yield = JsKey;
            type Next = ();
            type Return = ();

            fn next(&mut self, _value: Option<()>) -> Option<JsKey> {
                self.cursor.step(|key, _| key.clone())
            }

            fn complete(&mut self, _value: Option<()>) -> Option<JsKey> {
                None
            }
        }

        #[napi(iterator, js_name = $values_name)]
        pub struct $values {
            cursor: Cursor<$owner>,
        }

        impl Generator for $values {
            type Yield = JsKey;
            type Next = ();
            type Return = ();

            fn next(&mut self, _value: Option<()>) -> Option<JsKey> {
                self.cursor.step(|_, value| value.clone())
            }

            fn complete(&mut self, _value: Option<()>) -> Option<JsKey> {
                None
            }
        }
    };
}

map_iterators!(
    JsBiMap,
    JsBiMapEntries,
    JsBiMapKeys,
    JsBiMapValues,
    "BiMapEntries",
    "BiMapKeys",
    "BiMapValues"
);

map_iterators!(
    JsBiMapInverse,
    JsBiMapInverseEntries,
    JsBiMapInverseKeys,
    JsBiMapInverseValues,
    "BiMapInverseEntries",
    "BiMapInverseKeys",
    "BiMapInverseValues"
);

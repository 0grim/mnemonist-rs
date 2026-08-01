//! JS bridge for [`mnemonist_core::structures::static_interval_tree`].
//!
//! Thin translation only; every behavioural decision lives in the core crate.
//! Three adaptations are worth knowing about.
//!
//! 1. **`getters` are resolved once, at construction, and cached.** Upstream
//!    invokes `startGetter`/`endGetter` (or the default `interval[0]`/
//!    `interval[1]`) afresh on every visited node, in both query methods. The
//!    core crate takes pre-resolved `(start, end)` bounds instead (see its
//!    module docs for why that is observationally identical for any pure
//!    getter, which is the only kind either upstream module ever ships or
//!    the original suite ever supplies). This bridge is where the getters
//!    actually run: once per stored interval at construction, and once more
//!    per call for `intervalsOverlappingInterval`'s own query argument, which
//!    is not one of the stored intervals and so cannot have been resolved in
//!    advance.
//! 2. **`tree` and `augmentations` are not exposed.** They are public typed
//!    arrays upstream; napi can only hand out a copy, which would silently
//!    break the write-through a real caller could otherwise rely on. Same
//!    call `sparse-set`'s and `sparse-map`'s bridges make for `dense`/
//!    `sparse`/`vals`. They are public on the core type and the differential
//!    fuzzer compares both slot for slot.
//! 3. **`StaticIntervalTree.from`'s own iterable resolution is upstream's,
//!    not `obliterator/foreach`'s.** `Array.from(iterable)` is called
//!    through the real global — not `obliterator/foreach`'s five-branch
//!    dispatch, which most of this port's other `.from()` statics use. The
//!    two are **not interchangeable**: a `Map` owns a `.forEach`, which
//!    `obliterator/foreach` prefers over its `Symbol.iterator`, while
//!    `Array.from` always prefers `Symbol.iterator` when one exists. A `Map`'s
//!    default iterator yields `[key, value]` pairs — exactly the `[start,
//!    end]` shape this module wants — while its own `.forEach` invokes a
//!    callback with `(value, key, map)`. Routing a `Map` through
//!    `obliterator/foreach` here would silently swap `start` and `end`.

use napi::bindgen_prelude::*;
use napi_derive::napi;

use mnemonist_core::structures::static_interval_tree::{
    Error as CoreError, StaticIntervalTree as CoreTree,
};

use crate::foreach::{self, coerce_to_object, to_number};
use crate::iterables;
use crate::js_slot::JsSlot;

/// A resolved `startGetter`/`endGetter` pair, or `None` for the default
/// `interval[0]`/`interval[1]` access.
type Getters = Option<(
    FunctionRef<Unknown<'static>, Unknown<'static>>,
    FunctionRef<Unknown<'static>, Unknown<'static>>,
)>;

/// A static (build-once) interval tree over arbitrary JavaScript values.
#[napi(js_name = "StaticIntervalTree")]
pub struct JsStaticIntervalTree {
    inner: CoreTree<JsSlot>,
    /// Cached so `intervalsOverlappingInterval`'s own query argument -- never
    /// one of the stored intervals, and so never resolved at construction --
    /// can be resolved the same way the stored intervals were.
    getters: Getters,
}

#[napi]
impl JsStaticIntervalTree {
    /// `new StaticIntervalTree(intervals, getters)`.
    ///
    /// Upstream's raw constructor assumes `intervals` already behaves like an
    /// array (`.length` plus indexed reads) -- guaranteed by `.from()`, the
    /// only path the original suite ever takes. This constructor accepts the
    /// same array-like shape directly; a value that is not one degrades to
    /// [`crate::iterables::array_like_values`]'s own reading of an absent
    /// `length`, rather than to an untested upstream crash path.
    #[napi(constructor)]
    pub fn new(env: Env, intervals: Unknown, getters: Option<Unknown>) -> Result<Self> {
        let slots = iterables::array_like_values(&env, &intervals)?;

        Self::build(&env, slots, getters)
    }

    /// `StaticIntervalTree.from(iterable, getters)`.
    ///
    /// See the module docs: the array-like-or-`Array.from` resolution is
    /// upstream's own, not `obliterator/foreach`'s.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown, getters: Option<Unknown>) -> Result<Self> {
        let slots = if iterables::is_array_like(&env, &iterable)? {
            iterables::array_like_values(&env, &iterable)?
        } else {
            let materialized = array_from(&env, iterable)?;

            iterables::array_like_values(&env, &materialized)?
        };

        Self::build(&env, slots, getters)
    }

    /// `this.size`.
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.size() as u32
    }

    /// `this.height`.
    #[napi(getter)]
    pub fn height(&self) -> u32 {
        self.inner.height() as u32
    }

    /// `#.intervalsContainingPoint(point)`.
    #[napi]
    pub fn intervals_containing_point(&self, point: f64) -> Result<Vec<JsSlot>> {
        self.inner.intervals_containing_point(point).map_err(raise)
    }

    /// `#.intervalsOverlappingInterval(interval)`.
    ///
    /// `interval` is resolved through the same getters (or the same default
    /// `[0]`/`[1]` access) the stored intervals were, per the module docs.
    #[napi]
    pub fn intervals_overlapping_interval(
        &self,
        env: Env,
        interval: Unknown,
    ) -> Result<Vec<JsSlot>> {
        let (start, end) = bounds_of(&env, interval, &self.getters)?;

        self.inner
            .intervals_overlapping_interval(start, end)
            .map_err(raise)
    }

    fn build(env: &Env, slots: Vec<JsSlot>, getters: Option<Unknown>) -> Result<Self> {
        let getters = resolve_getters(env, getters)?;
        let mut bounds = Vec::with_capacity(slots.len());

        for slot in &slots {
            bounds.push(bounds_of(env, slot.get(env)?, &getters)?);
        }

        let inner = CoreTree::new(slots, bounds).map_err(raise)?;

        Ok(Self { inner, getters })
    }
}

/// The real global `Array.from(iterable)` -- see the module docs for why
/// this cannot be `obliterator/foreach`'s dispatch instead.
fn array_from<'env>(env: &'env Env, iterable: Unknown<'env>) -> Result<Unknown<'env>> {
    let global = env.get_global()?;
    let array_ctor: Object = global.get_named_property_unchecked("Array")?;
    let from: Function<'_, Unknown, Unknown> = array_ctor.get_named_property("from")?;

    from.apply(array_ctor, iterable)
}

/// `Array.isArray(getters) ? [getters[0], getters[1]] : [null, null]`, with
/// the two functions promoted to refs so they outlive this call -- they are
/// invoked again later, by `intervalsOverlappingInterval`.
///
/// Upstream never checks that `getters[0]`/`getters[1]` are callable at
/// assignment time; a non-function throws only when actually invoked. Cast
/// unchecked here for the same reason `crate::foreach`'s own dispatch does.
fn resolve_getters(env: &Env, getters: Option<Unknown>) -> Result<Getters> {
    let Some(getters) = getters else {
        return Ok(None);
    };

    if !foreach::is_array(env, &getters)? {
        return Ok(None);
    }

    let pair = coerce_to_object(env, &getters)?;
    let start: Unknown = pair.get_element(0)?;
    let end: Unknown = pair.get_element(1)?;

    // SAFETY: neither cast is dereferenced as a function until `.call()`
    // runs, at which point a non-function value raises napi's own error --
    // matching upstream calling an uncallable `startGetter` directly.
    let start: Function<'_, Unknown, Unknown> = unsafe { start.cast()? };
    let end: Function<'_, Unknown, Unknown> = unsafe { end.cast()? };

    Ok(Some((start.create_ref()?, end.create_ref()?)))
}

/// `startGetter ? startGetter(value) : value[0]`, and likewise for `end`.
fn bounds_of(env: &Env, value: Unknown, getters: &Getters) -> Result<(f64, f64)> {
    match getters {
        Some((start, end)) => {
            let start = start.borrow_back(env)?.call(value)?;
            let end = end.borrow_back(env)?.call(value)?;

            Ok((to_number(env, &start)?, to_number(env, &end)?))
        }
        None => {
            let object = coerce_to_object(env, &value)?;
            let start: Unknown = object.get_element(0)?;
            let end: Unknown = object.get_element(1)?;

            Ok((to_number(env, &start)?, to_number(env, &end)?))
        }
    }
}

fn raise(error: CoreError) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

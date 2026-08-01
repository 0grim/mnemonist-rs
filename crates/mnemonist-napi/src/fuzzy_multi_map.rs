//! JS bridge for [`mnemonist_core::structures::fuzzy_multi_map`].
//!
//! Inherits `fuzzy-map`'s hash-function plumbing (`HashFn`/`resolve_hash`/
//! `apply_hash`, duplicated locally for the same reason `bi_map`/`multi_map`
//! each keep their own `collect_pairs`) and `multi-map`'s `Container`
//! resolution and rendering. What is new here is **the value type**.
//!
//! # Why bucket values are `Rc<RefCell<Retained>>`, not `Retained`
//!
//! `test/fuzzy-multi-map.js`'s `Set`-container case stores plain **objects**
//! as values and depends on `Set`-kind deduplication by JavaScript identity
//! (SameValueZero) — see `mnemonist_core::structures::multi_map`'s module
//! docs for why that is a fallible, `Env`-aware equality callback
//! ([`same_value_zero`], below) rather than a `Hash`/`Eq` bound.
//!
//! [`crate::structures::multi_map::FlattenedCursor`] (which this module's
//! `values()`/`forEach` reuse via `MultiMap::cursor`) snapshots a bucket's
//! contents by **cloning** them, and a bare [`Retained`] cannot be cloned at
//! all — it owns exactly one `napi_ref`, and a `#[derive(Clone)]` would either
//! not compile or (worse) silently double-free. `Rc<RefCell<Retained>>`
//! clones cheaply (an `Rc` bump, never a second `napi_ref`) while keeping
//! exactly one real owned reference underneath every clone, and the
//! `RefCell` gives [`release`](Retained::release) — which needs `&mut self`
//! — a way in through a shared handle.
//!
//! One consequence is stated rather than hidden: if a caller keeps a
//! `values()`/`entries()`-style iterator open across a `clear()` (or the map
//! being finalized), the iterator's own still-held clone observes the
//! now-released, inert `Retained` if it is read afterwards — `test/
//! fuzzy-multi-map.js` never does this, and it is the same class of
//! documented gap `multi_map`'s own flattened cursor states for a
//! same-bucket mutation mid-walk.

use std::cell::RefCell;
use std::rc::Rc;

use mnemonist_core::structures::fuzzy_multi_map::FuzzyMultiMap as CoreMap;
use mnemonist_core::structures::multi_map::{Bucket, ContainerKind};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::foreach;
use crate::js_key::JsKey;
use crate::js_value::{JsPrimitive, Loaned, Retained};

/// One stored item: shared ownership of exactly one retained JS value. See
/// the module docs.
type Item = Rc<RefCell<Retained>>;

/// The map as core sees it: keys are hashed `JsKey`s, values are arbitrary
/// retained JS values.
type Core = CoreMap<JsKey, Item>;

/// A stored hash function, or `None` for upstream's `identity` substitution
/// — identical to `crate::fuzzy_map`'s `HashFn`, duplicated locally.
type HashFn = FunctionRef<Unknown<'static>, Unknown<'static>>;

const INVALID_HASH: &str = "mnemonist/FuzzyMultiMap.constructor: invalid hash function given.";

/// SameValueZero for two retained values — `Set`-kind membership's equality.
///
/// Primitives compare by value (with the `NaN`-equals-`NaN`, `-0`-equals-`+0`
/// folding SameValueZero requires); anything else (object, function, symbol,
/// bigint) compares by identity via `napi_strict_equals`, which for those
/// kinds *is* SameValueZero (the two relations differ only on `NaN`, which is
/// always a primitive).
fn same_value_zero(env: &Env, a: &Item, b: &Item) -> Result<bool> {
    let a = a.borrow();
    let b = b.borrow();

    match (&*a, &*b) {
        (Retained::Primitive(left), Retained::Primitive(right)) => {
            Ok(primitive_same_value_zero(left, right))
        }
        (Retained::Reference(left), Retained::Reference(right)) => {
            let left = resolve_reference(env, *left)?;
            let right = resolve_reference(env, *right)?;

            env.strict_equals(left, right)
        }
        // A primitive and a reference are never SameValueZero: different
        // fundamental kinds.
        _ => Ok(false),
    }
}

fn primitive_same_value_zero(a: &JsPrimitive, b: &JsPrimitive) -> bool {
    match (a, b) {
        (JsPrimitive::Null, JsPrimitive::Null) => true,
        (JsPrimitive::Bool(x), JsPrimitive::Bool(y)) => x == y,
        (JsPrimitive::Number(x), JsPrimitive::Number(y)) => (x.is_nan() && y.is_nan()) || x == y,
        (JsPrimitive::String(x), JsPrimitive::String(y)) => x == y,
        _ => false,
    }
}

/// A live handle to whatever a `napi_ref` currently points at.
fn resolve_reference<'env>(env: &'env Env, reference: sys::napi_ref) -> Result<Unknown<'env>> {
    let mut result = std::ptr::null_mut();

    napi::check_status!(
        unsafe { sys::napi_get_reference_value(env.raw(), reference, &mut result) },
        "mnemonist-rs: failed to read back a retained value"
    )?;

    // SAFETY: `napi_get_reference_value` produced a handle in `env`'s scope.
    Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), result) })
}

/// A bucket, rendered as a plain `Array` ([`ContainerKind::List`]) or a real
/// `Set` ([`ContainerKind::Set`]) of the actual retained items — see
/// `mnemonist_napi::multi_map`'s `Rendered` for the `List`/`Set` half of this;
/// the difference here is every element is loaned back through
/// [`Retained::loan`] rather than converted directly.
pub struct Rendered {
    kind: ContainerKind,
    loans: Vec<Loaned>,
}

impl Rendered {
    fn new(bucket: &Bucket<Item>) -> Self {
        Self {
            kind: bucket.kind(),
            loans: bucket
                .values()
                .iter()
                .map(|item| item.borrow().loan())
                .collect(),
        }
    }
}

impl ToNapiValue for Rendered {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let js_env = Env::from_raw(env);
        let array = Array::from_vec(&js_env, val.loans)?;

        match val.kind {
            ContainerKind::List => Ok(unsafe { ToNapiValue::to_napi_value(env, array)? }),
            ContainerKind::Set => {
                let global = js_env.get_global()?;
                let constructor: Function<'_, Array, Unknown> =
                    global.get_named_property_unchecked("Set")?;
                let instance = constructor.new_instance(array)?;

                Ok(instance.raw())
            }
        }
    }
}

#[napi(js_name = "FuzzyMultiMap", custom_finalize)]
pub struct JsFuzzyMultiMap {
    inner: RefCell<Core>,
    write_hash: Option<HashFn>,
    read_hash: Option<HashFn>,
}

#[napi]
impl JsFuzzyMultiMap {
    /// `new FuzzyMultiMap(descriptor, Container)`.
    #[napi(constructor)]
    pub fn new(env: Env, descriptor: Option<Unknown>, container: Option<Unknown>) -> Result<Self> {
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
            inner: RefCell::new(Core::new(resolve_kind(&env, container)?)),
            write_hash: resolve_hash(&env, write_candidate)?,
            read_hash: resolve_hash(&env, read_candidate)?,
        })
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    #[napi(getter)]
    pub fn dimension(&self) -> u32 {
        self.inner.borrow().dimension() as u32
    }

    #[napi]
    pub fn clear(&self, env: Env) -> Result<()> {
        let mut inner = self.inner.borrow_mut();

        for item in inner.values_mut() {
            item.borrow_mut().release(&env)?;
        }

        inner.clear();

        Ok(())
    }

    /// Upstream's `add`: hash the *item itself* with `writeHashFunction`.
    #[napi]
    pub fn add<'a>(&self, this: This<'a>, env: Env, item: Unknown) -> Result<This<'a>> {
        let key = apply_hash(&env, &self.write_hash, item)?;

        self.store(&env, key, item)?;

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

        self.store(&env, hashed, item)?;

        Ok(this)
    }

    /// Upstream's `get`: hash `key` with `readHashFunction`.
    #[napi]
    pub fn get(&self, env: Env, key: Unknown) -> Result<Either<Rendered, Undefined>> {
        let hashed = apply_hash(&env, &self.read_hash, key)?;

        Ok(match self.inner.borrow().get(&hashed) {
            Some(bucket) => Either::A(Rendered::new(bucket)),
            None => Either::B(()),
        })
    }

    /// Upstream's `has`: hash `key` with `readHashFunction`.
    #[napi]
    pub fn has(&self, env: Env, key: Unknown) -> Result<bool> {
        let hashed = apply_hash(&env, &self.read_hash, key)?;

        Ok(self.inner.borrow().has(&hashed))
    }

    /// Upstream's `forEach`: `this.items.forEach(function(value) {
    /// callback.call(scope, value, value); })` — `this.items`'s own `forEach`
    /// is `MultiMap`'s flattened walk, and only the value half survives here.
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(Loaned, Loaned)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let mut cursor = self.inner.borrow().items().cursor();

        let mut step = || {
            let inner = self.inner.borrow();

            cursor.step(inner.items().items()).map(|(_, value)| value)
        };

        while let Some(item) = step() {
            // Two independent loans of the same reference: `Loaned`'s own
            // conversion only *reads* the reference back
            // (`napi_get_reference_value`), never touches its count, so
            // taking it twice is exactly as safe as once. See `Loaned`'s docs.
            let args = FnArgs::from((item.borrow().loan(), item.borrow().loan()));

            match &scope {
                Some(scope) => callback.apply(*scope, args)?,
                None => callback.apply(this, args)?,
            };
        }

        Ok(())
    }

    /// Upstream's `values`, and its `Symbol.iterator` — `this.items.values()`
    /// flattened.
    #[napi]
    pub fn values(
        &self,
        env: Env,
        this: Reference<JsFuzzyMultiMap>,
    ) -> Result<JsFuzzyMultiMapValues> {
        Ok(JsFuzzyMultiMapValues {
            source: this.share_with(env, |map| Ok(&map.inner))?,
            state: mnemonist_core::structures::multi_map::FlattenedCursor::open(),
        })
    }

    /// `FuzzyMultiMap.from(iterable, descriptor, Container, useSet)`:
    /// `forEach(iterable, function(value, key) { if (useSet) map.set(key,
    /// value); else map.add(value); });`
    ///
    /// Upstream special-cases exactly three arguments with a boolean third:
    /// `if (arguments.length === 3) { if (typeof Container === 'boolean') {
    /// useSet = Container; Container = Array; } }` — `test/
    /// fuzzy-multi-map.js`'s own `FuzzyMultiMap.from(otherMap, readHash,
    /// true)` call relies on this to reach `useSet` at all. napi has no
    /// `arguments.length`, so this reproduces the one case that matters —
    /// a boolean where `Container` would be, with `useSet` itself absent —
    /// by the same shift.
    #[napi(factory)]
    pub fn from(
        env: Env,
        iterable: Unknown,
        descriptor: Option<Unknown>,
        container: Option<Unknown>,
        use_set: Option<bool>,
    ) -> Result<Self> {
        let (container, use_set) = match (container, use_set) {
            (Some(candidate), None) if candidate.get_type()? == ValueType::Boolean => {
                // SAFETY: the type check above is exactly `typeof ===
                // 'boolean'`.
                let flag: bool = unsafe { candidate.cast::<bool>()? };

                (None, Some(flag))
            }
            other => other,
        };

        let pairs = collect_pairs(&env, iterable)?;
        let map = Self::new(env, descriptor, container)?;
        let use_set = use_set.unwrap_or(false);

        for (value, key) in pairs {
            // `resolve` only borrows -- `value`/`key` still own their
            // retained slot afterwards, so the one actually stored below is
            // moved (not re-retained). See `store_retained`.
            if use_set {
                let key_live = resolve(&env, &key)?;
                let hashed = apply_hash(&env, &map.write_hash, key_live)?;

                if let Some(retained) = value {
                    map.store_retained(&env, hashed, retained)?;
                }
            } else {
                let value_live = resolve(&env, &value)?;
                let hashed = apply_hash(&env, &map.write_hash, value_live)?;

                if let Some(retained) = value {
                    map.store_retained(&env, hashed, retained)?;
                }
            }
        }

        Ok(map)
    }

    /// Retain `item` fresh and store it — the path `add`/`set` use, since
    /// their `item` argument is a live value from the current call that has
    /// never been retained before.
    fn store(&self, env: &Env, key: JsKey, item: Unknown) -> Result<()> {
        self.store_retained(env, key, Retained::new(&item)?)
    }

    /// Store an **already-retained** value directly, without retaining it a
    /// second time. `.from`'s collector (see [`collect_pairs`]) retains
    /// every value up front, before any hash function runs; storing through
    /// [`JsFuzzyMultiMap::store`] there would create a *second* independent
    /// `napi_ref` for the same JS object while the first one, still held by
    /// the now-discarded collected slot, is dropped without
    /// [`Retained::release`] -- the leak `Retained`'s own `Drop` warns
    /// about. This is that mistake, fixed: exactly one retained reference
    /// per stored item.
    fn store_retained(&self, env: &Env, key: JsKey, retained: Retained) -> Result<()> {
        let item: Item = Rc::new(RefCell::new(retained));

        let rejected = self
            .inner
            .borrow_mut()
            .set_with(key, item, |a, b| same_value_zero(env, a, b))?;

        // `Set`-kind membership already had an equal member: the candidate
        // was never stored, so it must be released here or it leaks --
        // there is no other slot left holding it. `Rc::try_unwrap` succeeds
        // because nothing else has cloned this particular candidate yet (it
        // was only just constructed above); if some future change ever made
        // that untrue, releasing a still-shared item would be the wrong
        // fix anyway, so this deliberately does not paper over that with
        // `Rc::strong_count` gymnastics.
        if let Some(item) = rejected {
            if let Ok(cell) = Rc::try_unwrap(item) {
                cell.into_inner().release(env)?;
            }
        }

        Ok(())
    }
}

impl ObjectFinalize for JsFuzzyMultiMap {
    fn finalize(self, env: Env) -> Result<()> {
        for item in self.inner.borrow_mut().values_mut() {
            item.borrow_mut().release(&env)?;
        }

        Ok(())
    }
}

/// Resolve `Container` to a [`ContainerKind`] — identical to
/// `multi_map::resolve_kind`, duplicated locally for the same reason every
/// other per-bridge helper in this crate is.
fn resolve_kind(env: &Env, container: Option<Unknown>) -> Result<ContainerKind> {
    let Some(container) = container else {
        return Ok(ContainerKind::List);
    };

    if container.get_type()? == ValueType::Undefined {
        return Ok(ContainerKind::List);
    }

    let global = env.get_global()?;
    let set_ctor: Unknown = global.get_named_property_unchecked("Set")?;

    if env.strict_equals(container, set_ctor)? {
        Ok(ContainerKind::Set)
    } else {
        Ok(ContainerKind::List)
    }
}

/// Resolve one descriptor slot to a stored hash function — identical to
/// `crate::fuzzy_map`'s `resolve_hash`, duplicated locally.
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

/// Hash `value`: `identity` when `hash` is `None`, otherwise the stored JS
/// function's return value.
fn apply_hash(env: &Env, hash: &Option<HashFn>, value: Unknown) -> Result<JsKey> {
    match hash {
        None => JsKey::from_unknown(&value),
        Some(function) => {
            let callable = function.borrow_back(env)?;
            // SAFETY: as in `crate::fuzzy_map::apply_hash`.
            let argument = unsafe { Unknown::from_raw_unchecked(env.raw(), value.raw()) };
            let result = callable.call(argument)?;

            JsKey::from_unknown(&result)
        }
    }
}

/// A handle in `env`'s current scope for a value collected earlier in the
/// same call — the inverse of retaining, for `.from`'s two-phase collect.
fn resolve<'env>(env: &'env Env, slot: &Option<Retained>) -> Result<Unknown<'env>> {
    let Some(retained) = slot else {
        return foreach::undefined(env);
    };

    // SAFETY: `to_napi_value` produces a handle in `env`'s current scope.
    let raw = unsafe { ToNapiValue::to_napi_value(env.raw(), retained.loan())? };

    Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), raw) })
}

/// Every `(value, key)` pair the dispatch visits, collected **before** any
/// hashing — same reason as `crate::fuzzy_map::collect_pairs`: the collector
/// closure must be `'static`, and the hash functions are not.
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

fn retain(value: &Unknown) -> Result<Option<Retained>> {
    match value.get_type()? {
        ValueType::Undefined => Ok(None),
        _ => Ok(Some(Retained::new(value)?)),
    }
}

/// `.values()`'s cursor: the flattened walk over `this.items`, one stored
/// item per step.
#[napi(iterator, js_name = "FuzzyMultiMapValues")]
pub struct JsFuzzyMultiMapValues {
    source: SharedReference<JsFuzzyMultiMap, &'static RefCell<Core>>,
    state: mnemonist_core::structures::multi_map::FlattenedCursor<JsKey, Item>,
}

impl Generator for JsFuzzyMultiMapValues {
    type Yield = Loaned;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Loaned> {
        let borrowed = self.source.borrow();

        self.state
            .step(borrowed.items().items())
            .map(|(_, value)| value.borrow().loan())
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Loaned> {
        None
    }
}

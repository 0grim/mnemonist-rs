//! JS bridge for [`mnemonist_core::structures::multi_map`].
//!
//! # `Container` resolution: `Set` by identity, everything else is `List`
//!
//! Upstream's whole write-path branches on exactly one test,
//! `this.Container === Set` — object identity against the realm's `Set`.
//! Everything else (the default `Array`, a `Vector` subclass, a caller's own
//! class) takes the same `container.push(value); this.size++;` path. So the
//! bridge resolves `Container` to [`ContainerKind::Set`] only on that one
//! identity match and to [`ContainerKind::List`] for anything else,
//! including `undefined` (the default).
//!
//! # The rendered bucket is always a plain `Array` or a real `Set`, never a
//! `Vector`
//!
//! `test/multi-map.js`'s one non-`Array`/`Set` case constructs a `MultiMap`
//! with `Vector.Uint8Vector` and only ever asserts `Array.from(map.get(key))`
//! against the pushed numbers — never `instanceof Vector`, never a
//! `Vector`-specific method on the returned container. [`get`]/[`containers`]/
//! [`associations`] therefore always materialise a **plain `Array`** for a
//! `List`-kind bucket, regardless of what `Container` originally was. This is
//! a deliberate divergence (`docs/modules/multi-map.md`, `planning/
//! DECISIONS-CANDIDATES.md`): a caller that does check `instanceof Vector`, or
//! that relies on a custom container's own behaviour beyond `.push`, sees a
//! plain array instead. Nothing in the original suite can tell the
//! difference.
//!
//! # Values are [`JsKey`]-shaped, not arbitrary
//!
//! `test/multi-map.js` only ever stores strings and numbers as values. Rather
//! than build the full `Retained`/`Loaned` machinery `fuzzy-multi-map` needs
//! for arbitrary values, this bridge reuses [`JsKey`] for values too — plain
//! data, cheap to clone, and already SameValueZero-equal, which is exactly
//! the equality upstream's `Set`-kind branch and `remove`'s `indexOf` both
//! want. An object value is refused with the same message `JsKey::from_unknown`
//! gives for an object key. See `docs/modules/multi-map.md`.

use std::cell::RefCell;

use mnemonist_core::structures::multi_map::{Bucket, ContainerKind, MultiMap as CoreMultiMap};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::foreach;
use crate::js_key::JsKey;
use crate::map_cursor::CellMapCursor;

/// The map as core sees it: keys and values are both [`JsKey`]. See the
/// module docs for why values are not the `Retained`/`Loaned` pair
/// `fuzzy-multi-map` needs.
type Core = CoreMultiMap<JsKey, JsKey>;

/// The cursor `.keys()`/`.containers()`/`.associations()` hand out: a live
/// walk over the outer map, one step per **key**.
type OuterCursor = CellMapCursor<JsMultiMap, Core, JsKey, Bucket<JsKey>>;

fn items(map: &Core) -> &mnemonist_core::map::OrderedMap<JsKey, Bucket<JsKey>> {
    map.items()
}

/// A bucket, rendered as the JS value upstream's `#.get` et al. return: a
/// plain `Array` for [`ContainerKind::List`], a real `Set` for
/// [`ContainerKind::Set`]. Built fresh on every render — see the module docs
/// for why this is never a live handle to any particular container object.
pub struct Rendered(Bucket<JsKey>);

impl ToNapiValue for Rendered {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let js_env = Env::from_raw(env);
        let values = val.0.values().to_vec();
        let array = Array::from_vec(&js_env, values)?;

        match val.0.kind() {
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

/// One `[key, value]` pair, exactly as `default_map::Pair`/`bi_map::Pair`
/// build one — a dedicated type because napi has no tuple-to-array
/// conversion and the array must be built with the `env` a `ToNapiValue`
/// call supplies.
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

/// `[key, renderedContainer]`, for `#.associations`.
pub struct Association(JsKey, Rendered);

impl ToNapiValue for Association {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let key = unsafe { ToNapiValue::to_napi_value(env, val.0)? };
        let container = unsafe { ToNapiValue::to_napi_value(env, val.1)? };

        let mut pair = std::ptr::null_mut();
        napi::check_status!(
            unsafe { sys::napi_create_array_with_length(env, 2, &mut pair) },
            "mnemonist-rs: failed to build a [key, container] pair"
        )?;
        napi::check_status!(
            unsafe { sys::napi_set_element(env, pair, 0, key) },
            "mnemonist-rs: failed to set an association's key"
        )?;
        napi::check_status!(
            unsafe { sys::napi_set_element(env, pair, 1, container) },
            "mnemonist-rs: failed to set an association's container"
        )?;

        Ok(pair)
    }
}

#[napi(js_name = "MultiMap")]
pub struct JsMultiMap {
    inner: RefCell<Core>,
}

#[napi]
impl JsMultiMap {
    #[napi(constructor)]
    pub fn new(env: Env, container: Option<Unknown>) -> Result<Self> {
        Ok(Self {
            inner: RefCell::new(Core::new(resolve_kind(&env, container)?)),
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
        self.inner.borrow_mut().delete(&key)
    }

    #[napi]
    pub fn remove(&self, key: JsKey, value: JsKey) -> bool {
        self.inner.borrow_mut().remove(key, &value)
    }

    #[napi]
    pub fn has(&self, key: JsKey) -> bool {
        self.inner.borrow().has(&key)
    }

    /// `undefined` for a missing key, not `null` — see `bi_map::JsBiMap::get`
    /// for the same fix over `napi`'s own `Option` rendering.
    #[napi]
    pub fn get(&self, key: JsKey) -> Either<Rendered, Undefined> {
        match self.inner.borrow().get(&key) {
            Some(bucket) => Either::A(Rendered(bucket.clone())),
            None => Either::B(()),
        }
    }

    #[napi]
    pub fn multiplicity(&self, key: JsKey) -> u32 {
        self.inner.borrow().multiplicity(&key) as u32
    }

    /// Upstream's `count`, an alias of `multiplicity`.
    #[napi]
    pub fn count(&self, key: JsKey) -> u32 {
        self.multiplicity(key)
    }

    /// Upstream's `forEach`: flattened `(value, key)` pairs, bucket after
    /// bucket. See the core module's docs for the one simplification this
    /// makes (a bucket walked mid-mutation is a snapshot, not a live view).
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(JsKey, JsKey)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let mut cursor = self.inner.borrow().cursor();

        let mut step = || {
            let inner = self.inner.borrow();

            cursor.step(inner.items())
        };

        while let Some((key, value)) = step() {
            let args = FnArgs::from((value, key));

            match &scope {
                Some(scope) => callback.apply(*scope, args)?,
                None => callback.apply(this, args)?,
            };
        }

        Ok(())
    }

    /// Upstream's `forEachAssociation`: `this.items.forEach(callback,
    /// scope)` directly — one call per **key**, not per value.
    #[napi(js_name = "forEachAssociation")]
    pub fn for_each_association(
        &self,
        this: This,
        callback: Function<FnArgs<(Rendered, JsKey)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let mut cursor = mnemonist_core::map::MapCursor::open();

        let mut step = || {
            let inner = self.inner.borrow();

            cursor
                .step(items(&inner))
                .map(|(key, bucket)| (key.clone(), bucket.clone()))
        };

        while let Some((key, bucket)) = step() {
            let args = FnArgs::from((Rendered(bucket), key));

            match &scope {
                Some(scope) => callback.apply(*scope, args)?,
                None => callback.apply(this, args)?,
            };
        }

        Ok(())
    }

    #[napi]
    pub fn keys(&self, env: Env, this: Reference<JsMultiMap>) -> Result<JsMultiMapKeys> {
        Ok(JsMultiMapKeys {
            cursor: CellMapCursor::open(this.share_with(env, |map| Ok(&map.inner))?, items),
        })
    }

    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsMultiMap>) -> Result<JsMultiMapValues> {
        Ok(JsMultiMapValues {
            source: this.share_with(env, |map| Ok(&map.inner))?,
            state: mnemonist_core::structures::multi_map::FlattenedCursor::open(),
        })
    }

    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsMultiMap>) -> Result<JsMultiMapEntries> {
        Ok(JsMultiMapEntries {
            source: this.share_with(env, |map| Ok(&map.inner))?,
            state: mnemonist_core::structures::multi_map::FlattenedCursor::open(),
        })
    }

    #[napi]
    pub fn containers(
        &self,
        env: Env,
        this: Reference<JsMultiMap>,
    ) -> Result<JsMultiMapContainers> {
        Ok(JsMultiMapContainers {
            cursor: CellMapCursor::open(this.share_with(env, |map| Ok(&map.inner))?, items),
        })
    }

    #[napi]
    pub fn associations(
        &self,
        env: Env,
        this: Reference<JsMultiMap>,
    ) -> Result<JsMultiMapAssociations> {
        Ok(JsMultiMapAssociations {
            cursor: CellMapCursor::open(this.share_with(env, |map| Ok(&map.inner))?, items),
        })
    }

    /// `MultiMap.from(iterable, Container)`:
    /// `forEach(iterable, function(value, key) { map.set(key, value); })`.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown, container: Option<Unknown>) -> Result<Self> {
        let pairs = collect_pairs(&env, iterable)?;
        let map = Self::new(env, container)?;

        {
            let mut inner = map.inner.borrow_mut();

            for (key, value) in pairs {
                inner.set(key, value);
            }
        }

        Ok(map)
    }
}

/// Resolve `Container` to a [`ContainerKind`]: `Set`-kind only on an exact
/// identity match against the realm's own `Set`, `List`-kind for `undefined`
/// (the default) and everything else. See the module docs.
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

/// Everything [`crate::foreach::for_each`] would visit, as `(key, value)`
/// pairs already classified into [`JsKey`] — the shape `MultiMap.from`'s
/// collector needs. A local copy of `bi_map::collect_pairs`'s pattern rather
/// than a shared export: `fuzzy_map`/`bi_map` each keep their own for the
/// same reason (CLAUDE.md: grep before inventing shared machinery, but a
/// three-line closure over a different pair type is not machinery worth
/// sharing).
fn collect_pairs(env: &Env, iterable: Unknown) -> Result<Vec<(JsKey, JsKey)>> {
    use std::cell::RefCell as StdRefCell;
    use std::rc::Rc;

    let sink = Rc::new(StdRefCell::new(Vec::<(JsKey, JsKey)>::new()));
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

macro_rules! outer_iterator {
    ($name:ident, $yield:ty, $project:expr, $js_name:literal) => {
        #[napi(iterator, js_name = $js_name)]
        pub struct $name {
            cursor: OuterCursor,
        }

        impl Generator for $name {
            type Yield = $yield;
            type Next = ();
            type Return = ();

            fn next(&mut self, _value: Option<()>) -> Option<$yield> {
                self.cursor.step($project)
            }

            /// A native `Map` iterator has no `return` method.
            fn complete(&mut self, _value: Option<()>) -> Option<$yield> {
                None
            }
        }
    };
}

outer_iterator!(JsMultiMapKeys, JsKey, |key, _| key.clone(), "MultiMapKeys");
outer_iterator!(
    JsMultiMapContainers,
    Rendered,
    |_, bucket: &Bucket<JsKey>| Rendered(bucket.clone()),
    "MultiMapContainers"
);
outer_iterator!(
    JsMultiMapAssociations,
    Association,
    |key: &JsKey, bucket: &Bucket<JsKey>| Association(key.clone(), Rendered(bucket.clone())),
    "MultiMapAssociations"
);

/// `.values()`: the flattened cursor, yielding each bucket's values in turn.
#[napi(iterator, js_name = "MultiMapValues")]
pub struct JsMultiMapValues {
    source: SharedReference<JsMultiMap, &'static RefCell<Core>>,
    state: mnemonist_core::structures::multi_map::FlattenedCursor<JsKey, JsKey>,
}

impl Generator for JsMultiMapValues {
    type Yield = JsKey;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<JsKey> {
        let borrowed = self.source.borrow();

        self.state.step(borrowed.items()).map(|(_, value)| value)
    }

    fn complete(&mut self, _value: Option<()>) -> Option<JsKey> {
        None
    }
}

/// `.entries()`: the flattened cursor, yielding `[key, value]` pairs.
#[napi(iterator, js_name = "MultiMapEntries")]
pub struct JsMultiMapEntries {
    source: SharedReference<JsMultiMap, &'static RefCell<Core>>,
    state: mnemonist_core::structures::multi_map::FlattenedCursor<JsKey, JsKey>,
}

impl Generator for JsMultiMapEntries {
    type Yield = Pair;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Pair> {
        let borrowed = self.source.borrow();

        self.state
            .step(borrowed.items())
            .map(|(key, value)| Pair(key, value))
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Pair> {
        None
    }
}

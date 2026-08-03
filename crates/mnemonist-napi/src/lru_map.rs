//! JS bridge for `lru-map.js` — the `Map`-backed pair's base class.
//!
//! Structurally identical to [`crate::lru_cache::JsLruCache`]; the only
//! difference is the index key. `IK = `[`JsKey`], directly and
//! unmodified — SameValueZero, not a property-string coercion — so `get`,
//! `has`, `peek`, `set` and `setpop` need no [`crate::lru_cache::property_key_of`]
//! step at all. See `crate::lru_cache`'s module docs for the shared design
//! this reuses wholesale.
//!
//! # A confirmed upstream inconsistency (BUG-LRU-CACHE-2)
//!
//! `lru-map.js`'s own `LRUMap.from` throws
//! `'mnemonist/lru-cache.from: could not guess iterable length. ...'` on an
//! unguessable iterable with no capacity — the **wrong** module name, a
//! copy-paste artefact from `lru-cache.js`. Verified against the vendored
//! source at `~/upstream-mnemonist/lru-map.js:241`. `lru-map-with-delete.js`'s
//! own `.from` gets it right (`'mnemonist/lru-map.from: ...'`), so the bug is
//! specific to this file. Reproduced verbatim below; see
//! `docs/modules/lru-cache.md`.

use std::cell::RefCell;
use std::rc::Rc;

use mnemonist_core::structures::lru_cache::{Projection, SetPop};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::array_class::ArrayClass;
use crate::cursor::CellCursor;
use crate::js_key::JsKey;
use crate::js_slot::JsSlot;
use crate::lru_cache::{
    coerce, is_js_truthy, js_key_of_stored, map_new_error, populate_from, resolve_construction,
    resolve_from_construction, step_entry, step_key, step_value, Cursor, Pair, SetPopOutcome,
};

/// Verbatim from `lru-map.js`.
const NOT_POSITIVE: &str = "mnemonist/lru-map: capacity should be positive number.";
/// Verbatim from `lru-map.js`.
const NOT_INTEGER: &str = "mnemonist/lru-map: capacity should be a finite positive integer.";
/// Verbatim from `lru-map.js` — **not** `mnemonist/lru-map.from`. See the
/// module docs (BUG-LRU-CACHE-2).
const CANNOT_GUESS: &str = "mnemonist/lru-cache.from: could not guess iterable length. \
     Please provide desired capacity as last argument.";

/// The core instantiation both `LRUMap` and `LRUMapWithDelete` share.
pub type MapCore = mnemonist_core::structures::lru_cache::LruCache<JsKey, JsSlot, JsSlot>;

/// `to_index` for the `Map`-backed pair's eviction: rebuild the `JsKey` the
/// stored slot started from. See [`crate::lru_cache::js_key_of_stored`].
pub fn map_to_index(stored: &JsSlot) -> JsKey {
    js_key_of_stored(stored)
}

/// An LRU cache backed by a real `Map`, SameValueZero. See the module docs.
#[napi(js_name = "LRUMap")]
pub struct JsLruMap {
    inner: Rc<RefCell<MapCore>>,
    keys_class: Option<ArrayClass>,
    values_class: Option<ArrayClass>,
}

#[napi]
impl JsLruMap {
    /// `new LRUMap(Keys, Values, capacity)`.
    #[napi(constructor)]
    pub fn new(env: Env, keys: Unknown, values: Unknown, capacity: Unknown) -> Result<Self> {
        let construction =
            resolve_construction(&env, &keys, &values, &capacity, NOT_POSITIVE, NOT_INTEGER)?;
        let inner = MapCore::new(construction.capacity)
            .map_err(|error| map_new_error(error, NOT_POSITIVE))?;

        Ok(Self {
            inner: Rc::new(RefCell::new(inner)),
            keys_class: construction.keys_class,
            values_class: construction.values_class,
        })
    }

    #[napi(getter)]
    pub fn capacity(&self) -> u32 {
        self.inner.borrow().capacity() as u32
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().len() as u32
    }

    /// Upstream's `head` property: the pointer of the most recently used
    /// entry.
    #[napi(getter)]
    pub fn head(&self) -> u32 {
        self.inner.borrow().head() as u32
    }

    /// Upstream's `tail` property: the pointer of the least recently used
    /// entry.
    #[napi(getter)]
    pub fn tail(&self) -> u32 {
        self.inner.borrow().tail() as u32
    }

    /// Upstream's `this.items`: a real `Map` there. `test/lru-cache.js:65`
    /// only inspects `cache.items.size`, so this exposes exactly that rather
    /// than rebuilding a full `Map` proxy with no assertion to justify it —
    /// see `docs/modules/lru-cache.md`.
    #[napi(getter)]
    pub fn items(&self, env: Env) -> Result<Object<'_>> {
        let mut object = Object::new(&env)?;

        object.set("size", self.inner.borrow().len() as u32)?;

        Ok(object)
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    #[napi]
    pub fn has(&self, key: JsKey) -> bool {
        self.inner.borrow().has(&key)
    }

    #[napi]
    pub fn peek(&self, key: JsKey) -> Either<JsSlot, Undefined> {
        self.inner.borrow().peek(&key).cloned().into()
    }

    #[napi]
    pub fn get(&self, key: JsKey) -> Either<JsSlot, Undefined> {
        self.inner.borrow_mut().get(&key).cloned().into()
    }

    /// Upstream returns `undefined`, same as `LRUCache.prototype.set`.
    #[napi]
    pub fn set(&self, env: Env, key: Unknown, value: Unknown) -> Result<()> {
        let index_key = JsKey::from_unknown(&key)?;
        let stored_key = coerce(&env, &self.keys_class, &key)?;
        let stored_value = coerce(&env, &self.values_class, &value)?;

        self.inner
            .borrow_mut()
            .set(index_key, stored_key, stored_value, map_to_index);

        Ok(())
    }

    #[napi]
    pub fn setpop(&self, env: Env, key: Unknown, value: Unknown) -> Result<Option<SetPopOutcome>> {
        let index_key = JsKey::from_unknown(&key)?;
        let stored_key = coerce(&env, &self.keys_class, &key)?;
        let stored_value = coerce(&env, &self.values_class, &value)?;

        let outcome =
            self.inner
                .borrow_mut()
                .set_pop(index_key, stored_key, stored_value, map_to_index);

        Ok(match outcome {
            SetPop::None => None,
            SetPop::Overwritten { key, value } => Some(SetPopOutcome {
                evicted: false,
                key,
                value,
            }),
            // BUG-LRU-CACHE-1: see `crate::lru_cache::is_js_truthy`'s doc comment.
            SetPop::Evicted { key, value } if is_js_truthy(&key) => Some(SetPopOutcome {
                evicted: true,
                key,
                value,
            }),
            SetPop::Evicted { .. } => None,
        })
    }

    #[allow(clippy::type_complexity)]
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(JsSlot, JsSlot, Object)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        crate::lru_cache::for_each_entries(&self.inner, this, callback, scope)
    }

    #[napi]
    pub fn keys(&self, env: Env, this: Reference<JsLruMap>) -> Result<JsLruMapKeys> {
        let frozen = self.inner.borrow().frozen(Projection::Keys);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruMapKeys {
            cursor: CellCursor::open_projected(source, frozen),
        })
    }

    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsLruMap>) -> Result<JsLruMapValues> {
        let frozen = self.inner.borrow().frozen(Projection::Values);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruMapValues {
            cursor: CellCursor::open_projected(source, frozen),
        })
    }

    /// Also `Symbol.iterator`, aliased by `install_iterator_factories`.
    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsLruMap>) -> Result<JsLruMapEntries> {
        let frozen = self.inner.borrow().frozen(Projection::Entries);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruMapEntries {
            cursor: CellCursor::open_projected(source, frozen),
        })
    }

    /// `LRUMap.from(iterable, Keys, Values, capacity)`.
    #[napi(factory)]
    pub fn from(
        env: Env,
        iterable: Unknown,
        keys: Unknown,
        values: Unknown,
        capacity: Unknown,
    ) -> Result<Self> {
        let construction = resolve_from_construction(
            &env,
            &iterable,
            &keys,
            &values,
            &capacity,
            NOT_POSITIVE,
            NOT_INTEGER,
            CANNOT_GUESS,
        )?;
        let inner = populate_from(
            &env,
            iterable,
            &construction.keys_class,
            &construction.values_class,
            construction.capacity,
            JsKey::clone,
            map_to_index,
        )?;

        Ok(Self {
            inner,
            keys_class: construction.keys_class,
            values_class: construction.values_class,
        })
    }
}

impl Default for JsLruMap {
    fn default() -> Self {
        Self {
            inner: Rc::new(RefCell::new(
                MapCore::new(1).expect("capacity 1 is always valid"),
            )),
            keys_class: None,
            values_class: None,
        }
    }
}

/// The cursor `LRUMap.prototype.keys()` hands out.
#[napi(iterator, js_name = "LRUMapKeys")]
pub struct JsLruMapKeys {
    cursor: Cursor<JsLruMap, JsKey>,
}

impl Generator for JsLruMapKeys {
    type Yield = JsSlot;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<JsSlot> {
        step_key(&mut self.cursor)
    }

    fn complete(&mut self, _value: Option<()>) -> Option<JsSlot> {
        None
    }
}

/// The cursor `LRUMap.prototype.values()` hands out.
#[napi(iterator, js_name = "LRUMapValues")]
pub struct JsLruMapValues {
    cursor: Cursor<JsLruMap, JsKey>,
}

impl Generator for JsLruMapValues {
    type Yield = JsSlot;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<JsSlot> {
        step_value(&mut self.cursor)
    }

    fn complete(&mut self, _value: Option<()>) -> Option<JsSlot> {
        None
    }
}

/// The cursor `LRUMap.prototype.entries()` hands out, and
/// `LRUMap.prototype[Symbol.iterator]`.
#[napi(iterator, js_name = "LRUMapEntries")]
pub struct JsLruMapEntries {
    cursor: Cursor<JsLruMap, JsKey>,
}

impl Generator for JsLruMapEntries {
    type Yield = Pair;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Pair> {
        step_entry(&mut self.cursor)
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Pair> {
        None
    }
}

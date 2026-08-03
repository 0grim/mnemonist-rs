//! JS bridge for `lru-cache-with-delete.js`.
//!
//! `LRUCacheWithDelete` upstream is `for (var k in LRUCache.prototype)
//! LRUCacheWithDelete.prototype[k] = LRUCache.prototype[k];` plus `delete` and
//! `remove` — the object-backed pair's base class with hole recycling turned
//! on. [`mnemonist_core::structures::lru_cache::LruCache`] already carries the
//! hole list unconditionally (see that module's docs), so this bridge is
//! [`crate::lru_cache::JsLruCache`] verbatim plus the two extra methods; every
//! other method here is the identical body, duplicated because `#[napi]`
//! needs one concrete struct per exported JS class, not because the logic
//! differs.
//!
//! Capacity error messages are `mnemonist/lru-cache: ...`, **not**
//! `mnemonist/lru-cache-with-delete: ...` — upstream's own constructor calls
//! `LRUCache.call(this, Keys, Values, capacity)` and the message is raised
//! from inside that call, naming the base file. Confirmed against
//! `~/upstream-mnemonist/lru-cache-with-delete.js`, which defines no capacity
//! validation of its own.

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
    cache_to_index, coerce, is_js_truthy, map_new_error, populate_from, property_key_of,
    resolve_construction, resolve_from_construction, step_entry, step_key, step_value, CacheCore,
    Cursor, Pair, PropertyKey, SetPopOutcome,
};

/// Verbatim from `lru-cache.js` — see the module docs.
const NOT_POSITIVE: &str = "mnemonist/lru-cache: capacity should be positive number.";
const NOT_INTEGER: &str = "mnemonist/lru-cache: capacity should be a finite positive integer.";
/// Verbatim from `lru-cache-with-delete.js`'s own `.from` — this one gets the
/// module name right, unlike `lru-map.js`'s (BUG-LRU-CACHE-2).
const CANNOT_GUESS: &str = "mnemonist/lru-cache.from: could not guess iterable length. \
     Please provide desired capacity as last argument.";

/// An LRU cache backed by a plain object index, with `delete`/`remove`.
#[napi(js_name = "LRUCacheWithDelete")]
pub struct JsLruCacheWithDelete {
    inner: Rc<RefCell<CacheCore>>,
    keys_class: Option<ArrayClass>,
    values_class: Option<ArrayClass>,
}

#[napi]
impl JsLruCacheWithDelete {
    #[napi(constructor)]
    pub fn new(env: Env, keys: Unknown, values: Unknown, capacity: Unknown) -> Result<Self> {
        let construction =
            resolve_construction(&env, &keys, &values, &capacity, NOT_POSITIVE, NOT_INTEGER)?;
        let inner = CacheCore::new(construction.capacity)
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
    /// entry. `test/lru-cache.js` asserts this directly after emptying the
    /// cache via `delete`.
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

    #[napi(getter)]
    pub fn items(&self, env: Env) -> Result<Object<'_>> {
        let inner = self.inner.borrow();
        let mut object = Object::new(&env)?;

        for (key, pointer) in inner.index_entries() {
            object.set(key.as_str(), pointer as u32)?;
        }

        Ok(object)
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    #[napi]
    pub fn has(&self, key: JsKey) -> bool {
        self.inner.borrow().has(&property_key_of(&key))
    }

    #[napi]
    pub fn peek(&self, key: JsKey) -> Either<JsSlot, Undefined> {
        self.inner
            .borrow()
            .peek(&property_key_of(&key))
            .cloned()
            .into()
    }

    #[napi]
    pub fn get(&self, key: JsKey) -> Either<JsSlot, Undefined> {
        self.inner
            .borrow_mut()
            .get(&property_key_of(&key))
            .cloned()
            .into()
    }

    #[napi]
    pub fn set(&self, env: Env, key: Unknown, value: Unknown) -> Result<()> {
        let raw_key = JsKey::from_unknown(&key)?;
        let index_key = property_key_of(&raw_key);
        let stored_key = coerce(&env, &self.keys_class, &key)?;
        let stored_value = coerce(&env, &self.values_class, &value)?;

        self.inner
            .borrow_mut()
            .set(index_key, stored_key, stored_value, cache_to_index);

        Ok(())
    }

    #[napi]
    pub fn setpop(&self, env: Env, key: Unknown, value: Unknown) -> Result<Option<SetPopOutcome>> {
        let raw_key = JsKey::from_unknown(&key)?;
        let index_key = property_key_of(&raw_key);
        let stored_key = coerce(&env, &self.keys_class, &key)?;
        let stored_value = coerce(&env, &self.values_class, &value)?;

        let outcome =
            self.inner
                .borrow_mut()
                .set_pop(index_key, stored_key, stored_value, cache_to_index);

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

    /// Upstream's `delete`: `true` if the key was present.
    #[napi(js_name = "delete")]
    pub fn delete(&self, key: JsKey) -> bool {
        self.inner.borrow_mut().delete(&property_key_of(&key))
    }

    /// Upstream's `remove(key, missing = undefined)`: the removed value, or
    /// `missing` — echoed back exactly as received, whatever it is — when the
    /// key was absent. `missing` is a plain `Unknown` rather than
    /// `Option<Unknown>`: napi fills an omitted trailing argument with
    /// `undefined` itself, which is exactly upstream's default parameter
    /// value, so no extra handling is needed for the omitted case.
    #[napi]
    pub fn remove(&self, env: Env, key: JsKey, missing: Unknown) -> Result<JsSlot> {
        match self.inner.borrow_mut().remove(&property_key_of(&key)) {
            Some(value) => Ok(value),
            None => JsSlot::new(&env, &missing),
        }
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
    pub fn keys(
        &self,
        env: Env,
        this: Reference<JsLruCacheWithDelete>,
    ) -> Result<JsLruCacheWithDeleteKeys> {
        let frozen = self.inner.borrow().frozen(Projection::Keys);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruCacheWithDeleteKeys {
            cursor: CellCursor::open_projected(source, frozen),
        })
    }

    #[napi]
    pub fn values(
        &self,
        env: Env,
        this: Reference<JsLruCacheWithDelete>,
    ) -> Result<JsLruCacheWithDeleteValues> {
        let frozen = self.inner.borrow().frozen(Projection::Values);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruCacheWithDeleteValues {
            cursor: CellCursor::open_projected(source, frozen),
        })
    }

    /// Also `Symbol.iterator`, aliased by `install_iterator_factories`.
    #[napi]
    pub fn entries(
        &self,
        env: Env,
        this: Reference<JsLruCacheWithDelete>,
    ) -> Result<JsLruCacheWithDeleteEntries> {
        let frozen = self.inner.borrow().frozen(Projection::Entries);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruCacheWithDeleteEntries {
            cursor: CellCursor::open_projected(source, frozen),
        })
    }

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
            property_key_of,
            cache_to_index,
        )?;

        Ok(Self {
            inner,
            keys_class: construction.keys_class,
            values_class: construction.values_class,
        })
    }
}

impl Default for JsLruCacheWithDelete {
    fn default() -> Self {
        Self {
            inner: Rc::new(RefCell::new(
                CacheCore::new(1).expect("capacity 1 is always valid"),
            )),
            keys_class: None,
            values_class: None,
        }
    }
}

#[napi(iterator, js_name = "LRUCacheWithDeleteKeys")]
pub struct JsLruCacheWithDeleteKeys {
    cursor: Cursor<JsLruCacheWithDelete, PropertyKey>,
}

impl Generator for JsLruCacheWithDeleteKeys {
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

#[napi(iterator, js_name = "LRUCacheWithDeleteValues")]
pub struct JsLruCacheWithDeleteValues {
    cursor: Cursor<JsLruCacheWithDelete, PropertyKey>,
}

impl Generator for JsLruCacheWithDeleteValues {
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

#[napi(iterator, js_name = "LRUCacheWithDeleteEntries")]
pub struct JsLruCacheWithDeleteEntries {
    cursor: Cursor<JsLruCacheWithDelete, PropertyKey>,
}

impl Generator for JsLruCacheWithDeleteEntries {
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

//! JS bridge for `lru-map-with-delete.js`.
//!
//! [`crate::lru_map::JsLruMap`] plus `delete`/`remove`, the same relationship
//! [`crate::lru_cache_with_delete`] has to [`crate::lru_cache`]. See that
//! module's docs and `crate::lru_cache`'s shared design.
//!
//! Unlike `lru-map.js`, this file's own `.from` gets the cannot-guess message
//! right (`mnemonist/lru-map.from: ...`) — see `crate::lru_map`'s docs for the
//! sibling that does not (B-142).

use std::cell::RefCell;
use std::rc::Rc;

use mnemonist_core::structures::lru_cache::{Projection, SetPop};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::array_class::ArrayClass;
use crate::cursor::CellCursor;
use crate::js_key::JsKey;
use crate::js_slot::JsSlot;
use crate::lru_cache::{coerce, map_new_error, populate_from, resolve_construction};
use crate::lru_cache::{resolve_from_construction, step_entry, step_key, step_value};
use crate::lru_cache::{Cursor, Pair, SetPopOutcome};
use crate::lru_map::{map_to_index, MapCore};

/// Verbatim from `lru-map.js` — see `crate::lru_map`'s docs on why
/// `LRUMapWithDelete`'s own constructor validation shares `LRUMap`'s message.
const NOT_POSITIVE: &str = "mnemonist/lru-map: capacity should be positive number.";
const NOT_INTEGER: &str = "mnemonist/lru-map: capacity should be a finite positive integer.";
/// Verbatim from `lru-map-with-delete.js`'s own `.from` — correct, unlike
/// `lru-map.js`'s (B-142).
const CANNOT_GUESS: &str = "mnemonist/lru-map.from: could not guess iterable length. \
     Please provide desired capacity as last argument.";

/// An LRU cache backed by a real `Map`, with `delete`/`remove`.
#[napi(js_name = "LRUMapWithDelete")]
pub struct JsLruMapWithDelete {
    inner: Rc<RefCell<MapCore>>,
    keys_class: Option<ArrayClass>,
    values_class: Option<ArrayClass>,
}

#[napi]
impl JsLruMapWithDelete {
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
            SetPop::Evicted { key, value } => Some(SetPopOutcome {
                evicted: true,
                key,
                value,
            }),
        })
    }

    #[napi(js_name = "delete")]
    pub fn delete(&self, key: JsKey) -> bool {
        self.inner.borrow_mut().delete(&key)
    }

    #[napi]
    pub fn remove(&self, env: Env, key: JsKey, missing: Unknown) -> Result<JsSlot> {
        match self.inner.borrow_mut().remove(&key) {
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
        this: Reference<JsLruMapWithDelete>,
    ) -> Result<JsLruMapWithDeleteKeys> {
        let frozen = self.inner.borrow().frozen(Projection::Keys);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruMapWithDeleteKeys {
            cursor: CellCursor::open_projected(source, frozen),
        })
    }

    #[napi]
    pub fn values(
        &self,
        env: Env,
        this: Reference<JsLruMapWithDelete>,
    ) -> Result<JsLruMapWithDeleteValues> {
        let frozen = self.inner.borrow().frozen(Projection::Values);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruMapWithDeleteValues {
            cursor: CellCursor::open_projected(source, frozen),
        })
    }

    /// Also `Symbol.iterator`, aliased by `install_iterator_factories`.
    #[napi]
    pub fn entries(
        &self,
        env: Env,
        this: Reference<JsLruMapWithDelete>,
    ) -> Result<JsLruMapWithDeleteEntries> {
        let frozen = self.inner.borrow().frozen(Projection::Entries);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruMapWithDeleteEntries {
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

impl Default for JsLruMapWithDelete {
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

#[napi(iterator, js_name = "LRUMapWithDeleteKeys")]
pub struct JsLruMapWithDeleteKeys {
    cursor: Cursor<JsLruMapWithDelete, JsKey>,
}

impl Generator for JsLruMapWithDeleteKeys {
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

#[napi(iterator, js_name = "LRUMapWithDeleteValues")]
pub struct JsLruMapWithDeleteValues {
    cursor: Cursor<JsLruMapWithDelete, JsKey>,
}

impl Generator for JsLruMapWithDeleteValues {
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

#[napi(iterator, js_name = "LRUMapWithDeleteEntries")]
pub struct JsLruMapWithDeleteEntries {
    cursor: Cursor<JsLruMapWithDelete, JsKey>,
}

impl Generator for JsLruMapWithDeleteEntries {
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

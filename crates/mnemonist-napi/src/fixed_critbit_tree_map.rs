//! JS bridge for [`mnemonist_core::structures::fixed_critbit_tree_map`].
//!
//! Same key/value handling as [`crate::critbit_tree_map`] (see that
//! module's docs for the byte-truncation divergence, D-245). Two things
//! specific to the fixed variant.
//!
//! # `set` can genuinely throw
//!
//! Once more than `capacity` distinct keys have been inserted, a later
//! `set` call can walk through the corrupted node the overflow left behind
//! and hit [`Error::Corrupted`] — core's own stand-in for upstream's
//! `TypeError: Cannot read properties of undefined (reading 'length')`. Sent
//! straight through with that same message text, so a caller two layers up
//! sees the identical crash upstream's own missing capacity guard produces
//! — see the core module's docs, part 1, for the full mechanism and NOTES.md
//! B-260/B-261.
//!
//! # No `delete`
//!
//! `fixed-critbit-tree-map.js` never had one; neither does this bridge.

use std::cell::RefCell;

use mnemonist_core::structures::fixed_critbit_tree_map::{
    Error as CoreError, FixedCritBitTreeMap as CoreMap,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::js_slot::read_utf16;
use crate::js_value::{release_slot, Loaned, Received, Retained};

/// A stored value slot. `None` is `undefined`.
type Value = Option<Retained>;

/// See `crate::critbit_tree_map::decode_key` — identical truncation, kept
/// duplicated because the two bridge modules are independent, matching
/// their independent core modules.
fn decode_key(env: &Env, value: &Unknown) -> Result<Vec<u8>> {
    if value.get_type()? != ValueType::String {
        return Err(Error::new(
            Status::InvalidArg,
            "mnemonist/fixed-critbit-tree-map: keys must be strings.",
        ));
    }

    let units = read_utf16(env, value)?;

    Ok(units.into_iter().map(|unit| unit as u8).collect())
}

fn loan(value: Option<&Value>) -> Loaned {
    Loaned::of(value.and_then(Option::as_ref))
}

/// Surface a core error as a JS exception, with upstream's own text.
fn raise(error: CoreError) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

/// `typeof capacity !== 'number' || capacity <= 0` — upstream's one check,
/// covering both "missing" and "wrong type/value" with the same message.
fn capacity_of(env: &Env, value: &Unknown) -> Result<usize> {
    use mnemonist_core::structures::fixed_critbit_tree_map::BAD_CAPACITY;

    if value.get_type()? != ValueType::Number {
        return Err(Error::new(Status::InvalidArg, BAD_CAPACITY));
    }

    let capacity = crate::foreach::to_number(env, value)?;

    if capacity <= 0.0 || !capacity.is_finite() || capacity.fract() != 0.0 {
        return Err(Error::new(Status::InvalidArg, BAD_CAPACITY));
    }

    Ok(capacity as usize)
}

/// Upstream's `FixedCritBitTreeMap`.
#[napi(js_name = "FixedCritBitTreeMap", custom_finalize)]
pub struct JsFixedCritBitTreeMap {
    inner: RefCell<CoreMap<Value>>,
}

#[napi]
impl JsFixedCritBitTreeMap {
    /// `new FixedCritBitTreeMap(capacity)`. `capacity` is [`Unknown`] rather
    /// than a typed number so a missing or non-numeric argument surfaces
    /// upstream's own message text (containing "capacity", which is all
    /// `test/fixed-critbit-tree-map.js`'s `assert.throws(fn, /capacity/)`
    /// checks) rather than napi's own generic coercion error.
    #[napi(constructor)]
    pub fn new(env: Env, capacity: Unknown) -> Result<Self> {
        let capacity = capacity_of(&env, &capacity)?;

        CoreMap::new(capacity)
            .map(|inner| Self {
                inner: RefCell::new(inner),
            })
            .map_err(raise)
    }

    #[napi(getter)]
    pub fn capacity(&self) -> u32 {
        self.inner.borrow().capacity() as u32
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    /// Upstream's `root` — a raw pointer, exposed verbatim, except right
    /// after a `clear`, where it is upstream's own `null` instead (B-260).
    /// `None` here becomes JS `null`, matching upstream's own
    /// `this.root = null`. See
    /// `mnemonist_core::structures::fixed_critbit_tree_map::FixedCritBitTreeMap::root`'s
    /// doc comment for why this needs no reconstruction the way the
    /// unbounded variant's `root` does.
    #[napi(getter)]
    pub fn root(&self) -> Option<i64> {
        self.inner.borrow().root()
    }

    /// Upstream's `clear`: resets `root`, and (see the core module's docs)
    /// leaves every backing array untouched — including every value it
    /// still holds a reference to. So nothing is released here; a value
    /// only stops being reachable through `JsFixedCritBitTreeMap` itself
    /// when the whole structure is finalized. Matches upstream precisely:
    /// there is no way to observe a "cleared" value released any sooner,
    /// because there is no `delete` and `get`/`has` both go through `root`.
    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// Upstream's `set`, which returns `this` for chaining. Throws with
    /// upstream's own crash text once capacity has been exceeded and this
    /// call's walk passes through the resulting corrupted node — see the
    /// module docs.
    #[napi]
    pub fn set<'a>(
        &self,
        this: This<'a>,
        env: Env,
        key: Unknown,
        value: Received,
    ) -> Result<This<'a>> {
        let key = decode_key(&env, &key)?;
        let displaced = self
            .inner
            .borrow_mut()
            .set(key, value.into_slot())
            .map_err(raise)?;

        if let Some(mut displaced) = displaced {
            release_slot(&mut displaced, &env)?;
        }

        Ok(this)
    }

    #[napi]
    pub fn get(&self, env: Env, key: Unknown) -> Result<Loaned> {
        let key = decode_key(&env, &key)?;

        Ok(loan(self.inner.borrow().get(&key)))
    }

    #[napi]
    pub fn has(&self, env: Env, key: Unknown) -> Result<bool> {
        let key = decode_key(&env, &key)?;

        Ok(self.inner.borrow().has(&key))
    }

    /// Upstream's `forEach(callback, scope)` — same shape as
    /// `crate::critbit_tree_map::JsCritBitTreeMap::for_each`.
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(Loaned, String)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let entries: Vec<(Vec<u8>, Loaned)> = {
            let inner = self.inner.borrow();
            let mut out = Vec::with_capacity(inner.size());

            inner.for_each(|value, key| out.push((key.to_vec(), loan(Some(value)))));

            out
        };

        let stack = this.object;

        for (key, value) in entries {
            let key_string: String = key.iter().map(|&byte| byte as char).collect();
            let arguments = (value, key_string).into();

            match &scope {
                Some(scope) => callback.apply(*scope, arguments)?,
                None => callback.apply(stack, arguments)?,
            };
        }

        Ok(())
    }
}

impl ObjectFinalize for JsFixedCritBitTreeMap {
    fn finalize(self, env: Env) -> Result<()> {
        let mut inner = self.inner.into_inner();

        for value in inner.values_mut() {
            release_slot(value, &env)?;
        }

        Ok(())
    }
}

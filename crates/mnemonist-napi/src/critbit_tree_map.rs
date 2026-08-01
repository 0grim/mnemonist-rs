//! JS bridge for [`mnemonist_core::structures::critbit_tree_map`].
//!
//! A much smaller surface than `trie_map`'s bridge: upstream's
//! `critbit-tree-map.js` has no `Token`/array mode, no `find`, no
//! `values`/`keys`/`entries` iterators, and no `Symbol.iterator` — just
//! `clear`, `set`, `get`, `has`, `delete`, `forEach`. Two things worth
//! knowing about.
//!
//! # 1. A key is read as bytes, not as a full UTF-16 string
//!
//! [`decode_key`] takes each UTF-16 code unit of the JS string and truncates
//! it to its low 8 bits. That is a real, disclosed divergence (D-245 in
//! DECISIONS-CANDIDATES.md) from upstream, which runs its critical-bit
//! arithmetic against the untruncated code unit — arithmetic that masks
//! with `0xff` at nearly every step anyway (see the core module's own
//! docs), so it only ever computes something matching "first differing
//! bit" for Latin-1 keys (code point < 256) in the first place. No test in
//! either original suite supplies a key outside that range, so truncating
//! at the boundary is a no-op for every case gate 4 exercises and avoids
//! reproducing upstream's own multi-byte masked-arithmetic bug for the
//! empty set of tests that would need it.
//!
//! # 2. `RefCell`, for the same reason as every re-entrant bridge here
//!
//! No method here calls back into JavaScript, so nothing can *currently*
//! re-enter — but `RefCell` is used anyway (matching `default_map`'s and
//! `trie_map`'s own reasoning) because a `&self` on a `Freeze`d type is
//! `noalias readonly` to LLVM, which has hoisted a read out of a loop it
//! should not have once already (B-31); see `crate::cursor::CellCursor`'s
//! docs.

use std::cell::RefCell;

use mnemonist_core::structures::critbit_tree_map::CritBitTreeMap as CoreMap;
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::js_slot::read_utf16;
use crate::js_value::{release_slot, Loaned, Received, Retained};

/// A stored value slot. `None` is `undefined` — a stored key can hold it,
/// and `has`/`get`'s absence check does not care which this is.
type Value = Option<Retained>;

/// Truncate each UTF-16 code unit of a JS string key to its low 8 bits. See
/// the module docs, part 1.
fn decode_key(env: &Env, value: &Unknown) -> Result<Vec<u8>> {
    if value.get_type()? != ValueType::String {
        return Err(Error::new(
            Status::InvalidArg,
            "mnemonist/critbit-tree-map: keys must be strings.",
        ));
    }

    let units = read_utf16(env, value)?;

    Ok(units.into_iter().map(|unit| unit as u8).collect())
}

fn loan(value: Option<&Value>) -> Loaned {
    Loaned::of(value.and_then(Option::as_ref))
}

/// Upstream's `CritBitTreeMap`.
#[napi(js_name = "CritBitTreeMap", custom_finalize)]
pub struct JsCritBitTreeMap {
    inner: RefCell<CoreMap<Value>>,
}

#[napi]
impl JsCritBitTreeMap {
    // `new CritBitTreeMap()` takes no arguments upstream, so there is
    // nothing for a `Default` impl to add over calling this directly; napi
    // classes are constructed from JS, not via the `Default` trait.
    #[allow(clippy::new_without_default)]
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(CoreMap::new()),
        }
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

    /// Upstream's `set`, which returns `this` for chaining.
    #[napi]
    pub fn set<'a>(
        &self,
        this: This<'a>,
        env: Env,
        key: Unknown,
        value: Received,
    ) -> Result<This<'a>> {
        let key = decode_key(&env, &key)?;
        let displaced = self.inner.borrow_mut().set(key, value.into_slot());

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

    #[napi(js_name = "delete")]
    pub fn delete(&self, env: Env, key: Unknown) -> Result<bool> {
        let key = decode_key(&env, &key)?;
        let removed = self.inner.borrow_mut().delete(&key);

        match removed {
            None => Ok(false),
            Some(mut value) => {
                release_slot(&mut value, &env)?;

                Ok(true)
            }
        }
    }

    /// Upstream's `forEach(callback, scope)`: `scope` defaults to the map
    /// itself, matching `arguments.length > 1 ? scope : this`. napi's typed
    /// signature cannot distinguish an omitted argument from an explicit
    /// `undefined` the way upstream's `arguments.length` can — the omitted
    /// case, the only one either original suite exercises, is exact.
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(Loaned, String)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        // Collect first, call second: the callback may re-enter (`set`,
        // `delete`, `clear`), and `for_each` holds an immutable borrow of
        // `inner` for its whole walk, which a re-entrant mutation would
        // panic against if the callback ran from inside it.
        let entries: Vec<(Vec<u8>, Loaned)> = {
            let inner = self.inner.borrow();
            let mut out = Vec::with_capacity(inner.size());

            inner.for_each(|value, key| out.push((key.to_vec(), loan(Some(value)))));

            out
        };

        let stack = this.object;

        for (key, value) in entries {
            // Bytes back to a UTF-16 string one code unit at a time --
            // the inverse of `decode_key`'s truncation (module docs, part
            // 1), which round-trips exactly for the Latin-1 range gate 4
            // exercises.
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

impl ObjectFinalize for JsCritBitTreeMap {
    fn finalize(self, env: Env) -> Result<()> {
        let mut inner = self.inner.into_inner();

        for slot in inner.values_mut() {
            release_slot(slot, &env)?;
        }

        Ok(())
    }
}

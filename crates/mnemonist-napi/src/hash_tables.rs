//! JS bridge for [`mnemonist_core::utils::hash_tables`].
//!
//! `jenkinsInt32` is a pure `i32 -> i32` function and bridges directly. The
//! three `linearProbing` functions take a hash function as their *first*
//! argument -- upstream's own generality, not something `test/_utils.js`
//! narrows away, even though the one hash it ever passes is
//! `hashTables.hashes.jenkinsInt32` itself. So `hash` crosses as a genuine JS
//! callback, one call per probe, using the same "sticky error" shape as
//! `crate::binary_search`'s comparators: [`mnemonist_core::utils::hash_tables`]'s
//! three functions take `F: Fn(u32) -> i32` with no `Result`, so a throwing
//! hash is recorded rather than propagated mid-probe and surfaced once the
//! walk returns.
//!
//! `keys`/`values` are mutated **in place** -- `linearProbingSet` writes into
//! the caller's own `Uint32Array`s, exactly as upstream's `keys[j] = key`
//! does. `Uint32Array::as_mut` is `unsafe` (napi 3.12 has no synchronisation
//! primitive over a buffer JS could be resizing concurrently; single-threaded
//! Node makes that moot here, same as every other typed-array mutation in
//! this crate).

use std::cell::RefCell;

use mnemonist_core::utils::hash_tables as core_hash;
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// `hash-tables.js#hashes.jenkinsInt32`.
#[napi(js_name = "hashTablesJenkinsInt32")]
pub fn jenkins_int32(a: i32) -> i32 {
    core_hash::jenkins_int32(a)
}

/// `hash-tables.js#linearProbing.get`.
#[napi(js_name = "hashTablesLinearProbingGet")]
pub fn linear_probing_get(
    hash: Function<u32, i32>,
    keys: Uint32Array,
    values: Uint32Array,
    key: u32,
) -> Result<Either<u32, Undefined>> {
    let sticky = Sticky::new();
    let found = core_hash::linear_probing_get(sticky.wrap(&hash), &keys, &values, key).copied();

    sticky.into_result(match found {
        Some(value) => Either::A(value),
        None => Either::B(()),
    })
}

/// `hash-tables.js#linearProbing.has`.
#[napi(js_name = "hashTablesLinearProbingHas")]
pub fn linear_probing_has(hash: Function<u32, i32>, keys: Uint32Array, key: u32) -> Result<bool> {
    let sticky = Sticky::new();
    let found = core_hash::linear_probing_has(sticky.wrap(&hash), &keys, key);

    sticky.into_result(found)
}

/// `hash-tables.js#linearProbing.set`.
///
/// # Errors
///
/// [`core_hash::TABLE_IS_FULL`] (matching upstream's thrown message,
/// verbatim) when a full turn finds neither the key nor an empty slot; the
/// hash's own thrown error, if it threw.
#[napi(js_name = "hashTablesLinearProbingSet")]
pub fn linear_probing_set(
    hash: Function<u32, i32>,
    mut keys: Uint32Array,
    mut values: Uint32Array,
    key: u32,
    value: u32,
) -> Result<()> {
    let sticky = Sticky::new();
    // SAFETY: single-threaded Node holds this call for its whole duration;
    // nothing else can touch the backing `ArrayBuffer` while the mutable view
    // is live. Same pattern as every other typed-array mutation in this
    // crate (e.g. `crate::sort`'s in-place sorts).
    let keys = unsafe { keys.as_mut() };
    let values = unsafe { values.as_mut() };

    let outcome = core_hash::linear_probing_set(sticky.wrap(&hash), keys, values, key, value);

    sticky.into_result(())?;

    outcome.map_err(|message| Error::new(Status::GenericFailure, message.to_owned()))
}

/// Same shape as `crate::binary_search::Sticky`, specialised to a
/// single-argument `u32 -> i32` callback.
struct Sticky {
    error: RefCell<Option<Error>>,
}

impl Sticky {
    fn new() -> Self {
        Self {
            error: RefCell::new(None),
        }
    }

    fn wrap<'a>(&'a self, hash: &'a Function<u32, i32>) -> impl Fn(u32) -> i32 + 'a {
        move |key: u32| {
            if self.error.borrow().is_some() {
                return 0;
            }

            match hash.call(key) {
                Ok(result) => result,
                Err(error) => {
                    *self.error.borrow_mut() = Some(error);
                    0
                }
            }
        }
    }

    fn into_result<T>(self, value: T) -> Result<T> {
        match self.error.into_inner() {
            Some(error) => Err(error),
            None => Ok(value),
        }
    }
}

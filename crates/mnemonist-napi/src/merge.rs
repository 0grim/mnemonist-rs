//! JS bridge for [`mnemonist_core::utils::merge`].
//!
//! Six free functions over plain `Array`s of numbers — `test/_utils.js`'s
//! `merge`/`unionUnique`/`intersectionUnique` describe blocks never pass
//! anything else, so this bridge does not attempt upstream's full duck-typed
//! generality (any array-like of anything comparable with `<`/`>`), the same
//! scoping call `crate::sort` already made for its own numeric arrays.
//!
//! # Variadicity is JS-arity, so it lives in the shim
//!
//! `merge`, `unionUnique` and `intersectionUnique` are upstream's real
//! exports, each dispatching on `arguments.length === 2` vs. everything else
//! and on `isArrayLike(arguments[0])`. napi has no variadic parameter
//! (`crate::set`'s module docs cover the same gap for `union`/`intersection`
//! there), so this file exposes the two-array and k-way halves as separate
//! `#[napi]` functions, and `tests/bridge/_utils.js` assembles the dispatch —
//! arity glue, not semantics, per DESIGN.md §2.3 Problem 2. `isArrayLike`
//! itself is `crate::iterables::js_is_array_like`, already exported at the
//! addon's top level; nothing new is built for it here.
//!
//! # B-180 surfaces as a thrown `TypeError`
//!
//! [`mnemonist_core::utils::merge::KWayError::StaleLengthMismatch`] becomes a
//! JS `Error` carrying
//! [`mnemonist_core::utils::merge::STALE_LENGTH_TYPE_ERROR`] verbatim, so a
//! caller checking upstream's message (as `assert.throws(fn, /message/)`
//! would) sees the identical text. See NOTES.md B-180 and the core module's
//! own docs for the empirical confirmation against Node 24.18.1.

use mnemonist_core::utils::merge as core_merge;
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// `merge.js#mergeArrays` (private upstream, reached only through `merge`
/// when `arguments.length === 2`).
#[napi(js_name = "mergeTwo")]
pub fn merge_two(a: Vec<f64>, b: Vec<f64>) -> Vec<f64> {
    core_merge::merge_two(&a, &b)
}

/// `merge.js#kWayMergeArrays`, reached whenever `arguments.length !== 2`.
///
/// # Errors
///
/// See the module docs — B-180.
#[napi(js_name = "mergeMany")]
pub fn merge_many(arrays: Vec<Vec<f64>>) -> Result<Vec<f64>> {
    let borrowed: Vec<&[f64]> = arrays.iter().map(Vec::as_slice).collect();

    stale_length(core_merge::merge_k(&borrowed))
}

/// `merge.js#unionUniqueArrays`.
#[napi(js_name = "unionUniqueTwo")]
pub fn union_unique_two(a: Vec<f64>, b: Vec<f64>) -> Vec<f64> {
    core_merge::union_unique_two(&a, &b)
}

/// `merge.js#kWayUnionUniqueArrays`.
///
/// # Errors
///
/// See the module docs — B-180.
#[napi(js_name = "unionUniqueMany")]
pub fn union_unique_many(arrays: Vec<Vec<f64>>) -> Result<Vec<f64>> {
    let borrowed: Vec<&[f64]> = arrays.iter().map(Vec::as_slice).collect();

    stale_length(core_merge::union_unique_k(&borrowed))
}

/// `merge.js#exports.intersectionUniqueArrays` (directly exported upstream,
/// unlike its merge/union siblings).
#[napi(js_name = "intersectionUniqueTwo")]
pub fn intersection_unique_two(a: Vec<f64>, b: Vec<f64>) -> Vec<f64> {
    core_merge::intersection_unique_two(&a, &b)
}

/// `merge.js#exports.kWayIntersectionUniqueArrays` — immune to B-180, so this
/// is infallible; see the core module's docs for why.
#[napi(js_name = "intersectionUniqueMany")]
pub fn intersection_unique_many(arrays: Vec<Vec<f64>>) -> Vec<f64> {
    let borrowed: Vec<&[f64]> = arrays.iter().map(Vec::as_slice).collect();

    core_merge::intersection_unique_k(&borrowed)
}

/// Surface [`core_merge::KWayError`] as the `TypeError` upstream throws.
fn stale_length(outcome: std::result::Result<Vec<f64>, core_merge::KWayError>) -> Result<Vec<f64>> {
    outcome.map_err(|core_merge::KWayError::StaleLengthMismatch| {
        Error::new(
            Status::GenericFailure,
            core_merge::STALE_LENGTH_TYPE_ERROR.to_owned(),
        )
    })
}

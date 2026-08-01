//! JS bridge for [`mnemonist_core::utils::binary_search`].
//!
//! Seven functions. The four plain ones (`search`, `lowerBound`, `upperBound`,
//! `lowerBoundIndices`) only ever see arrays of numbers in `test/_utils.js`,
//! so they are bridged directly over `Vec<f64>` — no `JsSlot`, no callback.
//!
//! The three `WithComparator` variants are different: `test/_utils.js` feeds
//! them an array of **strings** (`'one'`, `'two'`, ...) and a comparator that
//! resolves both strings and bare numbers through a lookup table. That
//! comparator is a JavaScript function called once per comparison from inside
//! the search loop — the one place this unit is not "just" a pure function.
//! [`mnemonist_core::utils::binary_search::search_with_comparator`] (and the
//! two bound variants) take a plain `Fn(&T, &T) -> Ordering` with no `Result`,
//! so a throwing comparator cannot propagate through it directly. The fix is
//! the same "sticky error" shape `crate::vector`/`crate::bit_vector` already
//! use for a fallible growth policy called from inside a core algorithm:
//! [`Sticky`] records the first `Error`, the wrapped closure answers
//! `Ordering::Equal` after that (the algorithm still terminates; the answer is
//! discarded), and the caller checks the cell once the algorithm returns.

use std::cell::RefCell;
use std::cmp::Ordering;

use mnemonist_core::utils::binary_search as core_search;
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::js_slot::JsSlot;

/// `binary-search.js#search`.
#[napi(js_name = "binarySearchSearch")]
pub fn search(array: Vec<f64>, value: f64, lo: Option<u32>, hi: Option<u32>) -> i64 {
    core_search::search(&array, &value, opt(lo), opt(hi)) as i64
}

/// `binary-search.js#lowerBound`.
#[napi(js_name = "binarySearchLowerBound")]
pub fn lower_bound(array: Vec<f64>, value: f64, lo: Option<u32>, hi: Option<u32>) -> i64 {
    core_search::lower_bound(&array, &value, opt(lo), opt(hi)) as i64
}

/// `binary-search.js#upperBound`.
#[napi(js_name = "binarySearchUpperBound")]
pub fn upper_bound(array: Vec<f64>, value: f64, lo: Option<u32>, hi: Option<u32>) -> i64 {
    core_search::upper_bound(&array, &value, opt(lo), opt(hi)) as i64
}

/// `binary-search.js#lowerBoundIndices`.
#[napi(js_name = "binarySearchLowerBoundIndices")]
pub fn lower_bound_indices(
    array: Vec<f64>,
    indices: Vec<u32>,
    value: f64,
    lo: Option<u32>,
    hi: Option<u32>,
) -> i64 {
    let indices: Vec<usize> = indices.into_iter().map(|i| i as usize).collect();

    core_search::lower_bound_indices(&array, &indices, &value, opt(lo), opt(hi)) as i64
}

/// `binary-search.js#searchWithComparator`.
#[napi(js_name = "binarySearchSearchWithComparator")]
pub fn search_with_comparator(
    env: Env,
    comparator: Function<FnArgs<(JsSlot, JsSlot)>, f64>,
    array: Vec<Unknown>,
    value: Unknown,
) -> Result<i64> {
    let array = to_slots(&env, array)?;
    let value = JsSlot::new(&env, &value)?;
    let sticky = Sticky::new();

    let index = core_search::search_with_comparator(sticky.wrap(&comparator), &array, &value);

    sticky.into_result(index as i64)
}

/// `binary-search.js#lowerBoundWithComparator`.
#[napi(js_name = "binarySearchLowerBoundWithComparator")]
pub fn lower_bound_with_comparator(
    env: Env,
    comparator: Function<FnArgs<(JsSlot, JsSlot)>, f64>,
    array: Vec<Unknown>,
    value: Unknown,
) -> Result<i64> {
    let array = to_slots(&env, array)?;
    let value = JsSlot::new(&env, &value)?;
    let sticky = Sticky::new();

    let index = core_search::lower_bound_with_comparator(sticky.wrap(&comparator), &array, &value);

    sticky.into_result(index as i64)
}

/// `binary-search.js#upperBoundWithComparator`.
#[napi(js_name = "binarySearchUpperBoundWithComparator")]
pub fn upper_bound_with_comparator(
    env: Env,
    comparator: Function<FnArgs<(JsSlot, JsSlot)>, f64>,
    array: Vec<Unknown>,
    value: Unknown,
) -> Result<i64> {
    let array = to_slots(&env, array)?;
    let value = JsSlot::new(&env, &value)?;
    let sticky = Sticky::new();

    let index = core_search::upper_bound_with_comparator(sticky.wrap(&comparator), &array, &value);

    sticky.into_result(index as i64)
}

fn opt(value: Option<u32>) -> Option<usize> {
    value.map(|value| value as usize)
}

fn to_slots(env: &Env, values: Vec<Unknown>) -> Result<Vec<JsSlot>> {
    values.iter().map(|value| JsSlot::new(env, value)).collect()
}

/// Captures the first `Error` a wrapped comparator throws, so a plain
/// `Fn(&T, &T) -> Ordering` (no `Result` in its signature) can still be
/// driven by a fallible JS call. Once set, the wrapped closure stops calling
/// into JS at all and answers `Ordering::Equal` -- the search still
/// terminates (every `binary_search` function is bounded by `array.len()`),
/// and whatever index it lands on is discarded by [`Sticky::into_result`].
struct Sticky {
    error: RefCell<Option<Error>>,
}

impl Sticky {
    fn new() -> Self {
        Self {
            error: RefCell::new(None),
        }
    }

    /// Wrap a comparator so it answers `Ordering` instead of `Result<f64>`,
    /// recording (rather than propagating) the first failure.
    fn wrap<'a>(
        &'a self,
        comparator: &'a Function<FnArgs<(JsSlot, JsSlot)>, f64>,
    ) -> impl Fn(&JsSlot, &JsSlot) -> Ordering + 'a {
        move |a: &JsSlot, b: &JsSlot| {
            if self.error.borrow().is_some() {
                return Ordering::Equal;
            }

            match comparator.call((a.clone(), b.clone()).into()) {
                Ok(result) if result < 0.0 => Ordering::Less,
                Ok(result) if result > 0.0 => Ordering::Greater,
                Ok(_) => Ordering::Equal,
                Err(error) => {
                    *self.error.borrow_mut() = Some(error);
                    Ordering::Equal
                }
            }
        }
    }

    /// The algorithm's own result, unless the comparator threw -- in which
    /// case that's the `Error` upstream's own `assert.throws` would see.
    fn into_result<T>(self, value: T) -> Result<T> {
        match self.error.into_inner() {
            Some(error) => Err(error),
            None => Ok(value),
        }
    }
}

//! JS bridge for [`mnemonist_core::sort`].
//!
//! The first ported module with **no instance**. Upstream `sort/quick.js` and
//! `sort/insertion.js` export four bare functions that mutate a caller-supplied
//! array, so there is no `#[napi]` class here, no `RefCell<Core…>`, and nothing
//! for `crate::cursor` to attach a `Symbol.iterator` to — the whole module is
//! four `#[napi]` free functions and the coercion around them.
//!
//! That shape has one consequence worth stating: **the addon's exports are a
//! flat namespace**, so `inplaceQuickSort` and friends sit next to `Stack` and
//! `BitSet` rather than under a `sort/quick` object. Re-assembling upstream's
//! two-file export shape is the shim's job — DESIGN.md §2.3's Problem 2, which
//! is exactly the case it describes — and `tests/bridge/sort.js` does it.
//!
//! # What the port accepts, and what upstream accepts
//!
//! Upstream is duck-typed: `array` is anything indexable and its elements are
//! compared with `>`, `>=` and `<=`, which coerce through `valueOf`/`toString`.
//! This bridge takes **a plain `Array` of numbers**, which is every input
//! `test/sort.js` uses and nothing else. Three consequences, all stated in
//! `docs/modules/sort.md` rather than hidden:
//!
//! 1. Strings, objects and mixed arrays are rejected with an error naming the
//!    limit, where upstream would sort them by JavaScript's relational
//!    comparison. Supporting them is DESIGN.md §3.3's T2 tier (callbacks into
//!    JS mid-algorithm), which this unit deliberately does not reach for.
//! 2. It follows that **B-80 and B-81 are unreachable here.** Both upstream
//!    bugs need an element whose comparison re-enters the sorter, which needs a
//!    `valueOf`, which needs an object element. With numbers only, upstream's
//!    shared global counter and shared partition stack are unobservable, so
//!    this port's locals cannot disagree with them.
//! 3. `lo`/`hi` outside `0..=length` are rejected. Upstream reads `undefined`
//!    past the end and writes into holes; a JS array hole has no Rust
//!    representation, and `mnemonist_core::sort::check_window` takes the same
//!    position `PointerVec::get` already does.
//!
//! # In place means in place
//!
//! Every function returns **the object it was given**, not a copy.
//! `Array<'env>` and the typed arrays both carry their original `napi_value`
//! through `ToNapiValue`, so `quick.inplaceQuickSort(data, 0, 3) === data`
//! holds as it does upstream. `test/sort.js` never asserts that identity — it
//! only inspects the return value — which is precisely why it is worth getting
//! right rather than returning a fresh array that would pass.
//!
//! # A napi-rs wart, recorded because it is real
//!
//! `indices` arrives as `Either3<Uint8Array, Uint16Array, Uint32Array>`.
//! napi-rs picks the variant by *trying* each in turn, and
//! `Uint8Array::from_napi_value` creates its `napi_ref` **before** checking the
//! element type, then returns the type error without deleting it. So a
//! `Uint16Array` or `Uint32Array` argument leaks one strong reference per call.
//! Unfixable from here, bounded by the number of calls the harness makes, and
//! not on any path that runs long enough to matter — but it is a leak, and
//! calling it anything else would be the kind of overclaim CLAUDE.md warns
//! about.

use mnemonist_core::sort::{insertion, quick};
use mnemonist_core::utils::typed_arrays::{
    IndicesError, PointerVec, INVALID_TYPED_ARRAY_LENGTH, POINTER_ARRAY_TOO_LARGE,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// A JS typed array of indices, at any of the three unsigned widths upstream's
/// `getPointerArray` can return.
///
/// Spelled out rather than aliased for the same reason the callback types in
/// `crate::stack` are: napi's macro reads the parameter type syntactically and
/// stops recognising the shape behind a `type`.
type Indices = Either3<Uint8Array, Uint16Array, Uint32Array>;

/// `sort/insertion.js#inplaceInsertionSort`.
#[napi(js_name = "inplaceInsertionSort")]
pub fn inplace_insertion_sort(array: Array<'_>, lo: f64, hi: f64) -> Result<Array<'_>> {
    sort_values(array, lo, hi, |values| {
        insertion::inplace_insertion_sort(values, 0, values.len());
    })
}

/// `sort/quick.js#inplaceQuickSort`.
#[napi(js_name = "inplaceQuickSort")]
pub fn inplace_quick_sort(array: Array<'_>, lo: f64, hi: f64) -> Result<Array<'_>> {
    sort_values(array, lo, hi, |values| {
        quick::inplace_quick_sort(values, 0, values.len());
    })
}

/// `sort/insertion.js#inplaceInsertionSortIndices`.
#[napi(js_name = "inplaceInsertionSortIndices")]
pub fn inplace_insertion_sort_indices(
    array: Array<'_>,
    indices: Indices,
    lo: f64,
    hi: f64,
) -> Result<Indices> {
    sort_indices(array, indices, lo, hi, |values, positions, lo, hi| {
        insertion::inplace_insertion_sort_indices(values, positions, lo, hi);
    })
}

/// `sort/quick.js#inplaceQuickSortIndices`.
#[napi(js_name = "inplaceQuickSortIndices")]
pub fn inplace_quick_sort_indices(
    array: Array<'_>,
    indices: Indices,
    lo: f64,
    hi: f64,
) -> Result<Indices> {
    sort_indices(array, indices, lo, hi, |values, positions, lo, hi| {
        quick::inplace_quick_sort_indices(values, positions, lo, hi);
    })
}

/// Read the window, sort it, write it back, hand the same array back.
///
/// Only `array[lo..hi)` is read, which is not an optimisation: upstream never
/// touches an element outside the window either, so
/// `inplaceQuickSort(['x', 2, 1], 1, 3)` must succeed despite the string.
fn sort_values<'env>(
    mut array: Array<'env>,
    lo: f64,
    hi: f64,
    sort: impl FnOnce(&mut [f64]),
) -> Result<Array<'env>> {
    let (lo, hi) = window(lo, hi, array.len() as usize, "the array")?;

    let mut values = Vec::with_capacity(hi - lo);

    for slot in lo..hi {
        values.push(element(&array, slot)?);
    }

    sort(&mut values);

    for (offset, value) in values.into_iter().enumerate() {
        array.set((lo + offset) as u32, value)?;
    }

    Ok(array)
}

/// As [`sort_values`], for the flavours that permute a separate index array.
///
/// The **whole** of `array` is read here rather than a window, because an
/// entry of `indices` may point anywhere; that is what makes the out-of-range
/// read in `mnemonist_core::sort` reachable rather than theoretical.
fn sort_indices(
    array: Array<'_>,
    indices: Indices,
    lo: f64,
    hi: f64,
    sort: impl FnOnce(&[f64], &mut PointerVec, usize, usize),
) -> Result<Indices> {
    let mut values = Vec::with_capacity(array.len() as usize);

    for slot in 0..array.len() {
        values.push(element(&array, slot as usize)?);
    }

    let mut indices = indices;
    let mut positions = read_indices(&indices);

    let (lo, hi) = window(lo, hi, positions.len(), "the indices array")?;

    sort(&values, &mut positions, lo, hi);

    write_indices(&mut indices, &positions);

    Ok(indices)
}

/// One element of the caller's array, as a number.
///
/// Both failures — not a number, and not present — are reported with the same
/// message, because they are the same limitation seen from two sides and
/// napi's own "Failed to convert napi value String into rust type `f64`" says
/// neither which slot nor that the limit is deliberate.
fn element(array: &Array<'_>, slot: usize) -> Result<f64> {
    match array.get::<f64>(slot as u32) {
        Ok(Some(value)) => Ok(value),
        Ok(None) | Err(_) => Err(Error::new(
            Status::InvalidArg,
            format!(
                "mnemonist-rs: sort element {slot} is not a number. This port's sort helpers \
                 take a dense array of numbers; upstream compares arbitrary values through \
                 valueOf, which is bridge tier T2 -- see docs/modules/sort.md."
            ),
        )),
    }
}

/// Copy the typed array out at its own width.
fn read_indices(indices: &Indices) -> PointerVec {
    match indices {
        Either3::A(values) => PointerVec::U8(values.to_vec()),
        Either3::B(values) => PointerVec::U16(values.to_vec()),
        Either3::C(values) => PointerVec::U32(values.to_vec()),
    }
}

/// Copy the sorted positions back into the caller's buffer.
///
/// Same width in as out, so no store here can truncate — but the copy is a
/// real write through the `ArrayBuffer`, which is what makes the mutation
/// visible to JavaScript rather than only to the returned handle.
fn write_indices(indices: &mut Indices, positions: &PointerVec) {
    // SAFETY (x3): `as_mut` is `unsafe` because JavaScript could mutate the
    // same buffer concurrently. Nothing here calls back into JS between taking
    // the slice and dropping it, so no other mutator can run.
    match (indices, positions) {
        (Either3::A(target), PointerVec::U8(source)) => unsafe {
            target.as_mut().copy_from_slice(source);
        },
        (Either3::B(target), PointerVec::U16(source)) => unsafe {
            target.as_mut().copy_from_slice(source);
        },
        (Either3::C(target), PointerVec::U32(source)) => unsafe {
            target.as_mut().copy_from_slice(source);
        },
        // `read_indices` produced `positions` from `indices`, so the widths
        // agree by construction and this arm is dead. It is a panic rather
        // than a silent no-op because a silent no-op would look exactly like
        // "the sort did nothing", which is the hardest failure to notice.
        _ => unreachable!("the indices width cannot change between read and write"),
    }
}

/// Validate `lo`/`hi` against the target's length.
///
/// Upstream validates nothing and walks off the end. This refuses instead, and
/// says so — see the module docs, point 3.
fn window(lo: f64, hi: f64, len: usize, what: &str) -> Result<(usize, usize)> {
    let lo = offset(lo, len, what, "lo")?;
    let hi = offset(hi, len, what, "hi")?;

    if lo > hi {
        return Err(Error::new(
            Status::InvalidArg,
            format!(
                "mnemonist-rs: sort window lo={lo} is past hi={hi}. Upstream would treat it as \
                 empty; this port refuses it -- see docs/modules/sort.md."
            ),
        ));
    }

    Ok((lo, hi))
}

fn offset(value: f64, len: usize, what: &str, name: &str) -> Result<usize> {
    if value.fract() == 0.0 && value >= 0.0 && value <= len as f64 {
        return Ok(value as usize);
    }

    Err(Error::new(
        Status::InvalidArg,
        format!(
            "mnemonist-rs: sort bound {name}={value} is not an integer in 0..={len} for {what}. \
             Upstream reads `undefined` past the end and writes into holes, which has no Rust \
             representation -- see docs/modules/sort.md."
        ),
    ))
}

/// A pointer array of `length` slots filled with `0..length`.
///
/// `utils/typed-arrays.js#indices`, the third module in `test/sort.js`'s
/// require-closure. Lives here rather than in a `typed_arrays` bridge module
/// because it is the only export of that file this unit reaches, and a bridge
/// module holding one function that only `sort` calls would be a directory
/// entry pretending to be a boundary.
///
/// Named `typedArraysIndices` in the addon's flat namespace and mapped back to
/// `indices` by `tests/bridge/utils/typed-arrays.js`: `indices` is far too
/// generic a name to claim at the top level of an addon that will eventually
/// export forty modules' worth of helpers.
#[napi(js_name = "typedArraysIndices")]
pub fn typed_arrays_indices(length: f64) -> Result<Indices> {
    // Both throws come from the core function, in upstream's own order:
    // `getPointerArray` runs before the TypedArray constructor, so `5e9`
    // reports mnemonist's message rather than a RangeError.
    let filled =
        mnemonist_core::utils::typed_arrays::indices(length).map_err(|error| match error {
            IndicesError::TooLarge => {
                Error::new(Status::GenericFailure, POINTER_ARRAY_TOO_LARGE.to_owned())
            }
            IndicesError::InvalidLength(value) => Error::new(
                Status::InvalidArg,
                format!("{INVALID_TYPED_ARRAY_LENGTH}: {value}"),
            ),
        })?;

    Ok(match filled {
        PointerVec::U8(values) => Either3::A(Uint8Array::new(values)),
        PointerVec::U16(values) => Either3::B(Uint16Array::new(values)),
        PointerVec::U32(values) => Either3::C(Uint32Array::new(values)),
    })
}

//! JS bridge for the three `utils/typed-arrays.js` exports `test/_utils.js`
//! reaches that `crate::sort` does not: `getPointerArray`, `getMinimalRepresentation`
//! and `concat`. `sort.rs` already bridges `indices` (the only export
//! `test/sort.js` uses); this file is the rest, added as `_utils` reaches them
//! — same policy `mnemonist_core::utils::typed_arrays`'s own module docs
//! state.
//!
//! # `getPointerArray`/`getMinimalRepresentation` return the constructor itself
//!
//! `test/_utils.js` asserts `typed.getPointerArray(min) === Uint8Array` with
//! `assert.strictEqual` — the real global constructor, not an instance of it
//! (contrast `indices`, which returns a filled typed array). So both
//! functions here fetch the constructor off `globalThis` by name, exactly as
//! [`crate::iterables::allocate`] does for `toArray`'s `new Array(l)`.
//!
//! # `concat` supports `Uint8Array` only
//!
//! Upstream is generic over any typed-array class via
//! `new (arguments[0].constructor)(length)`; `test/_utils.js`'s own
//! `#.concat` case only ever constructs `Uint8Array`s. Supporting every
//! numeric typed array would mean dispatching on the runtime class of an
//! `Unknown` argument for a capability nothing in scope exercises — so this
//! bridges `Vec<Uint8Array>` directly, and widening it is future work for
//! whichever module needs a different element class (D-style simplification,
//! same policy as `indices`).

use mnemonist_core::utils::typed_arrays::{self, NumberType, PointerWidth};
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// `typed-arrays.js#getPointerArray` — the real global constructor, chosen by
/// [`typed_arrays::get_pointer_array`].
#[napi(js_name = "typedArraysGetPointerArray")]
pub fn js_get_pointer_array<'env>(env: &'env Env, size: f64) -> Result<Unknown<'env>> {
    let width = typed_arrays::get_pointer_array(size)
        .map_err(|message| Error::new(Status::GenericFailure, message.to_owned()))?;

    constructor_for(env, class_name(width))
}

/// `typed-arrays.js#getMinimalRepresentation` — `null` for an empty array,
/// matching upstream's `maxType` starting `null` and never being set.
#[napi(js_name = "typedArraysGetMinimalRepresentation")]
pub fn js_get_minimal_representation<'env>(
    env: &'env Env,
    values: Vec<f64>,
) -> Result<Either<Unknown<'env>, Null>> {
    match typed_arrays::get_minimal_representation(&values) {
        None => Ok(Either::B(Null)),
        Some(kind) => Ok(Either::A(constructor_for(
            env,
            number_type_class_name(kind),
        )?)),
    }
}

/// `typed-arrays.js#concat` — see the module docs for the `Uint8Array`-only
/// scope. The shim spreads the variadic call into this `Vec`, the same shape
/// `crate::set`'s `setIntersection`/`setUnion` already established for a
/// variadic upstream export.
#[napi(js_name = "typedArraysConcat")]
pub fn js_concat(arrays: Vec<Uint8Array>) -> Uint8Array {
    let slices: Vec<&[u8]> = arrays.iter().map(|array| array.as_ref()).collect();

    Uint8Array::new(typed_arrays::concat(&slices))
}

fn class_name(width: PointerWidth) -> &'static str {
    match width {
        PointerWidth::U8 => "Uint8Array",
        PointerWidth::U16 => "Uint16Array",
        PointerWidth::U32 => "Uint32Array",
    }
}

fn number_type_class_name(kind: NumberType) -> &'static str {
    match kind {
        NumberType::U8 => "Uint8Array",
        NumberType::I8 => "Int8Array",
        NumberType::U16 => "Uint16Array",
        NumberType::I16 => "Int16Array",
        NumberType::U32 => "Uint32Array",
        NumberType::I32 => "Int32Array",
        NumberType::F32 => "Float32Array",
        NumberType::F64 => "Float64Array",
    }
}

/// `globalThis.<class>`, as a value the caller can `assert.strictEqual`
/// against the real constructor.
fn constructor_for<'env>(env: &'env Env, class: &str) -> Result<Unknown<'env>> {
    let global = env.get_global()?;

    global.get_named_property_unchecked(class)
}

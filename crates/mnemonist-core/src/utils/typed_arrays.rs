//! Port of upstream `utils/typed-arrays.js`.
//!
//! Upstream returns a JavaScript TypedArray *constructor* (`Uint8Array` and
//! friends). Rust has no equivalent runtime value, so the width is returned as
//! an enum and the napi bridge maps it back to the corresponding JS type.
//!
//! Only the helpers actually reached by ported modules live here; the rest of
//! the upstream file is added as modules need it.

/// The unsigned pointer widths upstream selects between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerWidth {
    U8,
    U16,
    U32,
}

const MAX_8BIT_INTEGER: f64 = 255.0;
const MAX_16BIT_INTEGER: f64 = 65_535.0;
const MAX_32BIT_INTEGER: f64 = 4_294_967_295.0;

/// Upstream throw message, reproduced verbatim so the bridge can surface an
/// identical `Error` to JS callers.
pub const POINTER_ARRAY_TOO_LARGE: &str =
    "mnemonist: Pointer Array of size > 4294967295 is not supported.";

/// Choose the narrowest unsigned width able to index `size` elements.
///
/// Takes `f64` rather than an integer type on purpose: upstream calls this as
/// `getPointerArray(Math.log2(size))`, so a non-integral argument is a normal
/// input, not an edge case. `NaN` propagates to `Err` here exactly as it falls
/// through to the `throw` upstream, because every `NaN` comparison is false in
/// both languages.
pub fn get_pointer_array(size: f64) -> Result<PointerWidth, &'static str> {
    let max_index = size - 1.0;

    if max_index <= MAX_8BIT_INTEGER {
        return Ok(PointerWidth::U8);
    }

    if max_index <= MAX_16BIT_INTEGER {
        return Ok(PointerWidth::U16);
    }

    if max_index <= MAX_32BIT_INTEGER {
        return Ok(PointerWidth::U32);
    }

    Err(POINTER_ARRAY_TOO_LARGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_boundaries_exactly_as_upstream() {
        // maxIndex = size - 1, compared against 2^n - 1.
        assert_eq!(get_pointer_array(1.0), Ok(PointerWidth::U8));
        assert_eq!(get_pointer_array(256.0), Ok(PointerWidth::U8));
        assert_eq!(get_pointer_array(257.0), Ok(PointerWidth::U16));
        assert_eq!(get_pointer_array(65_536.0), Ok(PointerWidth::U16));
        assert_eq!(get_pointer_array(65_537.0), Ok(PointerWidth::U32));
        assert_eq!(get_pointer_array(4_294_967_296.0), Ok(PointerWidth::U32));
    }

    #[test]
    fn rejects_beyond_u32() {
        assert_eq!(
            get_pointer_array(4_294_967_297.0),
            Err(POINTER_ARRAY_TOO_LARGE)
        );
    }

    #[test]
    fn accepts_non_integral_input() {
        // StaticDisjointSet sizes its ranks array with Math.log2(size).
        assert_eq!(get_pointer_array(10f64.log2()), Ok(PointerWidth::U8));
        assert_eq!(get_pointer_array(0.0), Ok(PointerWidth::U8));
    }

    #[test]
    fn nan_falls_through_to_error_like_upstream() {
        assert_eq!(get_pointer_array(f64::NAN), Err(POINTER_ARRAY_TOO_LARGE));
    }
}

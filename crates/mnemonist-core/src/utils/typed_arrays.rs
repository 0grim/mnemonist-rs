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

/// A fixed-width unsigned integer vector, standing in for the JS typed arrays
/// upstream allocates (`Uint8Array` / `Uint16Array` / `Uint32Array`).
///
/// # Why an enum over three `Vec`s rather than one `Vec<u32>` and a mask
///
/// A masked `Vec<u32>` reproduces the *truncation* correctly but gets the
/// *memory footprint* wrong by up to 4x, and the footprint is observable as
/// latency. Measured on `static-disjoint-set` at 4e6 items: upstream's
/// `Uint8Array` ranks array is 4 MB where a `Vec<u32>` is 16 MB, taking the
/// structure from 20 MB to 32 MB and straight through the host's 32 MB L3.
/// The result was a p99 2.7x worse than upstream while p50 stayed 1.7x better
/// -- a tail regression caused entirely by the representation. See
/// `docs/modules/static-disjoint-set.md`.
///
/// With a real per-width backing store the narrowing cast in [`PointerVec::set`]
/// **is** the truncation, so the mask stops being merely correct and becomes
/// unnecessary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerVec {
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
}

impl PointerVec {
    /// Allocate `len` zeroed slots at the given width.
    ///
    /// # Panics
    ///
    /// Aborts through the global allocator if `len` slots do not fit in
    /// memory. Upstream throws a catchable `RangeError` there; stable Rust has
    /// no fallible `Vec` allocation, so callers accepting untrusted sizes
    /// should bound them beforehand.
    pub fn zeroed(width: PointerWidth, len: usize) -> Self {
        match width {
            PointerWidth::U8 => Self::U8(vec![0; len]),
            PointerWidth::U16 => Self::U16(vec![0; len]),
            PointerWidth::U32 => Self::U32(vec![0; len]),
        }
    }

    /// Width this vector was allocated at, i.e. which JS typed array it stands
    /// in for.
    pub fn width(&self) -> PointerWidth {
        match self {
            Self::U8(_) => PointerWidth::U8,
            Self::U16(_) => PointerWidth::U16,
            Self::U32(_) => PointerWidth::U32,
        }
    }

    /// Read one slot, widened to `u32`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds. Upstream reads past the end of a
    /// typed array instead and gets `undefined`, which then propagates `NaN`
    /// through whatever arithmetic follows; there is no honest Rust
    /// reproduction of that, so the read is checked. Structures that need
    /// upstream's out-of-range *behaviour* reproduce it by guarding before the
    /// call, not by making this method lenient.
    pub fn get(&self, index: usize) -> u32 {
        match self {
            Self::U8(values) => u32::from(values[index]),
            Self::U16(values) => u32::from(values[index]),
            Self::U32(values) => values[index],
        }
    }

    /// Truncating write, mirroring a JS typed array store.
    ///
    /// The narrowing cast is the whole mechanism: `Uint8Array` writes take the
    /// value mod 256, and so does `value as u8`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds, as [`PointerVec::get`] does.
    /// Upstream's out-of-range typed-array *write* is a silent no-op, which is
    /// again reproduced by guarding at the call site where a structure needs
    /// it.
    pub fn set(&mut self, index: usize, value: u32) {
        match self {
            Self::U8(values) => values[index] = value as u8,
            Self::U16(values) => values[index] = value as u16,
            Self::U32(values) => values[index] = value,
        }
    }

    /// Number of slots, i.e. the JS typed array's `length`.
    pub fn len(&self) -> usize {
        match self {
            Self::U8(values) => values.len(),
            Self::U16(values) => values.len(),
            Self::U32(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes of backing storage, as the equivalent typed array would occupy.
    ///
    /// Exists so the footprint claim above is testable rather than asserted.
    pub fn byte_len(&self) -> usize {
        self.len()
            * match self {
                Self::U8(_) => 1,
                Self::U16(_) => 2,
                Self::U32(_) => 4,
            }
    }
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

    #[test]
    fn writes_truncate_at_the_selected_width() {
        let mut eight = PointerVec::zeroed(PointerWidth::U8, 1);
        eight.set(0, 300);
        assert_eq!(eight.get(0), 300 % 256);

        let mut sixteen = PointerVec::zeroed(PointerWidth::U16, 1);
        sixteen.set(0, 70_000);
        assert_eq!(sixteen.get(0), 70_000 % 65_536);

        let mut thirty_two = PointerVec::zeroed(PointerWidth::U32, 1);
        thirty_two.set(0, u32::MAX);
        assert_eq!(thirty_two.get(0), u32::MAX);
    }

    /// The reason the representation is an enum rather than a masked
    /// `Vec<u32>`: the mask gets truncation right and the footprint wrong, and
    /// the footprint was measurable as a 2.7x p99 regression at 4e6 items.
    #[test]
    fn footprint_matches_the_equivalent_typed_array() {
        assert_eq!(PointerVec::zeroed(PointerWidth::U8, 1000).byte_len(), 1000);
        assert_eq!(PointerVec::zeroed(PointerWidth::U16, 1000).byte_len(), 2000);
        assert_eq!(PointerVec::zeroed(PointerWidth::U32, 1000).byte_len(), 4000);
    }

    #[test]
    fn zeroed_reports_its_own_width_and_length() {
        let values = PointerVec::zeroed(PointerWidth::U16, 3);

        assert_eq!(values.width(), PointerWidth::U16);
        assert_eq!(values.len(), 3);
        assert!(!values.is_empty());
        assert!(PointerVec::zeroed(PointerWidth::U8, 0).is_empty());
        assert_eq!(values, PointerVec::U16(vec![0, 0, 0]));
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn reads_past_the_end_panic_rather_than_yielding_undefined() {
        PointerVec::zeroed(PointerWidth::U8, 2).get(2);
    }
}

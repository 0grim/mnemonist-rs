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

/// Upstream's message when a typed array is asked for an impossible length.
///
/// `new Uint8Array(-1)` and `new Uint8Array(3.5)` both throw a `RangeError`
/// reading `Invalid typed array length: <value>`. Only the fixed prefix is
/// reproduced; the bridge appends the offending value.
pub const INVALID_TYPED_ARRAY_LENGTH: &str = "Invalid typed array length";

/// A pointer array of `length` slots, filled with `0..length`.
///
/// Upstream's `exports.indices`, which is `getPointerArray(length)` followed
/// by `new PointerArray(length)` and a fill loop. The width is chosen to index
/// `length` elements, so the fill never truncates — the largest value written
/// is `length - 1`, which is precisely what the width was selected to hold.
///
/// # Errors
///
/// [`POINTER_ARRAY_TOO_LARGE`] for a length past `2³²`, as upstream throws.
pub fn indices(length: usize) -> Result<PointerVec, &'static str> {
    let width = get_pointer_array(length as f64)?;
    let mut array = PointerVec::zeroed(width, length);

    for slot in 0..length {
        array.set(slot, slot as u32);
    }

    Ok(array)
}

/// A value that can be stored into a JS typed array, and read back out.
///
/// Upstream `SparseMap` is constructed with a *value array constructor* —
/// `Array` by default, but the test file also uses `Uint8Array` — and the two
/// behave differently on a store. A plain `Array` keeps whatever it is given;
/// a typed array runs the operand through `ToUint32` and then narrows it to the
/// element width. This trait is the first half of that, the part that depends
/// on the value's own type; [`PointerVec::set`] is the second.
///
/// Implemented for `u32` (identity — the fuzzer's value type) and `f64` (the
/// real conversion — every JS number is one, which is what the bridge sees).
pub trait TypedValue: Copy + PartialEq {
    /// JS `ToUint32`: truncate towards zero, then take the result modulo 2³².
    ///
    /// `NaN` and the infinities are `0`, and negatives wrap rather than
    /// saturating — `-1` is `4294967295`, which then narrows to `255` in a
    /// `Uint8Array`. Rust's `as` cast saturates instead, so it cannot be used
    /// directly here.
    fn to_uint32(self) -> u32;

    /// Read back: a typed array always yields a non-negative integer.
    fn from_uint32(raw: u32) -> Self;
}

impl TypedValue for u32 {
    fn to_uint32(self) -> u32 {
        self
    }

    fn from_uint32(raw: u32) -> Self {
        raw
    }
}

/// Two-to-the-thirty-two, as the modulus `ToUint32` is defined against.
const TWO_POW_32: f64 = 4_294_967_296.0;

impl TypedValue for f64 {
    fn to_uint32(self) -> u32 {
        if !self.is_finite() {
            // ToUint32(NaN) = ToUint32(±Infinity) = +0.
            return 0;
        }

        // `rem_euclid` lands in `[0, 2^32)` for negatives too, which is
        // precisely the wrap `as u32` would have turned into a saturation.
        self.trunc().rem_euclid(TWO_POW_32) as u32
    }

    fn from_uint32(raw: u32) -> Self {
        f64::from(raw)
    }
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

    /// Read one slot, or `None` where JS would have produced `undefined`.
    ///
    /// The counterpart to [`PointerVec::get`], for the structures whose
    /// upstream code *relies* on the out-of-range read. `SparseSet` is the
    /// first: `this.sparse[member]` for a member past the array is `undefined`
    /// there, every comparison against `undefined` is false, and both `has`
    /// and `delete` fall out of their guards because of it. Reproducing that
    /// needs the read to be an [`Option`], not a panic.
    pub fn try_get(&self, index: usize) -> Option<u32> {
        match self {
            Self::U8(values) => values.get(index).copied().map(u32::from),
            Self::U16(values) => values.get(index).copied().map(u32::from),
            Self::U32(values) => values.get(index).copied(),
        }
    }

    /// Truncating write, silently dropped when out of range.
    ///
    /// A JS typed-array store past the end is a **no-op** — no throw, no
    /// growth, and in sloppy mode not even an error. [`PointerVec::set`]
    /// panics instead, which is right where a structure's own invariants say
    /// the index is good. This is for the places where upstream's own logic
    /// walks off the end and keeps going, and the no-op is load-bearing:
    /// `SparseSet.add(member)` with `member >= length` writes `sparse[member]`
    /// into the void, increments `size` anyway, and leaves the set in a state
    /// only this method reproduces.
    ///
    /// Returns whether the write landed.
    pub fn try_set(&mut self, index: usize, value: u32) -> bool {
        match self {
            Self::U8(values) => values.get_mut(index).map(|slot| *slot = value as u8),
            Self::U16(values) => values.get_mut(index).map(|slot| *slot = value as u16),
            Self::U32(values) => values.get_mut(index).map(|slot| *slot = value),
        }
        .is_some()
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

    /// `ToUint32`, pinned against Node 24.18.1 — `a = new Uint32Array(1);
    /// a[0] = v` for each `v` below. The negatives are the ones a plain `as`
    /// cast gets wrong: Rust saturates them to `0`, JS wraps them.
    #[test]
    fn to_uint32_wraps_where_an_as_cast_would_saturate() {
        for (input, expected) in [
            (0.0f64, 0u32),
            (13.0, 13),
            (13.9, 13),
            (-1.0, 4_294_967_295),
            (-1.5, 4_294_967_295),
            (-0.5, 0),
            (255.0, 255),
            (256.0, 256),
            (300.7, 300),
            (4_294_967_295.0, 4_294_967_295),
            (4_294_967_296.0, 0),
            (4_294_967_297.0, 1),
            (-4_294_967_296.0, 0),
            (1e21, 3_735_027_712),
            (f64::NAN, 0),
            (f64::INFINITY, 0),
            (f64::NEG_INFINITY, 0),
        ] {
            assert_eq!(input.to_uint32(), expected, "ToUint32({input})");
        }

        // The identity impl, so the fuzzer's value type costs nothing.
        assert_eq!(7u32.to_uint32(), 7);
        assert_eq!(u32::from_uint32(7), 7);
        assert_eq!(f64::from_uint32(7), 7.0);
    }

    /// `ToUint32` and the narrowing store compose to the JS element store:
    /// `b = new Uint8Array(1); b[0] = -1.5` is `255`.
    #[test]
    fn to_uint32_then_a_narrowing_store_is_the_js_element_store() {
        for (input, expected) in [(-1.5f64, 255u32), (300.7, 44), (256.0, 0), (1e21, 0)] {
            let mut values = PointerVec::zeroed(PointerWidth::U8, 1);
            values.set(0, input.to_uint32());

            assert_eq!(values.get(0), expected, "Uint8Array store of {input}");
        }
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

    /// `indices(n)` picks the width from `n` and then cannot truncate, because
    /// the largest value it writes is `n - 1`. Checked at both sides of each
    /// boundary, since that is where a width off by one would show.
    #[test]
    fn indices_fills_with_its_own_positions_at_every_width() {
        for length in [0usize, 1, 255, 256, 257, 65_535, 65_536, 65_537] {
            let array = indices(length).expect("length fits a pointer array");

            assert_eq!(array.len(), length, "length {length}");
            assert_eq!(
                array.width(),
                get_pointer_array(length as f64).unwrap(),
                "width for {length}"
            );

            for slot in 0..length {
                assert_eq!(array.get(slot), slot as u32, "slot {slot} of {length}");
            }
        }
    }

    #[test]
    fn indices_refuses_a_length_no_pointer_array_can_index() {
        assert_eq!(
            indices(4_294_967_297).map(|array| array.len()),
            Err(POINTER_ARRAY_TOO_LARGE)
        );
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn reads_past_the_end_panic_rather_than_yielding_undefined() {
        PointerVec::zeroed(PointerWidth::U8, 2).get(2);
    }

    /// The opt-in form, for structures that depend on `undefined`.
    #[test]
    fn try_get_reports_the_out_of_range_read_instead_of_panicking() {
        for width in [PointerWidth::U8, PointerWidth::U16, PointerWidth::U32] {
            let mut values = PointerVec::zeroed(width, 2);
            values.set(1, 7);

            assert_eq!(values.try_get(1), Some(7));
            assert_eq!(values.try_get(2), None);
            assert_eq!(values.try_get(usize::MAX), None);
        }
    }

    /// A JS typed-array store past the end is a no-op, not a throw.
    #[test]
    fn try_set_drops_out_of_range_writes_and_still_truncates_in_range_ones() {
        let mut values = PointerVec::zeroed(PointerWidth::U8, 2);

        assert!(values.try_set(0, 300));
        assert_eq!(values.try_get(0), Some(300 % 256));

        assert!(!values.try_set(2, 1));
        assert_eq!(values, PointerVec::U8(vec![(300u32 % 256) as u8, 0]));

        let mut wide = PointerVec::zeroed(PointerWidth::U16, 1);
        assert!(wide.try_set(0, 70_000));
        assert_eq!(wide.try_get(0), Some(70_000 % 65_536));
        assert!(!wide.try_set(9, 1));

        let mut widest = PointerVec::zeroed(PointerWidth::U32, 1);
        assert!(widest.try_set(0, u32::MAX));
        assert_eq!(widest.try_get(0), Some(u32::MAX));
        assert!(!widest.try_set(1, 1));
    }
}

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
/// `new Uint8Array(-1)` throws a `RangeError` reading
/// `Invalid typed array length: -1`. Only the fixed prefix is a constant; the
/// offending value is carried by [`IndicesError::InvalidLength`].
pub const INVALID_TYPED_ARRAY_LENGTH: &str = "Invalid typed array length";

/// The two throws [`indices`] can reach, which are thrown by different things.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndicesError {
    /// `getPointerArray` refused the length. Message: [`POINTER_ARRAY_TOO_LARGE`].
    TooLarge,
    /// The TypedArray constructor refused it. Message:
    /// [`INVALID_TYPED_ARRAY_LENGTH`], then the offending value.
    InvalidLength(f64),
}

/// A pointer array of `length` slots, filled with its own positions.
///
/// Upstream's `exports.indices` — `getPointerArray(length)`, then
/// `new PointerArray(length)`, then `for (i = 0; i < length; i++) array[i] = i`.
///
/// # The two lengths, which are not the same length
///
/// Takes `f64` because upstream's two uses of the argument coerce it
/// *differently*, and the difference is observable:
///
/// * `getPointerArray` compares `length - 1` as a double, so it sees `256.5`;
/// * the TypedArray constructor applies `ToIndex`, which **truncates**, so it
///   allocates 256 slots.
///
/// `indices(256.5)` is therefore a `Uint16Array` of 256 elements — a width
/// wider than 256 elements need. Confirmed against Node 24.18.1. Taking
/// `usize` here and letting the bridge truncate first would silently produce a
/// `Uint8Array` instead.
///
/// The fill loop runs on the untruncated bound too, so its last store lands
/// past the end of the array and is dropped, exactly as a JS typed-array store
/// past the end is. That makes it unobservable, which is why the loop below is
/// written against the allocated length.
///
/// # Errors
///
/// [`IndicesError::TooLarge`] for a length past `2³²`, `NaN`, or either
/// infinity — every comparison in `getPointerArray` is false for `NaN`, so it
/// falls through to the same throw. [`IndicesError::InvalidLength`] for a
/// negative one; `-0.5` is *not* negative after truncation and yields an empty
/// array, as it does upstream.
pub fn indices(length: f64) -> Result<PointerVec, IndicesError> {
    let width = get_pointer_array(length).map_err(|_| IndicesError::TooLarge)?;

    // `ToIndex`: truncate towards zero, then reject anything still negative.
    let truncated = length.trunc();

    if truncated < 0.0 {
        return Err(IndicesError::InvalidLength(length));
    }

    let count = truncated as usize;
    let mut array = PointerVec::zeroed(width, count);

    for slot in 0..count {
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

/// The eight typed-array element widths `getNumberType`/`getMinimalRepresentation`
/// choose between.
///
/// Upstream returns the constructor itself (`Uint8Array` and friends); the
/// napi bridge maps this enum back to the real global constructor, as
/// [`get_pointer_array`]'s width already does for [`PointerWidth`].
///
/// `F32` exists only to complete the priority table [`get_minimal_representation`]
/// walks -- upstream's own `TYPE_PRIORITY` dictionary has a `Float32Array`
/// slot between `Int32Array` and `Float64Array`, but [`get_number_type`] can
/// never produce it: every non-integral or out-of-`i32`-range value falls
/// straight through to `Float64Array`. So the slot is dead code upstream too,
/// not a gap this port introduces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberType {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    F32,
    F64,
}

/// `TYPE_PRIORITY`'s values, in the same order upstream lists them.
fn priority(kind: NumberType) -> u8 {
    match kind {
        NumberType::U8 => 1,
        NumberType::I8 => 2,
        NumberType::U16 => 3,
        NumberType::I16 => 4,
        NumberType::U32 => 5,
        NumberType::I32 => 6,
        NumberType::F32 => 7,
        NumberType::F64 => 8,
    }
}

/// The narrowest typed-array element able to represent `value` exactly.
///
/// `getNumberType(value)`. `value === (value | 0)` is JavaScript's ToInt32
/// round-trip check -- true exactly when `value` is already a 32-bit integer
/// -- reused here as [`crate::utils::bitwise::to_int32`], the same conversion
/// `utils/bitwise.js` is built on. `Math.sign(value) === -1` reduces to "the
/// round-tripped integer is negative": `to_int32` never produces a negative
/// zero (there is no such `i32`), so `-0` takes the non-negative branch here
/// exactly as `Math.sign(-0) === -0 !== -1` does upstream.
pub fn get_number_type(value: f64) -> NumberType {
    let truncated = super::bitwise::to_int32(value);

    if value != f64::from(truncated) {
        return NumberType::F64;
    }

    if truncated < 0 {
        if truncated >= -128 {
            NumberType::I8
        } else if truncated >= -32_768 {
            NumberType::I16
        } else {
            NumberType::I32
        }
    } else if truncated <= 255 {
        NumberType::U8
    } else if truncated <= 65_535 {
        NumberType::U16
    } else {
        NumberType::U32
    }
}

/// The narrowest typed-array element able to represent every value in
/// `values`.
///
/// `getMinimalRepresentation(array)` -- upstream's optional second `getter`
/// argument is not ported: `test/_utils.js` never supplies one, and every
/// call site in the modules ported so far passes a plain array of numbers
/// (D-style simplification noted in the module docs, same policy as
/// [`indices`]: helpers land as callers reach them).
///
/// Returns `None` for an empty slice, matching upstream's `null` (its own
/// `maxType` starts `null` and the loop that would set it never runs).
pub fn get_minimal_representation(values: &[f64]) -> Option<NumberType> {
    let mut max_type = None;
    let mut max_priority = 0u8;

    for &value in values {
        let kind = get_number_type(value);
        let rank = priority(kind);

        if rank > max_priority {
            max_priority = rank;
            max_type = Some(kind);
        }
    }

    max_type
}

/// Concatenate byte/typed arrays into one, in argument order.
///
/// `exports.concat`. Upstream allocates the result with
/// `new (arguments[0].constructor)(length)`, i.e. the same typed-array class
/// as its first argument; a Rust caller already knows `T` is uniform across
/// every slice, so there is no analogous "which constructor" question here --
/// the bridge picks the JS output class from the first argument's real type.
pub fn concat<T: Clone>(arrays: &[&[T]]) -> Vec<T> {
    let total: usize = arrays.iter().map(|a| a.len()).sum();
    let mut out = Vec::with_capacity(total);

    for array in arrays {
        out.extend_from_slice(array);
    }

    out
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
            let array = indices(length as f64).expect("length fits a pointer array");

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

    /// The width is chosen from the raw length and the allocation from the
    /// truncated one, so a fractional length can be one width too wide.
    /// Pinned against Node 24.18.1: `indices(256.5)` is a `Uint16Array` of 256
    /// elements, and `indices(255.5)` a `Uint8Array` of 255.
    #[test]
    fn indices_truncates_its_length_but_not_its_width() {
        let wide = indices(256.5).unwrap();
        assert_eq!(wide.width(), PointerWidth::U16);
        assert_eq!(wide.len(), 256);
        assert_eq!(wide.get(255), 255);

        let narrow = indices(255.5).unwrap();
        assert_eq!(narrow.width(), PointerWidth::U8);
        assert_eq!(narrow.len(), 255);

        assert_eq!(indices(3.5).unwrap(), PointerVec::U8(vec![0, 1, 2]));

        // `-0.5` truncates to `-0`, which `ToIndex` accepts as zero.
        assert_eq!(indices(-0.5).unwrap(), PointerVec::U8(vec![]));
    }

    #[test]
    fn indices_refuses_the_lengths_upstream_refuses() {
        assert_eq!(indices(4_294_967_297.0), Err(IndicesError::TooLarge));
        assert_eq!(indices(f64::NAN), Err(IndicesError::TooLarge));
        assert_eq!(indices(f64::INFINITY), Err(IndicesError::TooLarge));
        assert_eq!(indices(4_294_967_296.5), Err(IndicesError::TooLarge));

        assert_eq!(indices(-1.0), Err(IndicesError::InvalidLength(-1.0)));
        assert_eq!(indices(-3.5), Err(IndicesError::InvalidLength(-3.5)));
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

    // ----------------------------------------------------------- NumberType

    /// The one case `test/_utils.js` pins for `getMinimalRepresentation`,
    /// transcribed: three arrays of numbers, each expected to settle on a
    /// specific typed-array class.
    #[test]
    fn get_minimal_representation_matches_the_upstream_suites_own_cases() {
        assert_eq!(
            get_minimal_representation(&[1.0, 2.0, 3.0, 4.0, 5.0]),
            Some(NumberType::U8)
        );
        assert_eq!(
            get_minimal_representation(&[1.0, 2.0, -3.0, 4.0, 5.0]),
            Some(NumberType::I8)
        );
        assert_eq!(
            get_minimal_representation(&[1.0, 3.0, 4.0, 3.4]),
            Some(NumberType::F64)
        );
    }

    #[test]
    fn get_minimal_representation_of_empty_is_none() {
        assert_eq!(get_minimal_representation(&[]), None);
    }

    /// `getNumberType` at every boundary the upstream priority table
    /// distinguishes, unsigned and signed.
    #[test]
    fn get_number_type_selects_every_boundary() {
        assert_eq!(get_number_type(0.0), NumberType::U8);
        assert_eq!(get_number_type(255.0), NumberType::U8);
        assert_eq!(get_number_type(256.0), NumberType::U16);
        assert_eq!(get_number_type(65_535.0), NumberType::U16);
        assert_eq!(get_number_type(65_536.0), NumberType::U32);
        // `Uint32Array` tops out at `i32::MAX`, not `u32::MAX`: `value | 0`
        // (ToInt32) wraps anything from 2^31 up into a negative `i32`, which
        // breaks the `value === (value | 0)` round trip and sends it to
        // `Float64Array` instead -- verified against Node 24.18.1
        // (`getNumberType(2147483648).name === 'Float64Array'`).
        assert_eq!(get_number_type(2_147_483_647.0), NumberType::U32);
        assert_eq!(get_number_type(2_147_483_648.0), NumberType::F64);
        assert_eq!(get_number_type(4_294_967_295.0), NumberType::F64);

        assert_eq!(get_number_type(-1.0), NumberType::I8);
        assert_eq!(get_number_type(-128.0), NumberType::I8);
        assert_eq!(get_number_type(-129.0), NumberType::I16);
        assert_eq!(get_number_type(-32_768.0), NumberType::I16);
        assert_eq!(get_number_type(-32_769.0), NumberType::I32);
        assert_eq!(get_number_type(-2_147_483_648.0), NumberType::I32);
        // Symmetrically, one below `i32::MIN` also breaks the round trip.
        assert_eq!(get_number_type(-2_147_483_649.0), NumberType::F64);

        assert_eq!(get_number_type(3.4), NumberType::F64);
        assert_eq!(get_number_type(f64::NAN), NumberType::F64);
        assert_eq!(get_number_type(1e21), NumberType::F64);
    }

    /// `Math.sign(-0) === -0`, not `-1` -- so `-0` takes the non-negative
    /// branch, exactly as a plain `0` does. Verified against the ToInt32
    /// round trip: `to_int32(-0.0)` is the plain (non-negative) `i32` `0`.
    #[test]
    fn negative_zero_takes_the_non_negative_branch() {
        assert_eq!(get_number_type(-0.0), NumberType::U8);
    }

    // --------------------------------------------------------------- concat

    /// `test/_utils.js`'s own `#.concat` case, transcribed.
    #[test]
    fn concat_matches_the_upstream_suites_own_case() {
        let a: [u8; 3] = [1, 2, 3];
        let b: [u8; 2] = [4, 5];
        let c: [u8; 3] = [5, 5, 6];

        let ab: [&[u8]; 2] = [&a, &b];
        let abc: [&[u8]; 3] = [&a, &b, &c];
        let ba: [&[u8]; 2] = [&b, &a];

        assert_eq!(concat(&ab), vec![1, 2, 3, 4, 5]);
        assert_eq!(concat(&abc), vec![1, 2, 3, 4, 5, 5, 5, 6]);
        assert_eq!(concat(&ba), vec![4, 5, 1, 2, 3]);
    }

    #[test]
    fn concat_of_nothing_is_empty() {
        let empty: [&[u8]; 0] = [];
        assert_eq!(concat(&empty), Vec::<u8>::new());
    }
}

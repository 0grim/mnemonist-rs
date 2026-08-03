//! Port of upstream `utils/bitwise.js` (mnemonist v0.40.4).
//!
//! Nine small helpers: two most-significant-bit scans, two population counts,
//! a bit test, and the critical-bit machinery `critbit-tree-map` uses. There is
//! **no `test/bitwise.js`** upstream — not a thin test file, none at all — so
//! every line below is reached by the repo's own suite only indirectly, through
//! `bit-set`'s and `bit-vector`'s `rank`, which call `table8Popcount` and
//! nothing else.
//!
//! # These functions take `f64`, and that is not an accident
//!
//! Every one of them is written in terms of JavaScript's bitwise operators, and
//! every JavaScript bitwise operator begins by converting its operand with
//! **ToInt32** — truncate toward zero, take modulo 2^32, reinterpret as signed.
//! The results are then `i32`, including where that is visibly wrong (see
//! [`critical_bit32_mask`]). Taking `u32` here would quietly delete the
//! conversion, and the conversion is where three of the four defects below
//! live. [`to_int32`] and [`to_uint32`] are exposed so a caller can see exactly
//! what is happening rather than inferring it.
//!
//! # Four defects, all verified against Node 24.18.1
//!
//! | function | input | upstream | what it should be |
//! |---|---|---|---|
//! | [`msb32`] | any value whose bit 31 is set | `0` | `-2147483648` |
//! | [`msb8`] | anything above a byte | unmasked garbage (`msb8(256) == 256`) | documented as byte-only |
//! | [`critical_bit32_mask`] | anything | a **negative** `i32` | an unsigned mask |
//! | [`popcount`] | — | correct | — |
//!
//! `msb32` is the sharp one. `x |= x >> 1` is an *arithmetic* shift, so an input
//! with the top bit set smears to `-1` at the first step, and `-1 & ~(-1 >> 1)`
//! is `-1 & 0`, which is `0`. So `msb32` reports "no bits set" for exactly the
//! half of the 32-bit range where the answer is most obvious. `msb8` has the
//! same shape for `0xFF`-topped bytes, but there the smear stops before the
//! sign bit, so it only misfires on out-of-range input.
//!
//! `criticalBit32Mask` is the clearest instance of a conversion being undone:
//!
//! ```js
//! (~msb32(a ^ b) >>> 0) & 0xffffffff
//! ```
//!
//! The `>>> 0` produces the intended unsigned value, and then `& 0xffffffff`
//! converts *both* operands back to `i32` — `0xffffffff` is `-1` there — so the
//! mask is a no-op that re-signs the result. Its byte-wide sibling
//! `criticalBit8Mask` ends in `& 0xff` and is correct. See BUG-SPARSE-QUEUE-SET-3.

/// ToInt32: the conversion every JavaScript bitwise operator applies.
///
/// Truncate toward zero, reduce modulo 2^32, reinterpret as signed. `NaN` and
/// the infinities become `0`, which is why `1 << Infinity` is `1` in
/// JavaScript.
///
/// The `%` is exact: IEEE-754 remainder of two representable values is itself
/// representable, so this is correct for magnitudes beyond `i64` where a plain
/// `as` cast would saturate.
pub fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }

    let wrapped = value.trunc().rem_euclid(4_294_967_296.0);

    if wrapped >= 2_147_483_648.0 {
        (wrapped - 4_294_967_296.0) as i32
    } else {
        wrapped as i32
    }
}

/// ToUint32: as [`to_int32`], without the final reinterpretation.
///
/// Used for shift counts (`ToUint32(pos) & 0x1F`) and for the `>>> 0` in the
/// critical-bit masks.
pub fn to_uint32(value: f64) -> u32 {
    to_int32(value) as u32
}

/// `msb32(x)` — the most significant set bit of a 32-bit integer, by SWAR.
///
/// **Returns `0` whenever bit 31 of the input is set**, which is upstream's
/// defect rather than this port's. Verified against Node:
/// `msb32(0xFFFFFFFF) === 0` and `msb32(2**31) === 0`, while
/// `msb32(0x40000000) === 1073741824`.
pub fn msb32(x: f64) -> i32 {
    // `x |= (x >> 1)` converts to i32 on its first use and stays there.
    let mut x = to_int32(x);

    x |= x >> 1;
    x |= x >> 2;
    x |= x >> 4;
    x |= x >> 8;
    x |= x >> 16;

    x & !(x >> 1)
}

/// `msb8(x)` — the same scan, smeared over eight bits only.
///
/// Correct for `0..=255`. Above that the smear does not reach the high bits and
/// the result is meaningless: `msb8(256) === 256` on Node. Upstream documents
/// the parameter as "a byte" and does not check it.
pub fn msb8(x: f64) -> i32 {
    let mut x = to_int32(x);

    x |= x >> 1;
    x |= x >> 2;
    x |= x >> 4;

    x & !(x >> 1)
}

/// `test(x, pos)` — bit `pos` of `x`, as `0` or `1`.
///
/// The shift count wraps at 32, as JavaScript's `>>` does: `test(1, 32)` is
/// `test(1, 0)` and answers `1`. A negative `pos` becomes a huge `u32` and then
/// wraps too, so `test(5, -1)` is `test(5, 31)`, which is `0`.
pub fn test(x: f64, pos: f64) -> i32 {
    // ShiftExpression: ToUint32(rhs) & 0x1F.
    let shift = to_uint32(pos) & 0x1f;

    (to_int32(x) >> shift) & 1
}

/// `criticalBit8(a, b)` — the highest bit at which two bytes differ.
pub fn critical_bit8(a: f64, b: f64) -> i32 {
    msb8(f64::from(to_int32(a) ^ to_int32(b)))
}

/// `criticalBit8Mask(a, b)` — the complement of [`critical_bit8`], byte-wide.
///
/// Correct, unlike its 32-bit sibling, because `& 0xff` really does mask.
pub fn critical_bit8_mask(a: f64, b: f64) -> i32 {
    let unsigned = f64::from(!critical_bit8(a, b) as u32);

    to_int32(unsigned) & 0xff
}

/// `testCriticalBit8(x, mask)` — which side of a critical bit `x` falls on.
///
/// `1 + (x | mask)` is evaluated as a JavaScript *Number*, so it can reach
/// 2^31 and the following `>> 8` then converts it back to a negative `i32`.
/// Reproduced with an `f64` intermediate for exactly that reason.
pub fn test_critical_bit8(x: f64, mask: f64) -> i32 {
    let combined = to_int32(x) | to_int32(mask);

    to_int32(1.0 + f64::from(combined)) >> 8
}

/// `criticalBit32Mask(a, b)` — the 32-bit sibling of [`critical_bit8_mask`].
///
/// **Returns a negative `i32`**, because the trailing `& 0xffffffff` converts
/// its own operands to `i32` and so undoes the `>>> 0` that precedes it.
/// Verified against Node: `criticalBit32Mask(1, 2) === -3` and
/// `criticalBit32Mask(0, 0) === -1`. Reproduced bug-for-bug; see BUG-SPARSE-QUEUE-SET-3.
pub fn critical_bit32_mask(a: f64, b: f64) -> i32 {
    let unsigned = f64::from(!msb32(f64::from(to_int32(a) ^ to_int32(b))) as u32);

    // `& 0xffffffff` is `& -1`, i.e. identity on an i32.
    to_int32(unsigned) & to_int32(4_294_967_295.0)
}

/// `popcount(x)` — population count of the low 32 bits, by SWAR.
///
/// Correct for every input, which makes it the one function in the file with no
/// defect. The literal transcription matters more than it looks: upstream's
/// first statement is `x -= x >> 1 & 0x55555555`, where the subtraction happens
/// on the *Number* and only the right-hand side is converted, so an input at or
/// above 2^31 stays a float across the first step.
pub fn popcount(x: f64) -> i32 {
    let mut x = x;

    // x -= x >> 1 & 0x55555555;
    x -= f64::from(to_int32(x) >> 1 & 0x5555_5555);
    // x = (x & 0x33333333) + (x >> 2 & 0x33333333);
    x = f64::from(to_int32(x) & 0x3333_3333) + f64::from(to_int32(x) >> 2 & 0x3333_3333);
    // x = x + (x >> 4) & 0x0f0f0f0f;   -- `&` binds looser than `+`
    x = f64::from(to_int32(x + f64::from(to_int32(x) >> 4)) & 0x0f0f_0f0f);
    // x += x >> 8;
    x += f64::from(to_int32(x) >> 8);
    // x += x >> 16;
    x += f64::from(to_int32(x) >> 16);

    to_int32(x) & 0x7f
}

/// Upstream's `TABLE8`, one population count per byte value.
///
/// Upstream fills it by calling `popcount(i)` at module load. Building it from
/// [`u8::count_ones`] instead is only legitimate if the two agree everywhere,
/// which `table8_is_exactly_popcount_of_every_byte` checks for all 256 entries
/// rather than assuming.
const TABLE8: [u8; 256] = build_table8();

const fn build_table8() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut index = 0usize;

    while index < 256 {
        table[index] = (index as u8).count_ones() as u8;
        index += 1;
    }

    table
}

/// `table8Popcount(x)` — population count by four table lookups.
///
/// The hot one: `BitSet::rank` and `BitVector::rank` call it once per word, and
/// it is the only member of this module the upstream test suite reaches at all.
pub fn table8_popcount(x: f64) -> i32 {
    let x = to_int32(x);

    // Each `& 0xff` yields 0..=255, so every index is in range by construction.
    i32::from(TABLE8[(x & 0xff) as usize])
        + i32::from(TABLE8[((x >> 8) & 0xff) as usize])
        + i32::from(TABLE8[((x >> 16) & 0xff) as usize])
        + i32::from(TABLE8[((x >> 24) & 0xff) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every function in the module, checked against **real Node 24.18.1**
    /// rather than against this port's own reasoning about ToInt32.
    ///
    /// The table is machine-generated: 28 inputs crossed with 9 second
    /// arguments, run through the upstream file on Node, and pasted here as
    /// literals. That keeps `mnemonist-core` buildable and testable with no
    /// JavaScript runtime present (gate 2) while still making the claim
    /// "matches upstream" an executed comparison rather than an assertion.
    ///
    /// The inputs deliberately include the values where upstream misbehaves --
    /// `0x80000000`, `0xffffffff`, `-1`, `256` -- so any "tidying" of `msb32`
    /// or `criticalBit32Mask` fails here.
    #[test]
    fn matches_node_24_18_1_on_a_generated_cross_product() {
        // (x, y, msb32(x), msb8(x), popcount(x), table8Popcount(x),
        //  test(x, y), criticalBit8(x, y), criticalBit8Mask(x, y),
        //  testCriticalBit8(x, y), criticalBit32Mask(x, y))
        #[allow(clippy::type_complexity)]
        const NODE: &[(f64, f64, i32, i32, i32, i32, i32, i32, i32, i32, i32)] = &[
            (0.0, 0.0, 0, 0, 0, 0, 0, 0, 255, 0, -1),
            (0.0, 1.0, 0, 0, 0, 0, 0, 1, 254, 0, -2),
            (0.0, 2.0, 0, 0, 0, 0, 0, 2, 253, 0, -3),
            (0.0, 8.0, 0, 0, 0, 0, 0, 8, 247, 0, -9),
            (0.0, 31.0, 0, 0, 0, 0, 0, 16, 239, 0, -17),
            (0.0, 32.0, 0, 0, 0, 0, 0, 32, 223, 0, -33),
            (0.0, 255.0, 0, 0, 0, 0, 0, 128, 127, 1, -129),
            (0.0, 2147483648.0, 0, 0, 0, 0, 0, 0, 255, -8388608, -1),
            (0.0, -1.0, 0, 0, 0, 0, 0, 0, 255, 0, -1),
            (1.0, 0.0, 1, 1, 1, 1, 1, 1, 254, 0, -2),
            (1.0, 1.0, 1, 1, 1, 1, 0, 0, 255, 0, -1),
            (1.0, 2.0, 1, 1, 1, 1, 0, 2, 253, 0, -3),
            (1.0, 8.0, 1, 1, 1, 1, 0, 8, 247, 0, -9),
            (1.0, 31.0, 1, 1, 1, 1, 0, 16, 239, 0, -17),
            (1.0, 32.0, 1, 1, 1, 1, 1, 32, 223, 0, -33),
            (1.0, 255.0, 1, 1, 1, 1, 0, 128, 127, 1, -129),
            (1.0, 2147483648.0, 1, 1, 1, 1, 1, 1, 254, -8388608, -1),
            (1.0, -1.0, 1, 1, 1, 1, 0, 0, 255, 0, -1),
            (2.0, 0.0, 2, 2, 1, 1, 0, 2, 253, 0, -3),
            (2.0, 1.0, 2, 2, 1, 1, 1, 2, 253, 0, -3),
            (2.0, 2.0, 2, 2, 1, 1, 0, 0, 255, 0, -1),
            (2.0, 8.0, 2, 2, 1, 1, 0, 8, 247, 0, -9),
            (2.0, 31.0, 2, 2, 1, 1, 0, 16, 239, 0, -17),
            (2.0, 32.0, 2, 2, 1, 1, 0, 32, 223, 0, -33),
            (2.0, 255.0, 2, 2, 1, 1, 0, 128, 127, 1, -129),
            (2.0, 2147483648.0, 2, 2, 1, 1, 0, 2, 253, -8388608, -1),
            (2.0, -1.0, 2, 2, 1, 1, 0, 0, 255, 0, -1),
            (3.0, 0.0, 2, 2, 2, 2, 1, 2, 253, 0, -3),
            (3.0, 1.0, 2, 2, 2, 2, 1, 2, 253, 0, -3),
            (3.0, 2.0, 2, 2, 2, 2, 0, 1, 254, 0, -2),
            (3.0, 8.0, 2, 2, 2, 2, 0, 8, 247, 0, -9),
            (3.0, 31.0, 2, 2, 2, 2, 0, 16, 239, 0, -17),
            (3.0, 32.0, 2, 2, 2, 2, 1, 32, 223, 0, -33),
            (3.0, 255.0, 2, 2, 2, 2, 0, 128, 127, 1, -129),
            (3.0, 2147483648.0, 2, 2, 2, 2, 1, 2, 253, -8388608, -1),
            (3.0, -1.0, 2, 2, 2, 2, 0, 0, 255, 0, -1),
            (7.0, 0.0, 4, 4, 3, 3, 1, 4, 251, 0, -5),
            (7.0, 1.0, 4, 4, 3, 3, 1, 4, 251, 0, -5),
            (7.0, 2.0, 4, 4, 3, 3, 1, 4, 251, 0, -5),
            (7.0, 8.0, 4, 4, 3, 3, 0, 8, 247, 0, -9),
            (7.0, 31.0, 4, 4, 3, 3, 0, 16, 239, 0, -17),
            (7.0, 32.0, 4, 4, 3, 3, 1, 32, 223, 0, -33),
            (7.0, 255.0, 4, 4, 3, 3, 0, 128, 127, 1, -129),
            (7.0, 2147483648.0, 4, 4, 3, 3, 1, 4, 251, -8388608, -1),
            (7.0, -1.0, 4, 4, 3, 3, 0, 0, 255, 0, -1),
            (8.0, 0.0, 8, 8, 1, 1, 0, 8, 247, 0, -9),
            (8.0, 1.0, 8, 8, 1, 1, 0, 8, 247, 0, -9),
            (8.0, 2.0, 8, 8, 1, 1, 0, 8, 247, 0, -9),
            (8.0, 8.0, 8, 8, 1, 1, 0, 0, 255, 0, -1),
            (8.0, 31.0, 8, 8, 1, 1, 0, 16, 239, 0, -17),
            (8.0, 32.0, 8, 8, 1, 1, 0, 32, 223, 0, -33),
            (8.0, 255.0, 8, 8, 1, 1, 0, 128, 127, 1, -129),
            (8.0, 2147483648.0, 8, 8, 1, 1, 0, 8, 247, -8388608, -1),
            (8.0, -1.0, 8, 8, 1, 1, 0, 0, 255, 0, -1),
            (127.0, 0.0, 64, 64, 7, 7, 1, 64, 191, 0, -65),
            (127.0, 1.0, 64, 64, 7, 7, 1, 64, 191, 0, -65),
            (127.0, 2.0, 64, 64, 7, 7, 1, 64, 191, 0, -65),
            (127.0, 8.0, 64, 64, 7, 7, 0, 64, 191, 0, -65),
            (127.0, 31.0, 64, 64, 7, 7, 0, 64, 191, 0, -65),
            (127.0, 32.0, 64, 64, 7, 7, 1, 64, 191, 0, -65),
            (127.0, 255.0, 64, 64, 7, 7, 0, 128, 127, 1, -129),
            (127.0, 2147483648.0, 64, 64, 7, 7, 1, 64, 191, -8388608, -1),
            (127.0, -1.0, 64, 64, 7, 7, 0, 0, 255, 0, -1),
            (128.0, 0.0, 128, 128, 1, 1, 0, 128, 127, 0, -129),
            (128.0, 1.0, 128, 128, 1, 1, 0, 128, 127, 0, -129),
            (128.0, 2.0, 128, 128, 1, 1, 0, 128, 127, 0, -129),
            (128.0, 8.0, 128, 128, 1, 1, 0, 128, 127, 0, -129),
            (128.0, 31.0, 128, 128, 1, 1, 0, 128, 127, 0, -129),
            (128.0, 32.0, 128, 128, 1, 1, 0, 128, 127, 0, -129),
            (128.0, 255.0, 128, 128, 1, 1, 0, 64, 191, 1, -65),
            (
                128.0,
                2147483648.0,
                128,
                128,
                1,
                1,
                0,
                128,
                127,
                -8388608,
                -1,
            ),
            (128.0, -1.0, 128, 128, 1, 1, 0, 0, 255, 0, -1),
            (255.0, 0.0, 128, 128, 8, 8, 1, 128, 127, 1, -129),
            (255.0, 1.0, 128, 128, 8, 8, 1, 128, 127, 1, -129),
            (255.0, 2.0, 128, 128, 8, 8, 1, 128, 127, 1, -129),
            (255.0, 8.0, 128, 128, 8, 8, 0, 128, 127, 1, -129),
            (255.0, 31.0, 128, 128, 8, 8, 0, 128, 127, 1, -129),
            (255.0, 32.0, 128, 128, 8, 8, 1, 128, 127, 1, -129),
            (255.0, 255.0, 128, 128, 8, 8, 0, 0, 255, 1, -1),
            (
                255.0,
                2147483648.0,
                128,
                128,
                8,
                8,
                1,
                128,
                127,
                -8388607,
                -1,
            ),
            (255.0, -1.0, 128, 128, 8, 8, 0, 0, 255, 0, -1),
            (256.0, 0.0, 256, 256, 1, 1, 0, 256, 255, 1, -257),
            (256.0, 1.0, 256, 256, 1, 1, 0, 256, 255, 1, -257),
            (256.0, 2.0, 256, 256, 1, 1, 0, 256, 255, 1, -257),
            (256.0, 8.0, 256, 256, 1, 1, 1, 256, 255, 1, -257),
            (256.0, 31.0, 256, 256, 1, 1, 0, 256, 255, 1, -257),
            (256.0, 32.0, 256, 256, 1, 1, 0, 256, 255, 1, -257),
            (256.0, 255.0, 256, 256, 1, 1, 0, 256, 255, 2, -257),
            (
                256.0,
                2147483648.0,
                256,
                256,
                1,
                1,
                0,
                256,
                255,
                -8388607,
                -1,
            ),
            (256.0, -1.0, 256, 256, 1, 1, 0, 0, 255, 0, -1),
            (257.0, 0.0, 256, 256, 2, 2, 1, 256, 255, 1, -257),
            (257.0, 1.0, 256, 256, 2, 2, 0, 256, 255, 1, -257),
            (257.0, 2.0, 256, 256, 2, 2, 0, 256, 255, 1, -257),
            (257.0, 8.0, 256, 256, 2, 2, 1, 256, 255, 1, -257),
            (257.0, 31.0, 256, 256, 2, 2, 0, 256, 255, 1, -257),
            (257.0, 32.0, 256, 256, 2, 2, 1, 256, 255, 1, -257),
            (257.0, 255.0, 256, 256, 2, 2, 0, 256, 255, 2, -257),
            (
                257.0,
                2147483648.0,
                256,
                256,
                2,
                2,
                1,
                256,
                255,
                -8388607,
                -1,
            ),
            (257.0, -1.0, 256, 256, 2, 2, 0, 0, 255, 0, -1),
            (1023.0, 0.0, 512, 512, 10, 10, 1, 512, 255, 4, -513),
            (1023.0, 1.0, 512, 512, 10, 10, 1, 512, 255, 4, -513),
            (1023.0, 2.0, 512, 512, 10, 10, 1, 512, 255, 4, -513),
            (1023.0, 8.0, 512, 512, 10, 10, 1, 512, 255, 4, -513),
            (1023.0, 31.0, 512, 512, 10, 10, 0, 512, 255, 4, -513),
            (1023.0, 32.0, 512, 512, 10, 10, 1, 512, 255, 4, -513),
            (1023.0, 255.0, 512, 512, 10, 10, 0, 512, 255, 4, -513),
            (
                1023.0,
                2147483648.0,
                512,
                512,
                10,
                10,
                1,
                512,
                255,
                -8388604,
                -1,
            ),
            (1023.0, -1.0, 512, 512, 10, 10, 0, 0, 255, 0, -1),
            (
                65535.0, 0.0, 32768, 32768, 16, 16, 1, 32768, 255, 256, -32769,
            ),
            (
                65535.0, 1.0, 32768, 32768, 16, 16, 1, 32768, 255, 256, -32769,
            ),
            (
                65535.0, 2.0, 32768, 32768, 16, 16, 1, 32768, 255, 256, -32769,
            ),
            (
                65535.0, 8.0, 32768, 32768, 16, 16, 1, 32768, 255, 256, -32769,
            ),
            (
                65535.0, 31.0, 32768, 32768, 16, 16, 0, 32768, 255, 256, -32769,
            ),
            (
                65535.0, 32.0, 32768, 32768, 16, 16, 1, 32768, 255, 256, -32769,
            ),
            (
                65535.0, 255.0, 32768, 32768, 16, 16, 0, 32768, 255, 256, -32769,
            ),
            (
                65535.0,
                2147483648.0,
                32768,
                32768,
                16,
                16,
                1,
                32768,
                255,
                -8388352,
                -1,
            ),
            (65535.0, -1.0, 32768, 32768, 16, 16, 0, 0, 255, 0, -1),
            (65536.0, 0.0, 65536, 65536, 1, 1, 0, 65536, 255, 256, -65537),
            (65536.0, 1.0, 65536, 65536, 1, 1, 0, 65537, 254, 256, -65537),
            (65536.0, 2.0, 65536, 65536, 1, 1, 0, 65538, 253, 256, -65537),
            (65536.0, 8.0, 65536, 65536, 1, 1, 0, 65544, 247, 256, -65537),
            (
                65536.0, 31.0, 65536, 65536, 1, 1, 0, 65552, 239, 256, -65537,
            ),
            (
                65536.0, 32.0, 65536, 65536, 1, 1, 0, 65568, 223, 256, -65537,
            ),
            (
                65536.0, 255.0, 65536, 65536, 1, 1, 0, 65664, 127, 257, -65537,
            ),
            (
                65536.0,
                2147483648.0,
                65536,
                65536,
                1,
                1,
                0,
                65536,
                255,
                -8388352,
                -1,
            ),
            (65536.0, -1.0, 65536, 65536, 1, 1, 0, 0, 255, 0, -1),
            (
                305419896.0,
                0.0,
                268435456,
                268435456,
                13,
                13,
                0,
                268435456,
                255,
                1193046,
                -268435457,
            ),
            (
                305419896.0,
                1.0,
                268435456,
                268435456,
                13,
                13,
                0,
                268435456,
                255,
                1193046,
                -268435457,
            ),
            (
                305419896.0,
                2.0,
                268435456,
                268435456,
                13,
                13,
                0,
                268435456,
                255,
                1193046,
                -268435457,
            ),
            (
                305419896.0,
                8.0,
                268435456,
                268435456,
                13,
                13,
                0,
                268435456,
                255,
                1193046,
                -268435457,
            ),
            (
                305419896.0,
                31.0,
                268435456,
                268435456,
                13,
                13,
                0,
                268435456,
                255,
                1193046,
                -268435457,
            ),
            (
                305419896.0,
                32.0,
                268435456,
                268435456,
                13,
                13,
                0,
                268435456,
                255,
                1193046,
                -268435457,
            ),
            (
                305419896.0,
                255.0,
                268435456,
                268435456,
                13,
                13,
                0,
                268435456,
                255,
                1193047,
                -268435457,
            ),
            (
                305419896.0,
                2147483648.0,
                268435456,
                268435456,
                13,
                13,
                0,
                0,
                255,
                -7195562,
                -1,
            ),
            (
                305419896.0,
                -1.0,
                268435456,
                268435456,
                13,
                13,
                0,
                0,
                255,
                0,
                -1,
            ),
            (
                1073741824.0,
                0.0,
                1073741824,
                1073741824,
                1,
                1,
                0,
                1073741824,
                255,
                4194304,
                -1073741825,
            ),
            (
                1073741824.0,
                1.0,
                1073741824,
                1073741824,
                1,
                1,
                0,
                1073741825,
                254,
                4194304,
                -1073741825,
            ),
            (
                1073741824.0,
                2.0,
                1073741824,
                1073741824,
                1,
                1,
                0,
                1073741826,
                253,
                4194304,
                -1073741825,
            ),
            (
                1073741824.0,
                8.0,
                1073741824,
                1073741824,
                1,
                1,
                0,
                1073741832,
                247,
                4194304,
                -1073741825,
            ),
            (
                1073741824.0,
                31.0,
                1073741824,
                1073741824,
                1,
                1,
                0,
                1073741840,
                239,
                4194304,
                -1073741825,
            ),
            (
                1073741824.0,
                32.0,
                1073741824,
                1073741824,
                1,
                1,
                0,
                1073741856,
                223,
                4194304,
                -1073741825,
            ),
            (
                1073741824.0,
                255.0,
                1073741824,
                1073741824,
                1,
                1,
                0,
                1073741952,
                127,
                4194305,
                -1073741825,
            ),
            (
                1073741824.0,
                2147483648.0,
                1073741824,
                1073741824,
                1,
                1,
                0,
                0,
                255,
                -4194304,
                -1,
            ),
            (
                1073741824.0,
                -1.0,
                1073741824,
                1073741824,
                1,
                1,
                0,
                0,
                255,
                0,
                -1,
            ),
            (
                2147483647.0,
                0.0,
                1073741824,
                1073741824,
                31,
                31,
                1,
                1073741824,
                255,
                -8388608,
                -1073741825,
            ),
            (
                2147483647.0,
                1.0,
                1073741824,
                1073741824,
                31,
                31,
                1,
                1073741824,
                255,
                -8388608,
                -1073741825,
            ),
            (
                2147483647.0,
                2.0,
                1073741824,
                1073741824,
                31,
                31,
                1,
                1073741824,
                255,
                -8388608,
                -1073741825,
            ),
            (
                2147483647.0,
                8.0,
                1073741824,
                1073741824,
                31,
                31,
                1,
                1073741824,
                255,
                -8388608,
                -1073741825,
            ),
            (
                2147483647.0,
                31.0,
                1073741824,
                1073741824,
                31,
                31,
                0,
                1073741824,
                255,
                -8388608,
                -1073741825,
            ),
            (
                2147483647.0,
                32.0,
                1073741824,
                1073741824,
                31,
                31,
                1,
                1073741824,
                255,
                -8388608,
                -1073741825,
            ),
            (
                2147483647.0,
                255.0,
                1073741824,
                1073741824,
                31,
                31,
                0,
                1073741824,
                255,
                -8388608,
                -1073741825,
            ),
            (
                2147483647.0,
                2147483648.0,
                1073741824,
                1073741824,
                31,
                31,
                1,
                0,
                255,
                0,
                -1,
            ),
            (
                2147483647.0,
                -1.0,
                1073741824,
                1073741824,
                31,
                31,
                0,
                0,
                255,
                0,
                -1,
            ),
            (2147483648.0, 0.0, 0, 0, 1, 1, 0, 0, 255, -8388608, -1),
            (2147483648.0, 1.0, 0, 0, 1, 1, 0, 1, 254, -8388608, -1),
            (2147483648.0, 2.0, 0, 0, 1, 1, 0, 2, 253, -8388608, -1),
            (2147483648.0, 8.0, 0, 0, 1, 1, 0, 8, 247, -8388608, -1),
            (2147483648.0, 31.0, 0, 0, 1, 1, 1, 16, 239, -8388608, -1),
            (2147483648.0, 32.0, 0, 0, 1, 1, 0, 32, 223, -8388608, -1),
            (2147483648.0, 255.0, 0, 0, 1, 1, 1, 128, 127, -8388607, -1),
            (
                2147483648.0,
                2147483648.0,
                0,
                0,
                1,
                1,
                0,
                0,
                255,
                -8388608,
                -1,
            ),
            (
                2147483648.0,
                -1.0,
                0,
                0,
                1,
                1,
                1,
                1073741824,
                255,
                0,
                -1073741825,
            ),
            (3735928559.0, 0.0, 0, 0, 24, 24, 1, 0, 255, -2183746, -1),
            (3735928559.0, 1.0, 0, 0, 24, 24, 1, 0, 255, -2183746, -1),
            (3735928559.0, 2.0, 0, 0, 24, 24, 1, 0, 255, -2183746, -1),
            (3735928559.0, 8.0, 0, 0, 24, 24, 0, 0, 255, -2183746, -1),
            (3735928559.0, 31.0, 0, 0, 24, 24, 1, 0, 255, -2183745, -1),
            (3735928559.0, 32.0, 0, 0, 24, 24, 1, 0, 255, -2183746, -1),
            (3735928559.0, 255.0, 0, 0, 24, 24, 1, 0, 255, -2183745, -1),
            (
                3735928559.0,
                2147483648.0,
                0,
                0,
                24,
                24,
                1,
                1073741824,
                255,
                -2183746,
                -1073741825,
            ),
            (
                3735928559.0,
                -1.0,
                0,
                0,
                24,
                24,
                1,
                536870912,
                255,
                0,
                -536870913,
            ),
            (4294967295.0, 0.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (4294967295.0, 1.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (4294967295.0, 2.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (4294967295.0, 8.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (4294967295.0, 31.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (4294967295.0, 32.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (4294967295.0, 255.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (
                4294967295.0,
                2147483648.0,
                0,
                0,
                32,
                32,
                1,
                1073741824,
                255,
                0,
                -1073741825,
            ),
            (4294967295.0, -1.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (-1.0, 0.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (-1.0, 1.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (-1.0, 2.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (-1.0, 8.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (-1.0, 31.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (-1.0, 32.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (-1.0, 255.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (
                -1.0,
                2147483648.0,
                0,
                0,
                32,
                32,
                1,
                1073741824,
                255,
                0,
                -1073741825,
            ),
            (-1.0, -1.0, 0, 0, 32, 32, 1, 0, 255, 0, -1),
            (-2.0, 0.0, 0, 0, 31, 31, 0, 0, 255, -1, -1),
            (-2.0, 1.0, 0, 0, 31, 31, 1, 0, 255, 0, -1),
            (-2.0, 2.0, 0, 0, 31, 31, 1, 0, 255, -1, -1),
            (-2.0, 8.0, 0, 0, 31, 31, 1, 0, 255, -1, -1),
            (-2.0, 31.0, 0, 0, 31, 31, 1, 0, 255, 0, -1),
            (-2.0, 32.0, 0, 0, 31, 31, 0, 0, 255, -1, -1),
            (-2.0, 255.0, 0, 0, 31, 31, 1, 0, 255, 0, -1),
            (
                -2.0,
                2147483648.0,
                0,
                0,
                31,
                31,
                0,
                1073741824,
                255,
                -1,
                -1073741825,
            ),
            (-2.0, -1.0, 0, 0, 31, 31, 1, 1, 254, 0, -2),
            (-2147483648.0, 0.0, 0, 0, 1, 1, 0, 0, 255, -8388608, -1),
            (-2147483648.0, 1.0, 0, 0, 1, 1, 0, 1, 254, -8388608, -1),
            (-2147483648.0, 2.0, 0, 0, 1, 1, 0, 2, 253, -8388608, -1),
            (-2147483648.0, 8.0, 0, 0, 1, 1, 0, 8, 247, -8388608, -1),
            (-2147483648.0, 31.0, 0, 0, 1, 1, 1, 16, 239, -8388608, -1),
            (-2147483648.0, 32.0, 0, 0, 1, 1, 0, 32, 223, -8388608, -1),
            (-2147483648.0, 255.0, 0, 0, 1, 1, 1, 128, 127, -8388607, -1),
            (
                -2147483648.0,
                2147483648.0,
                0,
                0,
                1,
                1,
                0,
                0,
                255,
                -8388608,
                -1,
            ),
            (
                -2147483648.0,
                -1.0,
                0,
                0,
                1,
                1,
                1,
                1073741824,
                255,
                0,
                -1073741825,
            ),
            (4294967296.0, 0.0, 0, 0, 0, 0, 0, 0, 255, 0, -1),
            (4294967296.0, 1.0, 0, 0, 0, 0, 0, 1, 254, 0, -2),
            (4294967296.0, 2.0, 0, 0, 0, 0, 0, 2, 253, 0, -3),
            (4294967296.0, 8.0, 0, 0, 0, 0, 0, 8, 247, 0, -9),
            (4294967296.0, 31.0, 0, 0, 0, 0, 0, 16, 239, 0, -17),
            (4294967296.0, 32.0, 0, 0, 0, 0, 0, 32, 223, 0, -33),
            (4294967296.0, 255.0, 0, 0, 0, 0, 0, 128, 127, 1, -129),
            (
                4294967296.0,
                2147483648.0,
                0,
                0,
                0,
                0,
                0,
                0,
                255,
                -8388608,
                -1,
            ),
            (4294967296.0, -1.0, 0, 0, 0, 0, 0, 0, 255, 0, -1),
            (4294967297.0, 0.0, 1, 1, 1, 1, 1, 1, 254, 0, -2),
            (4294967297.0, 1.0, 1, 1, 1, 1, 0, 0, 255, 0, -1),
            (4294967297.0, 2.0, 1, 1, 1, 1, 0, 2, 253, 0, -3),
            (4294967297.0, 8.0, 1, 1, 1, 1, 0, 8, 247, 0, -9),
            (4294967297.0, 31.0, 1, 1, 1, 1, 0, 16, 239, 0, -17),
            (4294967297.0, 32.0, 1, 1, 1, 1, 1, 32, 223, 0, -33),
            (4294967297.0, 255.0, 1, 1, 1, 1, 0, 128, 127, 1, -129),
            (
                4294967297.0,
                2147483648.0,
                1,
                1,
                1,
                1,
                1,
                1,
                254,
                -8388608,
                -1,
            ),
            (4294967297.0, -1.0, 1, 1, 1, 1, 0, 0, 255, 0, -1),
            (0.5, 0.0, 0, 0, 0, 0, 0, 0, 255, 0, -1),
            (0.5, 1.0, 0, 0, 0, 0, 0, 1, 254, 0, -2),
            (0.5, 2.0, 0, 0, 0, 0, 0, 2, 253, 0, -3),
            (0.5, 8.0, 0, 0, 0, 0, 0, 8, 247, 0, -9),
            (0.5, 31.0, 0, 0, 0, 0, 0, 16, 239, 0, -17),
            (0.5, 32.0, 0, 0, 0, 0, 0, 32, 223, 0, -33),
            (0.5, 255.0, 0, 0, 0, 0, 0, 128, 127, 1, -129),
            (0.5, 2147483648.0, 0, 0, 0, 0, 0, 0, 255, -8388608, -1),
            (0.5, -1.0, 0, 0, 0, 0, 0, 0, 255, 0, -1),
            (3.9, 0.0, 2, 2, 2, 2, 1, 2, 253, 0, -3),
            (3.9, 1.0, 2, 2, 2, 2, 1, 2, 253, 0, -3),
            (3.9, 2.0, 2, 2, 2, 2, 0, 1, 254, 0, -2),
            (3.9, 8.0, 2, 2, 2, 2, 0, 8, 247, 0, -9),
            (3.9, 31.0, 2, 2, 2, 2, 0, 16, 239, 0, -17),
            (3.9, 32.0, 2, 2, 2, 2, 1, 32, 223, 0, -33),
            (3.9, 255.0, 2, 2, 2, 2, 0, 128, 127, 1, -129),
            (3.9, 2147483648.0, 2, 2, 2, 2, 1, 2, 253, -8388608, -1),
            (3.9, -1.0, 2, 2, 2, 2, 0, 0, 255, 0, -1),
            (-3.9, 0.0, 0, 0, 31, 31, 1, 0, 255, -1, -1),
            (-3.9, 1.0, 0, 0, 31, 31, 0, 0, 255, -1, -1),
            (-3.9, 2.0, 0, 0, 31, 31, 1, 0, 255, 0, -1),
            (-3.9, 8.0, 0, 0, 31, 31, 1, 0, 255, -1, -1),
            (-3.9, 31.0, 0, 0, 31, 31, 1, 0, 255, 0, -1),
            (-3.9, 32.0, 0, 0, 31, 31, 1, 0, 255, -1, -1),
            (-3.9, 255.0, 0, 0, 31, 31, 1, 0, 255, 0, -1),
            (
                -3.9,
                2147483648.0,
                0,
                0,
                31,
                31,
                1,
                1073741824,
                255,
                -1,
                -1073741825,
            ),
            (-3.9, -1.0, 0, 0, 31, 31, 1, 2, 253, 0, -3),
        ];

        for &(x, y, m32, m8, pc, t8, bit, cb8, cb8m, tcb8, cb32m) in NODE {
            assert_eq!(msb32(x), m32, "msb32({x})");
            assert_eq!(msb8(x), m8, "msb8({x})");
            assert_eq!(popcount(x), pc, "popcount({x})");
            assert_eq!(table8_popcount(x), t8, "table8Popcount({x})");
            assert_eq!(test(x, y), bit, "test({x}, {y})");
            assert_eq!(critical_bit8(x, y), cb8, "criticalBit8({x}, {y})");
            assert_eq!(critical_bit8_mask(x, y), cb8m, "criticalBit8Mask({x}, {y})");
            assert_eq!(test_critical_bit8(x, y), tcb8, "testCriticalBit8({x}, {y})");
            assert_eq!(
                critical_bit32_mask(x, y),
                cb32m,
                "criticalBit32Mask({x}, {y})"
            );
        }
    }

    /// A spread of 32-bit patterns wide enough to be worth calling coverage,
    /// without the 4 billion an exhaustive sweep would need.
    fn sample_words() -> Vec<u32> {
        let mut words = vec![0u32, 1, 2, 3, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff];

        // Every single bit, and every adjacent pair.
        for bit in 0..32u32 {
            words.push(1 << bit);
            words.push(!(1 << bit));
            words.push((1u32 << bit) | (1u32 << ((bit + 1) % 32)));
        }

        // Every 16-bit value, in both halves.
        for low in 0..=u16::MAX {
            words.push(u32::from(low));
            words.push(u32::from(low) << 16);
        }

        // A deterministic xorshift32 spread over the full range.
        let mut state = 0x1234_5678u32;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            words.push(state);
        }

        words
    }

    /// The only property the whole module rests on: both population counts are
    /// exactly the number of set bits, over a wide sample of the 32-bit range.
    ///
    /// Upstream has no test file at all, so this is the first time either
    /// function has been checked against a reference.
    #[test]
    fn both_popcounts_agree_with_the_true_bit_count() {
        for word in sample_words() {
            let expected = word.count_ones() as i32;

            assert_eq!(popcount(f64::from(word)), expected, "popcount({word:#x})");
            assert_eq!(
                table8_popcount(f64::from(word)),
                expected,
                "table8_popcount({word:#x})"
            );
        }
    }

    /// The substitution made in [`TABLE8`]'s construction, checked rather than
    /// assumed: upstream fills the table with its own `popcount`.
    #[test]
    fn table8_is_exactly_popcount_of_every_byte() {
        for byte in 0..256u32 {
            assert_eq!(i32::from(TABLE8[byte as usize]), popcount(f64::from(byte)));
        }
    }

    /// Negative inputs are ToInt32'd, so they are the two's-complement pattern.
    /// Verified against Node: `popcount(-1) === 32`.
    #[test]
    fn popcount_of_a_negative_is_the_count_of_its_two_s_complement() {
        assert_eq!(popcount(-1.0), 32);
        assert_eq!(table8_popcount(-1.0), 32);
        assert_eq!(popcount(-2.0), 31);
        assert_eq!(popcount(f64::from(i32::MIN)), 1);
    }

    /// Non-integers truncate before the count. Verified: `popcount(0.5) === 0`.
    #[test]
    fn non_integers_truncate_toward_zero_first() {
        assert_eq!(popcount(0.5), 0);
        assert_eq!(popcount(3.9), 2);
        assert_eq!(popcount(-3.9), popcount(-3.0));
        assert_eq!(table8_popcount(3.9), 2);
    }

    /// Values past 2^32 wrap rather than saturating, which a plain `as` cast in
    /// Rust would get wrong in the other direction.
    #[test]
    fn values_past_the_32_bit_range_wrap() {
        assert_eq!(to_int32(4_294_967_296.0), 0);
        assert_eq!(to_int32(4_294_967_297.0), 1);
        assert_eq!(to_int32(-1.0), -1);
        assert_eq!(to_int32(2_147_483_648.0), i32::MIN);
        assert_eq!(to_uint32(-1.0), u32::MAX);
        // 2^53, the largest exactly representable odd-adjacent integer, and
        // beyond the range any `i64` cast would handle by saturating.
        assert_eq!(to_int32(9_007_199_254_740_992.0), 0);
        assert_eq!(to_int32(1e30), 0);

        assert_eq!(popcount(4_294_967_296.0), 0);
        assert_eq!(popcount(4_294_967_297.0), 1);
    }

    /// `NaN` and the infinities are `0`, which is why `1 << Infinity === 1`.
    #[test]
    fn non_finite_inputs_become_zero() {
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(f64::INFINITY), 0);
        assert_eq!(to_int32(f64::NEG_INFINITY), 0);
        assert_eq!(popcount(f64::NAN), 0);
        assert_eq!(test(1.0, f64::INFINITY), 1);
    }

    /// The headline defect: `msb32` answers `0` for the entire top half of the
    /// range, because `>>` is arithmetic and smears the sign bit.
    ///
    /// Verified against Node: `msb32(0xFFFFFFFF) === 0`, `msb32(2**31) === 0`,
    /// `msb32(-1) === 0`.
    #[test]
    fn msb32_returns_zero_for_every_input_with_the_top_bit_set() {
        assert_eq!(msb32(4_294_967_295.0), 0);
        assert_eq!(msb32(2_147_483_648.0), 0);
        assert_eq!(msb32(-1.0), 0);

        for bit in 0..32u32 {
            let value = f64::from(1u32 << bit);
            let expected = if bit == 31 { 0 } else { (1i32) << bit };

            assert_eq!(msb32(value), expected, "msb32(1 << {bit})");
        }
    }

    /// And it is correct everywhere else, which is what makes the defect easy
    /// to miss.
    #[test]
    fn msb32_is_correct_below_the_sign_bit() {
        assert_eq!(msb32(0.0), 0);
        assert_eq!(msb32(1.0), 1);
        assert_eq!(msb32(0x40000000_u32 as f64), 0x4000_0000);
        assert_eq!(msb32(0x12345678_u32 as f64), 0x1000_0000);

        for word in sample_words() {
            if word & 0x8000_0000 != 0 {
                continue;
            }

            let expected = if word == 0 {
                0
            } else {
                1i32 << (31 - word.leading_zeros())
            };

            assert_eq!(msb32(f64::from(word)), expected, "msb32({word:#x})");
        }
    }

    /// `msb8` is exact on a byte and meaningless off it. Verified:
    /// `msb8(255) === 128`, `msb8(256) === 256`, `msb8(-1) === 0`.
    #[test]
    fn msb8_is_correct_on_bytes_and_unmasked_above_them() {
        for byte in 0..256u32 {
            let expected = if byte == 0 {
                0
            } else {
                1i32 << (31 - byte.leading_zeros())
            };

            assert_eq!(msb8(f64::from(byte)), expected, "msb8({byte})");
        }

        // The parameter is documented as a byte and never checked.
        assert_eq!(msb8(256.0), 256);
        assert_eq!(msb8(-1.0), 0);
    }

    #[test]
    fn test_reads_one_bit_and_wraps_the_shift_count() {
        assert_eq!(test(5.0, 0.0), 1);
        assert_eq!(test(5.0, 1.0), 0);
        assert_eq!(test(5.0, 2.0), 1);
        assert_eq!(test(5.0, 3.0), 0);

        // JavaScript shift counts are taken mod 32.
        assert_eq!(test(1.0, 32.0), 1);
        assert_eq!(test(1.0, 64.0), 1);
        // ToUint32(-1) & 31 == 31.
        assert_eq!(test(5.0, -1.0), 0);
        assert_eq!(test(-1.0, 31.0), 1);

        for bit in 0..32u32 {
            assert_eq!(test(f64::from(1u32 << bit), f64::from(bit)), 1);
        }
    }

    /// The byte-wide critical-bit trio, which `fixed-critbit-tree-map` relies
    /// on and which no upstream test touches.
    #[test]
    fn the_byte_wide_critical_bit_helpers_agree_with_each_other() {
        assert_eq!(critical_bit8(10.0, 2.0), 8);
        assert_eq!(critical_bit8_mask(10.0, 2.0), 247);
        assert_eq!(
            test_critical_bit8(10.0, f64::from(critical_bit8_mask(10.0, 2.0))),
            1
        );

        // Identical bytes have no critical bit, and the mask is then 0xff.
        assert_eq!(critical_bit8(0.0, 0.0), 0);
        assert_eq!(critical_bit8_mask(0.0, 0.0), 255);

        // The mask is the complement of the critical bit, byte-wide, for every
        // pair of bytes -- which is the property `& 0xff` is there to give and
        // which `criticalBit32Mask` fails to give.
        for a in 0..64u32 {
            for b in 0..64u32 {
                let bit = critical_bit8(f64::from(a), f64::from(b));

                assert_eq!(
                    critical_bit8_mask(f64::from(a), f64::from(b)),
                    !bit & 0xff,
                    "criticalBit8Mask({a}, {b})"
                );
            }
        }
    }

    /// `1 + (x | mask)` is Number arithmetic, so it can reach 2^31 and the
    /// following `>> 8` reinterprets it as negative.
    #[test]
    fn test_critical_bit8_carries_through_number_arithmetic() {
        assert_eq!(test_critical_bit8(255.0, 0.0), 1);
        assert_eq!(test_critical_bit8(256.0, 0.0), 1);
        assert_eq!(test_critical_bit8(0.0, 0.0), 0);
        // x | mask == 0x7fffffff, so 1 + it is 2^31 as a Number and ToInt32
        // takes it to i32::MIN before the shift.
        assert_eq!(test_critical_bit8(2_147_483_647.0, 0.0), i32::MIN >> 8);
    }

    /// The `& 0xffffffff` no-op: it re-signs the value the `>>> 0` had just
    /// made unsigned. Verified against Node, which gives `-3` and `-1`.
    #[test]
    fn critical_bit32_mask_is_negative_because_the_trailing_mask_re_signs_it() {
        assert_eq!(critical_bit32_mask(1.0, 2.0), -3);
        assert_eq!(critical_bit32_mask(0.0, 0.0), -1);
        assert_eq!(critical_bit32_mask(1.0, 2_147_483_648.0), -1);

        // The intent was the unsigned complement; the `>>> 0` alone gives it.
        assert_eq!(!msb32(3.0) as u32, 4_294_967_293);
        // What upstream actually returns is the same bits, read as signed.
        assert_eq!(critical_bit32_mask(1.0, 2.0) as u32, 4_294_967_293);

        // And where msb32 returns 0 -- the whole top half of the range -- the
        // "mask" degenerates to -1, i.e. all bits, for every such pair.
        assert_eq!(critical_bit32_mask(0.0, 2_147_483_648.0), -1);
    }
}

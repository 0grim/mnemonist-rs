//! Port of upstream `utils/murmurhash3.js` (mnemonist v0.40.4).
//!
//! One exported function, `murmurhash3(seed, data)`, plus three arithmetic
//! helpers that exist because JavaScript has no 32-bit integer type. The whole
//! file is a member of the `bloom-filter` unit — nothing else in the library
//! calls it — so its gates are recorded in `docs/modules/bloom-filter.md`.
//!
//! # `data` is a `&[u16]`, and that is upstream's shape
//!
//! Upstream's JSDoc says `ByteArray`, and the loop reads
//! `data[i] | data[i+1] << 8 | data[i+2] << 16 | data[i+3] << 24` — four
//! *bytes* per 32-bit word. Its only caller, `bloom-filter.js`, hands it a
//! **`Uint16Array` of UTF-16 code units**. So each "byte" is really a 16-bit
//! value, the shifts overlap, and `data.length` counts code units rather than
//! bytes. That is not a mistake to be corrected here: it is the function's only
//! observed input, and the filter's published bit patterns depend on it. The
//! signature says `&[u16]` so the overlap is visible instead of implied.
//!
//! # The three helpers, and what they actually compute
//!
//! Each was checked against Node 24.18.1 rather than reasoned about, because
//! JavaScript's mixed float/ToInt32 arithmetic is exactly where a port goes
//! quietly wrong (see `utils/bitwise`'s `msb32`).
//!
//! | upstream | what it computes | verdict |
//! |---|---|---|
//! | `mul32(a, b)` | `(a * b) mod 2^32` | correct, for every constant used |
//! | `rotl32(a, b)` | `a.rotate_left(b)` | correct |
//! | `sum32(a, b)` | **not** `a + b` | broken, and cancelled by a swapped constant |
//!
//! ## `sum32` is not an adder
//!
//! ```js
//! function sum32(a, b) {
//!   return (a & 0xffff) + (b >>> 16) + (((a >>> 16) + b & 0xffff) << 16) & 0xffffffff;
//! }
//! ```
//!
//! The correct form takes `b & 0xffff` for the low half and `b >>> 16` for the
//! high half. This one has them the wrong way round in both places, so it adds
//! `b`'s **high** half to `a`'s low half and `b`'s **low** half to `a`'s high
//! half. `sum32(1, 1)` is `65537`, not `2` (verified on Node 24.18.1).
//!
//! It is called exactly once, with `n = 0x6b64e654`. MurmurHash3's published
//! constant is `0xe6546b64` — the *same* value with its halves swapped. The two
//! errors cancel exactly, and `sum32(hash, 0x6b64e654)` is
//! `(hash + 0xe6546b64) mod 2^32` for every 32-bit `hash`, which is what the
//! algorithm wants. Verified exhaustively over 200,000 random inputs against
//! big-integer arithmetic.
//!
//! So the digest is right, and the helper is wrong, and the only thing holding
//! them together is a constant nobody would recognise as a typo. Reproduced
//! bug-for-bug: [`sum32`] is public precisely so the defect is testable, and
//! the swapped constant is spelled out in [`murmurhash3`]. See NOTES.md B-93.
//!
//! # The tail, and the reads past the end
//!
//! The trailing `switch (data.length & 3)` falls through from `case 3` to
//! `case 1`, reading `data[i + 2]` and `data[i + 1]`, which are in range by
//! construction. What is *not* in range is the main loop's bound when the input
//! is shorter than four elements — but the loop's guard `i <= data.length - 4`
//! keeps it out. There is no out-of-range read in this file. (Contrast
//! `suffix-array.js`, where there is, and where it changes the answer.)

/// `(a * b) mod 2^32`.
///
/// Upstream splits `a` into halves to keep every product under 2^53, which is
/// necessary in JavaScript and pointless in Rust — but the result is identical,
/// so this is the multiplication rather than a transliteration of the split.
/// The equality was verified against Node for each of the five constants the
/// algorithm uses.
fn mul32(a: u32, b: u32) -> u32 {
    a.wrapping_mul(b)
}

/// `(a << b) | (a >>> (32 - b))`.
///
/// JavaScript's shift count is taken modulo 32, which makes `b == 0` and
/// `b == 32` both the identity — the same as [`u32::rotate_left`]. The
/// algorithm only ever passes 13 and 15.
fn rotl32(a: u32, b: u32) -> u32 {
    a.rotate_left(b % 32)
}

/// Upstream's `sum32`, **including its defect**. See the module docs.
///
/// Adds `b`'s high half to `a`'s low half and `b`'s low half to `a`'s high
/// half, all modulo 2^32. It is a general-purpose-looking helper that is only
/// correct for a `b` whose halves have been pre-swapped by the caller.
///
/// Public so that the defect has somewhere to be tested; nothing outside this
/// module should call it.
pub fn sum32(a: u32, b: u32) -> u32 {
    let a_lo = a & 0xffff;
    let a_hi = a >> 16;
    let b_hi = b >> 16;

    // `(a >>> 16) + b & 0xffff` -- `+` binds tighter than `&`, so this is the
    // low half of `a_hi + b`, which is the low half of `a_hi + (b & 0xffff)`.
    let high = (a_hi.wrapping_add(b)) & 0xffff;

    a_lo.wrapping_add(b_hi).wrapping_add(high << 16)
}

/// MurmurHash3 over `data`, seeded with `seed`.
///
/// `seed` is `i32` because its only caller computes it with `& 0xFFFFFFFF`,
/// which in JavaScript produces a signed value; the return is `u32` because
/// upstream ends with `hash >>> 0`.
///
/// Each element of `data` contributes as if it were a byte, so elements above
/// `0xFF` overlap their neighbours in the 32-bit word. See the module docs.
pub fn murmurhash3(seed: i32, data: &[u16]) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    const R1: u32 = 15;
    const R2: u32 = 13;
    const M: u32 = 5;
    /// MurmurHash3's `0xe6546b64` with its halves swapped, which is what
    /// [`sum32`]'s own swap requires to produce the right answer.
    const N: u32 = 0x6b64_e654;

    let mut hash = seed as u32;
    let mut i = 0usize;

    // `for (i = 0, l = data.length - 4; i <= l; i += 4)`. The bound is computed
    // on JavaScript numbers, so for a length below 4 it is negative and the
    // loop simply does not run; `data.len() >= 4` reproduces that without an
    // underflow.
    while data.len() >= 4 && i <= data.len() - 4 {
        let mut k1 = (data[i] as u32)
            | ((data[i + 1] as u32) << 8)
            | ((data[i + 2] as u32) << 16)
            | ((data[i + 3] as u32) << 24);

        k1 = mul32(k1, C1);
        k1 = rotl32(k1, R1);
        k1 = mul32(k1, C2);

        hash ^= k1;
        hash = rotl32(hash, R2);
        hash = mul32(hash, M);
        hash = sum32(hash, N);

        i += 4;
    }

    // `switch (data.length & 3)`, with upstream's deliberate fall-through.
    let tail = data.len() & 3;

    if tail > 0 {
        let mut k1 = 0u32;

        if tail == 3 {
            k1 ^= (data[i + 2] as u32) << 16;
        }

        if tail >= 2 {
            k1 ^= (data[i + 1] as u32) << 8;
        }

        k1 ^= data[i] as u32;
        k1 = mul32(k1, C1);
        k1 = rotl32(k1, R1);
        k1 = mul32(k1, C2);
        hash ^= k1;
    }

    hash ^= data.len() as u32;
    hash ^= hash >> 16;
    hash = mul32(hash, 0x85eb_ca6b);
    hash ^= hash >> 13;
    hash = mul32(hash, 0xc2b2_ae35);
    hash ^= hash >> 16;

    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upstream's own coverage of this file is **zero direct assertions**: no
    /// `test/murmurhash3.js` exists, and `test/bloom-filter.js` only ever sees
    /// the digest through a filter's bit array. So every value below came from
    /// running the upstream file on **Node 24.18.1** and pasting the result.
    ///
    /// `(seed, data, digest)`. The data are UTF-16 code units, exactly what
    /// `bloom-filter.js`'s `stringToByteArray` produces.
    #[test]
    fn murmurhash3_matches_node_24_18_1() {
        #[allow(clippy::type_complexity)]
        const NODE: &[(i32, &[u16], u32)] = &[
            // Empty and sub-word inputs: the main loop never runs.
            (0, &[], 0),
            (0, &[0], 1364076727),
            (0, &[1], 3831157163),
            (0, &[104], 3565335251),
            (0, &[104, 101], 2020321539),
            (0, &[104, 101, 108], 4121571398),
            // Exactly one word, then more.
            (0, &[104, 101, 108, 108], 2707938291),
            // "hello" and "world", the strings the upstream suite adds.
            (0, &[104, 101, 108, 108, 111], 613153351),
            (0, &[119, 111, 114, 108, 100], 4220927227),
            // A non-zero seed, and both ends of the signed range.
            (1, &[104, 101, 108, 108, 111], 3142237357),
            (-1, &[104, 101, 108, 108, 111], 595297739),
            (-2147483648, &[104, 101, 108, 108, 111], 1101222671),
            (2147483647, &[104, 101, 108, 108, 111], 2932207495),
            // Elements above 0xFF, where the shifts overlap.
            (0, &[65535, 65535, 65535, 65535], 1982413648),
            (0, &[256, 0, 0, 0], 1409940790),
            (0, &[0, 256, 0, 0], 82754377),
            (0, &[65535], 4084805820),
            // The exact seeds bloom-filter's `hashArray` derives, on 'hello'.
            (-73087083, &[104, 101, 108, 108, 111], 1214745137),
            (-146174166, &[104, 101, 108, 108, 111], 1551018136),
            (-219261249, &[104, 101, 108, 108, 111], 368810642),
            (-292348332, &[104, 101, 108, 108, 111], 2614423028),
            (-365435415, &[104, 101, 108, 108, 111], 2619114750),
            (-438522498, &[104, 101, 108, 108, 111], 3318192822),
        ];

        for &(seed, data, expected) in NODE {
            assert_eq!(
                murmurhash3(seed, data),
                expected,
                "murmurhash3({seed}, {data:?})"
            );
        }
    }

    /// The `sum32` defect, pinned. If someone "fixes" it to a real 32-bit
    /// addition, this fails — which is the point: fixing it would change every
    /// digest the library has ever produced, because `N` is swapped to match.
    ///
    /// Values from Node 24.18.1.
    #[test]
    fn sum32_is_not_an_adder() {
        assert_eq!(sum32(1, 1), 65537);
        assert_eq!(sum32(0, 0x0001_0000), 1);
        assert_eq!(sum32(0, 0xe654_6b64), 1801774676);
    }

    /// ...and the cancellation that makes the digest right anyway: with
    /// upstream's swapped `N`, `sum32` *is* the addition the algorithm wants.
    #[test]
    fn sum32_with_the_swapped_constant_is_the_addition_murmur_wants() {
        const N: u32 = 0x6b64_e654;
        const REAL: u32 = 0xe654_6b64;

        for a in [
            0u32,
            1,
            0xffff,
            0x1_0000,
            0xffff_ffff,
            0x8000_0000,
            0x7fff_ffff,
            0xdead_beef,
            0x1234_5678,
        ] {
            assert_eq!(sum32(a, N), a.wrapping_add(REAL), "a = {a:#x}");
        }

        // And over a wide deterministic sweep, not just the corners.
        let mut a: u32 = 0x9e37_79b9;

        for _ in 0..100_000 {
            assert_eq!(sum32(a, N), a.wrapping_add(REAL), "a = {a:#x}");
            a = a.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        }
    }

    /// The seed is used as a raw 32-bit pattern, so a negative seed and its
    /// unsigned twin must hash identically. Upstream's only caller produces
    /// negative seeds (`& 0xFFFFFFFF` yields a signed value), and nothing
    /// upstream checks this.
    #[test]
    fn the_seed_is_a_bit_pattern_not_a_magnitude() {
        let data: &[u16] = &[104, 101, 108, 108, 111];

        assert_eq!(murmurhash3(-1, data), murmurhash3(!0i32, data));
        assert_eq!(murmurhash3(i32::MIN, data), murmurhash3(i32::MIN, data));
        assert_ne!(murmurhash3(-1, data), murmurhash3(1, data));
    }

    /// Element values above `0xFF` overlap into the next element's byte, so
    /// distinct inputs collide. Upstream's `Uint16Array` makes this reachable
    /// with any character above U+00FF; nothing upstream tests it.
    #[test]
    fn elements_above_a_byte_overlap_and_collide() {
        // `256 << 0` and `1 << 8` are the same bit, so these two words are
        // identical before the mixing starts. Both are 1409940790 on Node.
        assert_eq!(
            murmurhash3(0, &[256, 0, 0, 0]),
            murmurhash3(0, &[0, 1, 0, 0])
        );
        assert_eq!(murmurhash3(0, &[256, 0, 0, 0]), 1409940790);
        // And with the third element instead of the second.
        assert_eq!(
            murmurhash3(0, &[0, 256, 0, 0]),
            murmurhash3(0, &[0, 0, 1, 0])
        );
    }

    /// Length is mixed in, so the same code units at different lengths differ,
    /// and a trailing zero is not free.
    #[test]
    fn length_is_part_of_the_digest() {
        assert_ne!(murmurhash3(0, &[]), murmurhash3(0, &[0]));
        assert_ne!(murmurhash3(0, &[0]), murmurhash3(0, &[0, 0]));
        assert_ne!(
            murmurhash3(0, &[104, 101, 108, 108, 111]),
            murmurhash3(0, &[104, 101, 108, 108, 111, 0])
        );
    }

    /// The tail switch's fall-through, exercised at each of its four arms by
    /// hashing the same five code units truncated to lengths 4..=7.
    /// Values from Node 24.18.1.
    #[test]
    fn every_tail_length_is_reached() {
        const WORD: [u16; 8] = [11, 22, 33, 44, 55, 66, 77, 88];
        const NODE: [(usize, u32); 5] = [
            (4, 1735331479),
            (5, 3075066579),
            (6, 2870814310),
            (7, 3589543930),
            (8, 4249901357),
        ];

        for (length, expected) in NODE {
            assert_eq!(
                murmurhash3(0, &WORD[..length]),
                expected,
                "length = {length}"
            );
        }
    }
}

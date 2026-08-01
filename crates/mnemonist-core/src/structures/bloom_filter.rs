//! Port of upstream `bloom-filter.js` (mnemonist v0.40.4).
//!
//! A classic Bloom filter over MurmurHash3, sized from a capacity and a target
//! false-positive rate. The unit is this file plus
//! [`crate::utils::murmurhash3`]; `obliterator/foreach` is the third member of
//! the require-closure and already exists at the boundary
//! (`crates/mnemonist-napi/src/foreach.rs`), which is where it belongs.
//!
//! # Items are `&[u16]`, not `&str`
//!
//! Upstream converts an item with
//!
//! ```js
//! var array = new Uint16Array(string.length);
//! for (i = 0; i < string.length; i++) array[i] = string.charCodeAt(i);
//! ```
//!
//! so the hashed sequence is the string's **UTF-16 code units**, one per
//! element, and `murmurhash3` then reads each 16-bit element as if it were a
//! byte. Taking `&str` here would mean hashing UTF-8, which produces different
//! bits for every non-ASCII input and would silently make this a different
//! filter. The bridge does the `charCodeAt` conversion; the core takes what
//! comes out of it.
//!
//! # Three upstream defects, all reproduced
//!
//! **B-97 — a filter with zero hash functions says yes to everything.**
//! `hashFunctions` is `(length * 8 / capacity * Math.LN2) | 0`, and nothing
//! checks the result. When it truncates to `0`, [`BloomFilter::add`] writes no
//! bits and [`BloomFilter::test`] returns `true` vacuously — the loop it would
//! have failed in never runs. This is not an exotic corner:
//! `new BloomFilter(0.5)` reaches it, and `0.5` passes upstream's own
//! validation, which only requires `typeof capacity === 'number' && capacity > 0`
//! despite the error message saying "positive **integer**".
//!
//! **B-98 — every non-string item hashes identically.** `string.length` on a
//! number is `undefined`, `new Uint16Array(undefined)` is empty, and the loop
//! never runs, so `add(42)` hashes the empty sequence — the same sequence
//! `add('')` hashes. After `add(42)`, `test(7)` and `test('')` are both `true`.
//!
//! **B-99 — an `errorRate` above 1 raises a raw `RangeError`.** `Math.log` of
//! anything above 1 is positive, `bits` goes negative, and `new Uint8Array(-59)`
//! throws `RangeError: Invalid typed array length: -59` from the allocator —
//! not the module's own message, and only for a *large enough* capacity, since
//! `(-7.2 / 8) | 0` truncates to `0` and allocates an empty filter instead.
//!
//! All three verified against Node 24.18.1; see `docs/modules/bloom-filter.md`.

use crate::utils::bitwise::to_int32;
use crate::utils::murmurhash3::murmurhash3;

/// `Math.LN2 * Math.LN2`.
const LN2_SQUARED: f64 = std::f64::consts::LN_2 * std::f64::consts::LN_2;

/// `DEFAULTS.errorRate`.
pub const DEFAULT_ERROR_RATE: f64 = 0.005;

/// The seed multiplier `hashArray` derives its per-function seed with.
const SEED_MULTIPLIER: f64 = 0xFBA4_C795u32 as f64;

/// Why a filter could not be built.
///
/// Three variants rather than one string, because upstream raises two
/// *different JavaScript error classes* here and the bridge has to pick the
/// right one: its own `Error` for the two validation failures, and the
/// allocator's `RangeError` for the third.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildError {
    /// `capacity` is absent, not a number, or not positive.
    Capacity,
    /// `errorRate` was supplied and is not a positive number.
    ErrorRate,
    /// `new Uint8Array(n)` with `n` negative. Carries `n` so the bridge can
    /// reproduce upstream's message verbatim.
    InvalidLength(i32),
}

impl BuildError {
    /// Upstream's message, verbatim, so a bridge can re-throw it unchanged and
    /// upstream's own `assert.throws(..., /capacity/)` still matches.
    pub fn message(&self) -> String {
        match self {
            Self::Capacity => String::from(
                "mnemonist/BloomFilter.constructor: `capacity` option should be a positive integer.",
            ),
            Self::ErrorRate => String::from(
                "mnemonist/BloomFilter.constructor: `errorRate` option should be a positive float.",
            ),
            Self::InvalidLength(length) => format!("Invalid typed array length: {length}"),
        }
    }
}

/// A Bloom filter.
#[derive(Debug, Clone)]
pub struct BloomFilter {
    capacity: f64,
    error_rate: f64,
    hash_functions: usize,
    data: Vec<u8>,
}

impl BloomFilter {
    /// Build a filter for `capacity` items at `error_rate`.
    ///
    /// `error_rate` is `None` when the caller omitted the option, which is not
    /// the same as passing `0`: upstream validates the *option* (`options.errorRate <= 0`)
    /// while storing `options.errorRate || DEFAULTS.errorRate`. So an omitted
    /// rate defaults silently, an explicit `0` throws, and an explicit `NaN`
    /// **also** defaults silently — `NaN` is falsy, and `NaN <= 0` is false.
    /// All three are reproduced.
    ///
    /// # Errors
    ///
    /// See [`BuildError`].
    pub fn new(capacity: f64, error_rate: Option<f64>) -> Result<Self, BuildError> {
        // `typeof options.capacity !== 'number' || options.capacity <= 0`.
        // `NaN <= 0` is false in JavaScript, so a NaN capacity gets through
        // here exactly as it does upstream, and falls out later as a zero-
        // length filter.
        if capacity <= 0.0 {
            return Err(BuildError::Capacity);
        }

        // `this.errorRate = options.errorRate || DEFAULTS.errorRate`.
        let stored = match error_rate {
            Some(rate) if rate != 0.0 && !rate.is_nan() => rate,
            _ => DEFAULT_ERROR_RATE,
        };

        // `... || options.errorRate <= 0` -- the OPTION, not the stored value.
        if matches!(error_rate, Some(rate) if rate <= 0.0) {
            return Err(BuildError::ErrorRate);
        }

        let mut filter = Self {
            capacity,
            error_rate: stored,
            hash_functions: 0,
            data: Vec::new(),
        };

        filter.clear()?;

        Ok(filter)
    }

    /// `#.clear` — recompute the sizing and drop every bit.
    ///
    /// Upstream's `clear` is not merely a reset: it re-derives `hashFunctions`
    /// and reallocates `data` from `capacity` and `errorRate`, so it can throw
    /// exactly where the constructor can (B-99). Reproduced, including the
    /// order — `hashFunctions` is assigned before the allocation that fails.
    ///
    /// # Errors
    ///
    /// [`BuildError::InvalidLength`] when the sizing goes negative.
    pub fn clear(&mut self) -> Result<(), BuildError> {
        let bits = -1.0 / LN2_SQUARED * self.capacity * self.error_rate.ln();
        let length = to_int32(bits / 8.0);

        self.hash_functions =
            to_int32(length as f64 * 8.0 / self.capacity * std::f64::consts::LN_2).max(0) as usize;

        if length < 0 {
            return Err(BuildError::InvalidLength(length));
        }

        self.data = vec![0; length as usize];

        Ok(())
    }

    /// `#.capacity`.
    pub fn capacity(&self) -> f64 {
        self.capacity
    }

    /// `#.errorRate`.
    pub fn error_rate(&self) -> f64 {
        self.error_rate
    }

    /// `#.hashFunctions` — how many bits each item sets.
    ///
    /// **Zero is reachable and is not an error upstream.** See B-97.
    pub fn hash_functions(&self) -> usize {
        self.hash_functions
    }

    /// `#.data` / `#.toJSON` — the bit array.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// `hashArray(length, seed, array)`.
    ///
    /// `(seed * 0xFBA4C795) & 0xFFFFFFFF` is done on JavaScript numbers, so the
    /// product is exact — `seed` is at most a few dozen — and the mask is a
    /// ToInt32, which is why [`murmurhash3`] takes a signed seed.
    ///
    /// The `%` is only reached when `data` is non-empty: `hashFunctions` is `0`
    /// whenever `data` is empty, so the loops that call this never run. That is
    /// load-bearing rather than incidental — upstream's `hash % (0 * 8)` is
    /// `NaN`, and `data[NaN >> 3] |= …` would be a silent no-op.
    fn hash(&self, seed: usize, item: &[u16]) -> usize {
        let seed = to_int32(seed as f64 * SEED_MULTIPLIER);
        let hash = murmurhash3(seed, item);

        (hash as u64 % (self.data.len() as u64 * 8)) as usize
    }

    /// `#.add` — record `item`.
    ///
    /// A filter with zero hash functions records nothing, silently. See B-97.
    pub fn add(&mut self, item: &[u16]) {
        for seed in 0..self.hash_functions {
            let index = self.hash(seed, item);

            self.data[index >> 3] |= 1 << (7 & index);
        }
    }

    /// `#.test` — whether `item` might have been added.
    ///
    /// Returns `true` for anything at all when there are zero hash functions,
    /// because the loop that would have returned `false` never runs. See B-97.
    pub fn test(&self, item: &[u16]) -> bool {
        for seed in 0..self.hash_functions {
            let index = self.hash(seed, item);

            if self.data[index >> 3] & (1 << (7 & index)) == 0 {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(value: &str) -> Vec<u16> {
        value.encode_utf16().collect()
    }

    // ------------------------------------------------- upstream's own suite

    /// `test/bloom-filter.js`, `'should compute the correct settings.'`
    #[test]
    fn matches_the_upstream_suites_own_settings() {
        let filter = BloomFilter::new(3.0, None).unwrap();

        assert_eq!(filter.data().len(), 4);
        assert_eq!(filter.hash_functions(), 7);
    }

    /// `'should be possible to add items to the filter.'` — the exact byte
    /// arrays upstream asserts, after each of three adds.
    #[test]
    fn matches_the_upstream_suites_own_bit_arrays() {
        let mut filter = BloomFilter::new(3.0, None).unwrap();

        filter.add(&units("hello"));
        assert_eq!(filter.data(), [128, 0, 86, 65]);

        filter.add(&units("world"));
        assert_eq!(filter.data(), [131, 130, 94, 89]);

        filter.add(&units("longer string"));
        assert_eq!(filter.data(), [167, 130, 95, 121]);
    }

    /// `'should be possible to insert more items.'` — 50 items into a
    /// capacity-50 filter, against upstream's 68-byte expectation.
    #[test]
    fn matches_the_upstream_suites_own_fifty_item_case() {
        let mut filter = BloomFilter::new(50.0, None).unwrap();

        for i in 0..50 {
            let item = if i % 2 == 1 {
                format!("hello{i}")
            } else {
                format!("world{i}")
            };

            filter.add(&units(&item));
        }

        assert_eq!(
            filter.data(),
            [
                168, 120, 121, 113, 105, 114, 37, 230, 138, 115, 203, 112, 167, 31, 235, 139, 90,
                200, 77, 118, 194, 243, 25, 93, 128, 18, 115, 178, 23, 200, 73, 134, 160, 117, 57,
                192, 116, 205, 164, 241, 63, 169, 140, 184, 195, 92, 45, 15, 33, 254, 79, 217, 147,
                240, 50, 100, 251, 96, 216, 34, 104, 35, 6, 17, 179, 77, 146, 178
            ]
        );
    }

    /// `'should be possible to test items.'`
    #[test]
    fn matches_the_upstream_suites_own_membership_case() {
        let mut filter = BloomFilter::new(3.0, None).unwrap();

        filter.add(&units("hello"));

        assert!(filter.test(&units("hello")));
        assert!(!filter.test(&units("world")));
    }

    /// `'should throw when given options are invalid.'` — the two validation
    /// failures the core owns. The third (`new BloomFilter()` with no argument
    /// at all) is a falsy-argument check that lives at the bridge.
    #[test]
    fn matches_the_upstream_suites_own_validation() {
        assert_eq!(
            BloomFilter::new(-34.0, None).unwrap_err(),
            BuildError::Capacity
        );
        assert_eq!(
            BloomFilter::new(3.0, Some(-45.0)).unwrap_err(),
            BuildError::ErrorRate
        );
        assert!(BloomFilter::new(-34.0, None)
            .unwrap_err()
            .message()
            .contains("capacity"));
        assert!(BloomFilter::new(3.0, Some(-45.0))
            .unwrap_err()
            .message()
            .contains("errorRate"));
    }

    // ------------------------------------------------------------ the bugs

    /// **B-97**: `hashFunctions` truncating to zero makes `test` return `true`
    /// for everything, and `add` a no-op. Reachable from a filter that passes
    /// every one of upstream's own validations. Values from Node 24.18.1.
    #[test]
    fn b97_a_filter_with_no_hash_functions_says_yes_to_everything() {
        for (capacity, error_rate, length) in [
            // A positive, entirely legal-looking capacity below 1. Upstream's
            // message says "positive integer"; its check says `> 0`.
            (0.5, None, 0),
            (3.0, Some(0.9), 0),
            // ...and one where `data` is NOT empty, so this is not simply
            // "an empty filter".
            (10.0, Some(0.5), 1),
        ] {
            let mut filter = BloomFilter::new(capacity, error_rate).unwrap();

            assert_eq!(filter.hash_functions(), 0, "capacity {capacity}");
            assert_eq!(filter.data().len(), length, "capacity {capacity}");

            assert!(filter.test(&units("anything")), "capacity {capacity}");
            assert!(filter.test(&units("")), "capacity {capacity}");

            filter.add(&units("anything"));

            // Nothing was recorded, and everything still tests positive.
            assert!(filter.data().iter().all(|&byte| byte == 0));
            assert!(filter.test(&units("something else")));
        }
    }

    /// **B-98**: every item without a `length` collapses onto the empty
    /// sequence, so a filter of numbers reports every number present.
    ///
    /// The conversion happens at the bridge, so what the core can pin is the
    /// half that makes it possible: the empty sequence is a perfectly ordinary
    /// hashable item, and it sets real bits.
    #[test]
    fn b98_the_empty_sequence_is_an_ordinary_item() {
        let mut filter = BloomFilter::new(10.0, None).unwrap();

        assert!(!filter.test(&[]));
        filter.add(&[]);
        assert!(filter.test(&[]));
        // Verified against Node: `new BloomFilter(10).add('')` gives exactly
        // this. Anything upstream converts to an empty Uint16Array -- every
        // number, every boolean, every plain object -- lands on the same bits.
        assert_eq!(filter.data(), [1, 1, 0, 0, 64, 128, 0, 0, 0, 0, 1, 17, 0]);
    }

    /// **B-99**: an `errorRate` above 1 makes the sizing negative, and a large
    /// enough capacity turns that into an allocation failure rather than an
    /// empty filter. Lengths from Node 24.18.1's own `RangeError` messages.
    #[test]
    fn b99_an_error_rate_above_one_is_a_range_error_only_sometimes() {
        assert_eq!(
            BloomFilter::new(50.0, Some(100.0)).unwrap_err(),
            BuildError::InvalidLength(-59)
        );
        assert_eq!(
            BloomFilter::new(50.0, Some(3.0)).unwrap_err(),
            BuildError::InvalidLength(-14)
        );
        assert_eq!(
            BloomFilter::new(20.0, Some(1.5)).unwrap_err(),
            BuildError::InvalidLength(-2)
        );
        assert_eq!(
            BloomFilter::new(50.0, Some(100.0)).unwrap_err().message(),
            "Invalid typed array length: -59"
        );

        // ...but a small capacity truncates the same negative value to zero and
        // builds a filter that says yes to everything (B-97 again).
        let filter = BloomFilter::new(5.0, Some(2.0)).unwrap();

        assert_eq!(filter.data().len(), 0);
        assert!(filter.test(&units("anything")));
    }

    // --------------------------------------------------------------- gaps

    /// The sizing table, against Node 24.18.1. Upstream asserts one row of
    /// this (`capacity 3`); the interesting ones are the boundaries where a
    /// truncation changes the answer.
    #[test]
    fn settings_match_node_24_18_1() {
        // (capacity, errorRate or None, data.length, hashFunctions)
        const NODE: &[(f64, Option<f64>, usize, usize)] = &[
            (1.0, None, 1, 5),
            (2.0, None, 2, 5),
            (3.0, None, 4, 7),
            (10.0, None, 13, 7),
            (50.0, None, 68, 7),
            (100.0, None, 137, 7),
            (1000.0, None, 1378, 7),
            (10.0, Some(0.001), 17, 9),
            (10.0, Some(0.01), 11, 6),
            (10.0, Some(0.1), 5, 2),
            (10.0, Some(0.5), 1, 0),
            (3.0, Some(0.9), 0, 0),
            // A fractional capacity, which upstream's "positive integer"
            // message forbids and its check allows.
            (2.5, None, 3, 6),
            (0.5, None, 0, 0),
            (1_000_000.0, None, 1_378_469, 7),
        ];

        for &(capacity, error_rate, length, hash_functions) in NODE {
            let filter = BloomFilter::new(capacity, error_rate)
                .unwrap_or_else(|error| panic!("({capacity}, {error_rate:?}): {error:?}"));

            assert_eq!(
                filter.data().len(),
                length,
                "data.length for ({capacity}, {error_rate:?})"
            );
            assert_eq!(
                filter.hash_functions(),
                hash_functions,
                "hashFunctions for ({capacity}, {error_rate:?})"
            );
        }
    }

    /// Per-item bit patterns against Node 24.18.1, on a fresh capacity-10
    /// filter each time. Upstream's suite only ever checks cumulative state on
    /// a capacity-3 filter, so a hash defect that happened to preserve those
    /// four bytes would go unnoticed.
    ///
    /// The last two rows are the point: a BMP character above U+00FF and an
    /// astral character, which becomes **two** code units. Upstream tests
    /// neither, and both are where `murmurhash3`'s 16-bit-elements-as-bytes
    /// overlap becomes observable.
    #[test]
    fn per_item_bits_match_node_24_18_1() {
        #[allow(clippy::type_complexity)]
        const NODE: &[(&[u16], &[u8])] = &[
            // "hello"
            (
                &[104, 101, 108, 108, 111],
                &[0, 0, 68, 128, 0, 0, 0, 0, 64, 2, 0, 0, 17],
            ),
            // "world"
            (
                &[119, 111, 114, 108, 100],
                &[0, 0, 0, 16, 0, 130, 0, 0, 2, 8, 9, 0, 0],
            ),
            // "longer string"
            (
                &[
                    108, 111, 110, 103, 101, 114, 32, 115, 116, 114, 105, 110, 103,
                ],
                &[0, 2, 0, 0, 32, 4, 5, 8, 0, 0, 0, 0, 32],
            ),
            // ""
            (&[], &[1, 1, 0, 0, 64, 128, 0, 0, 0, 0, 1, 17, 0]),
            // "a"
            (&[97], &[0, 32, 0, 16, 0, 0, 2, 0, 0, 0, 64, 0, 164]),
            // "\u{0}" -- a code unit equal to the sentinel murmur's tail pads with
            (&[0], &[0, 0, 128, 0, 128, 128, 0, 0, 16, 4, 128, 0, 128]),
            // "日本"
            (
                &[26085, 26412],
                &[0, 0, 16, 0, 0, 0, 0, 30, 16, 0, 0, 0, 128],
            ),
            // "😀" -- one code POINT, two code UNITS
            (
                &[55357, 56832],
                &[4, 0, 0, 0, 0, 0, 128, 18, 0, 4, 32, 4, 0],
            ),
        ];

        for &(item, expected) in NODE {
            let mut filter = BloomFilter::new(10.0, None).unwrap();

            filter.add(item);

            assert_eq!(filter.data(), expected, "item {item:?}");
            assert!(filter.test(item), "item {item:?}");
        }
    }

    /// `#.clear` re-derives the sizing rather than only zeroing. Upstream
    /// defines it, its suite never calls it, and it is the one method that can
    /// throw after construction.
    #[test]
    fn clear_resets_the_bits_and_keeps_the_sizing() {
        let mut filter = BloomFilter::new(3.0, None).unwrap();

        filter.add(&units("hello"));
        assert_eq!(filter.data(), [128, 0, 86, 65]);

        filter.clear().unwrap();

        assert_eq!(filter.data(), [0, 0, 0, 0]);
        assert_eq!(filter.hash_functions(), 7);
        assert!(!filter.test(&units("hello")));
    }

    /// The `errorRate` validation's three-way split, which upstream's single
    /// `assert.throws` cannot see: an omitted rate defaults, an explicit `0`
    /// throws, and an explicit `NaN` **also** defaults — because `NaN` is falsy
    /// and `NaN <= 0` is false. Verified against Node.
    #[test]
    fn the_error_rate_check_reads_the_option_and_the_default_reads_the_value() {
        assert_eq!(
            BloomFilter::new(5.0, None).unwrap().error_rate(),
            DEFAULT_ERROR_RATE
        );
        assert_eq!(
            BloomFilter::new(5.0, Some(0.0)).unwrap_err(),
            BuildError::ErrorRate
        );
        assert_eq!(
            BloomFilter::new(5.0, Some(f64::NAN)).unwrap().error_rate(),
            DEFAULT_ERROR_RATE
        );
        // ...and the NaN filter is a normal one, sized from the default.
        assert_eq!(
            BloomFilter::new(5.0, Some(f64::NAN)).unwrap().data().len(),
            6
        );
    }

    /// An infinite capacity passes validation and produces an empty filter
    /// rather than an error, because `(Infinity / 8) | 0` is `0`. Verified
    /// against Node.
    #[test]
    fn an_infinite_capacity_produces_an_empty_filter() {
        let filter = BloomFilter::new(f64::INFINITY, None).unwrap();

        assert_eq!(filter.data().len(), 0);
        assert_eq!(filter.hash_functions(), 0);
        assert!(filter.test(&units("anything")));
    }

    /// No false negatives, which is the only guarantee a Bloom filter makes and
    /// the one upstream's suite checks with a single item. Here: 200 items in a
    /// filter sized for 200, every one of them found.
    #[test]
    fn never_reports_a_false_negative() {
        let mut filter = BloomFilter::new(200.0, None).unwrap();
        let items: Vec<Vec<u16>> = (0..200).map(|i| units(&format!("item-{i}"))).collect();

        for item in &items {
            filter.add(item);
        }

        for item in &items {
            assert!(filter.test(item), "false negative for {item:?}");
        }
    }

    /// ...and the false-positive rate is in the neighbourhood the sizing
    /// promises. Not a tight bound — 0.5% nominal, asserted below 5% — because
    /// the point is to catch a hash that has collapsed, not to re-derive the
    /// Bloom bound.
    #[test]
    fn the_false_positive_rate_is_roughly_what_was_asked_for() {
        let mut filter = BloomFilter::new(500.0, None).unwrap();

        for i in 0..500 {
            filter.add(&units(&format!("in-{i}")));
        }

        let positives = (0..2000)
            .filter(|i| filter.test(&units(&format!("out-{i}"))))
            .count();

        assert!(
            positives < 100,
            "{positives}/2000 false positives at a nominal 0.5%; the hash has collapsed"
        );
    }
}

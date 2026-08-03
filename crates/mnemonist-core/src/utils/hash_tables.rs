//! Port of upstream `utils/hash-tables.js` (mnemonist v0.40.4).
//!
//! Bob Jenkins' 32-bit integer hash, plus three open-addressing helpers that
//! implement linear probing over a caller-owned pair of parallel arrays.
//!
//! # Scope note: this is infrastructure, not a unit
//!
//! Same standing as [`crate::utils::binary_search`]: there is no
//! `test/hash-tables.js`, only a single `it()` inside `test/_utils.js`, whose
//! require-closure needs `merge` and `iterables` and therefore cannot run yet.
//! Gates 1, 2, 7 and 8 apply; it never appears in `tests/scope.txt` on its own.
//!
//! Nothing in the shipped library calls these helpers at all — `git grep` finds
//! exactly one caller, `benchmark/misc/hashmap.js`, which is not part of the
//! published package. They exist for the benefit of future structures.
//!
//! # `0` is the empty sentinel, so `0` is not a storable key
//!
//! [`linear_probing_get`] stops on `c === 0` meaning "empty slot". A caller who
//! stores the key `0` gets a slot that reads back as empty: [`linear_probing_set`]
//! writes it, and [`linear_probing_get`] then returns `None` for it *and* stops
//! probing there, hiding every key that collided behind it. Upstream documents
//! none of this and its one test never uses `0`. Reproduced, not fixed.
//!
//! # The table length must be a power of two
//!
//! The initial slot is `hash(key) & (n - 1)`, which is only a modulo when `n` is
//! a power of two. For any other `n` the mask can select a slot at or past the
//! end — upstream then reads `undefined`, treats it as "occupied but not equal",
//! and probes on. Reproduced through `slot`, which returns `None` for an
//! out-of-range read exactly as JavaScript does.
//!
//! # Deliberate divergences
//!
//! * **A full table returns `Err`, not a thrown `Error`.** `mnemonist-core` has
//!   no exceptions; the message is upstream's, verbatim, so a bridge can
//!   re-throw it unchanged.
//! * **`n == 0` returns `Err`/`None` instead of hanging.** Upstream computes
//!   `i %= 0`, gets `NaN`, and then `i === j` is never true, so
//!   `linearProbingGet` on a zero-length table **loops forever**. Verified
//!   against Node 24.18.1 (the call does not return). An infinite loop is not a
//!   behaviour worth reproducing; the guard is stated here and in
//!   `docs/modules/utils-hash-tables.md`.

/// The message upstream throws when a linear-probing insert cannot find a slot.
pub const TABLE_IS_FULL: &str = "mnemonist/utils/hash-tables.linearProbingSet: table is full.";

/// Bob Jenkins' 32-bit integer hash, as upstream spells it.
///
/// # Why this is `i32` arithmetic and why that is exact
///
/// Upstream alternates `+` (ordinary IEEE-754 addition, no truncation) with
/// `^`, `<<` and `>>` (each of which applies ToInt32 to both operands). The
/// intermediate sums therefore leave the 32-bit range — `(a + 0x7ed55d16) + (a << 12)`
/// can reach 2^33 — and are only folded back on the next bitwise operator.
///
/// Every step is nevertheless congruent modulo 2^32, and every intermediate is
/// an exact integer well inside 2^53, so the whole function is equal to the
/// same computation done in wrapping 32-bit arithmetic. That equality was
/// checked against Node rather than assumed; see the tests.
pub fn jenkins_int32(a: i32) -> i32 {
    let mut a = a as u32;

    a = a.wrapping_add(0x7ed5_5d16).wrapping_add(a << 12);
    a = (a ^ 0xc761_c23c) ^ ((a as i32) >> 19) as u32;
    a = a.wrapping_add(0x1656_67b1).wrapping_add(a << 5);
    a = a.wrapping_add(0xd3a2_646c) ^ (a << 9);
    a = a.wrapping_add(0xfd70_46c5).wrapping_add(a << 3);
    a = (a ^ 0xb55a_4f09) ^ ((a as i32) >> 16) as u32;

    a as i32
}

/// `array[index]` where `index` came from a mask that may overshoot.
fn slot<T: Copy>(array: &[T], index: usize) -> Option<T> {
    array.get(index).copied()
}

/// The starting probe position, `hash(key) & (n - 1)`.
///
/// `n - 1` goes through ToInt32 in JavaScript, so for `n == 0` the mask is `-1`
/// and the result is the raw hash — negative for half of all keys, which is how
/// upstream ends up indexing an array with a negative number. Callers here
/// reject `n == 0` before reaching this.
fn start<F: Fn(u32) -> i32>(hash: F, n: usize, key: u32) -> usize {
    (hash(key) & (n as i32 - 1)) as usize
}

/// Value stored under `key`, or `None` if absent.
///
/// Probes forward from `hash(key) & (n - 1)`, stopping at the key, at an empty
/// slot, or after a full turn.
pub fn linear_probing_get<'a, V, F>(
    hash: F,
    keys: &[u32],
    values: &'a [V],
    key: u32,
) -> Option<&'a V>
where
    F: Fn(u32) -> i32,
{
    let n = keys.len();

    if n == 0 {
        return None;
    }

    let j = start(hash, n, key);
    let mut i = j;

    loop {
        match slot(keys, i) {
            Some(c) if c == key => return values.get(i),
            Some(0) => return None,
            // `undefined` at an out-of-range slot: neither equal nor empty, so
            // upstream probes on.
            _ => {}
        }

        i = (i + 1) % n;

        if i == j {
            return None;
        }
    }
}

/// Whether `key` is present. The same walk as [`linear_probing_get`], without
/// the parallel value array.
pub fn linear_probing_has<F>(hash: F, keys: &[u32], key: u32) -> bool
where
    F: Fn(u32) -> i32,
{
    let n = keys.len();

    if n == 0 {
        return false;
    }

    let j = start(hash, n, key);
    let mut i = j;

    loop {
        match slot(keys, i) {
            Some(c) if c == key => return true,
            Some(0) => return false,
            _ => {}
        }

        i = (i + 1) % n;

        if i == j {
            return false;
        }
    }
}

/// Store `value` under `key`, overwriting an existing entry for the same key.
///
/// Returns [`TABLE_IS_FULL`] when a full turn finds neither the key nor an
/// empty slot; upstream throws an `Error` carrying that same message.
///
/// # Panics
///
/// Never. An out-of-range initial slot — only reachable with a non-power-of-two
/// table, see the module docs — is probed past exactly as upstream does, and if
/// the probe lands back on it the write is refused with [`TABLE_IS_FULL`]
/// rather than performed out of bounds.
pub fn linear_probing_set<V, F>(
    hash: F,
    keys: &mut [u32],
    values: &mut [V],
    key: u32,
    value: V,
) -> Result<(), &'static str>
where
    F: Fn(u32) -> i32,
{
    let n = keys.len();

    if n == 0 {
        return Err(TABLE_IS_FULL);
    }

    let j = start(hash, n, key);
    let mut i = j;

    loop {
        match slot(keys, i) {
            Some(0) => break,
            Some(c) if c == key => break,
            _ => {}
        }

        i = (i + 1) % n;

        if i == j {
            return Err(TABLE_IS_FULL);
        }
    }

    // Upstream writes unconditionally after the loop; `i` is in range here
    // because the only way out of the loop is a slot that was read
    // successfully.
    keys[i] = key;
    values[i] = value;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `jenkinsInt32` against **real Node 24.18.1**, over a hand-picked set of
    /// inputs that straddles every place the port could have gone wrong: the
    /// sign boundary, `-1`, the values where the arithmetic right shifts see a
    /// set bit 31, and the keys upstream's own `test/_utils.js` uses.
    ///
    /// Generated by running the upstream file and pasting the results, so the
    /// crate still builds and tests with no JavaScript runtime present.
    #[test]
    fn jenkins_int32_matches_node_24_18_1() {
        const NODE: &[(i32, i32)] = &[
            (0, -1800283865),
            (1, -1266253386),
            (2, -496519092),
            (3, -1332670820),
            (-1, -26951294),
            (-2, -2115843390),
            (127, -607972253),
            (128, -1404961551),
            (255, -1572223203),
            (256, -823782022),
            (65535, -1820203624),
            (65536, -462644230),
            (2147483647, -603263981),
            (-2147483648, -963664666),
            (-2147483647, -1858835461),
            (1234567890, -989781500),
            (-1234567890, -456063543),
            (4563, -1250256773),
            (534274, -146807220),
            (36464, -446051035),
            (45353, -1278544095),
            (82754, -1505329539),
            (8696007, -1895527640),
            (344994, -1091653662),
            (71654, -1578156136),
            (453, -1706540273),
            (485385, -665050467),
            (48753, -1785254830),
        ];

        for &(input, expected) in NODE {
            assert_eq!(jenkins_int32(input), expected, "jenkinsInt32({input})");
        }
    }

    /// Upstream's own `hash-tables` test, transcribed: eight pairs into an
    /// eight-slot table, then reads, membership, a rejected ninth insert, and
    /// two misses.
    #[test]
    fn linear_probing_matches_the_upstream_suites_own_case() {
        const PAIRS: [(u32, u32); 8] = [
            (4563, 1),
            (534274, 2),
            (36464, 3),
            (45353, 4),
            (82754, 5),
            (8696007, 6),
            (344994, 7),
            (71654, 8),
        ];

        let hash = |key: u32| jenkins_int32(key as i32);
        let mut keys = [0u32; 8];
        let mut values = [0u32; 8];

        for &(key, value) in &PAIRS {
            linear_probing_set(hash, &mut keys, &mut values, key, value).expect("table has room");
        }

        for &(key, value) in &PAIRS {
            assert_eq!(linear_probing_get(hash, &keys, &values, key), Some(&value));
            assert!(linear_probing_has(hash, &keys, key));
        }

        assert_eq!(
            linear_probing_set(hash, &mut keys, &mut values, 453, 9),
            Err(TABLE_IS_FULL)
        );
        assert_eq!(linear_probing_get(hash, &keys, &values, 485385), None);
        assert!(!linear_probing_has(hash, &keys, 48753));
    }

    /// The slot layout the eight upstream pairs actually produce, pinned.
    /// Upstream asserts only that the reads round-trip, so the probe order
    /// itself — which is what a wrong `jenkinsInt32` or a wrong mask would
    /// change — is unchecked there. Values from Node 24.18.1.
    #[test]
    fn the_upstream_pairs_land_in_a_known_layout() {
        let hash = |key: u32| jenkins_int32(key as i32);
        let mut keys = [0u32; 8];
        let mut values = [0u32; 8];

        for (key, value) in [
            (4563u32, 1u32),
            (534274, 2),
            (36464, 3),
            (45353, 4),
            (82754, 5),
            (8696007, 6),
            (344994, 7),
            (71654, 8),
        ] {
            linear_probing_set(hash, &mut keys, &mut values, key, value).expect("table has room");
        }

        assert_eq!(
            keys,
            [8696007, 45353, 344994, 4563, 534274, 36464, 82754, 71654]
        );
        assert_eq!(values, [6, 4, 7, 1, 2, 3, 5, 8]);
    }

    // ---------------------------------------------------------------- gaps

    /// Overwriting an existing key reuses its slot rather than consuming a new
    /// one. Upstream's suite never sets the same key twice.
    #[test]
    fn setting_an_existing_key_overwrites_in_place() {
        let hash = |key: u32| jenkins_int32(key as i32);
        let mut keys = [0u32; 4];
        let mut values = [0u32; 4];

        linear_probing_set(hash, &mut keys, &mut values, 7, 1).unwrap();
        linear_probing_set(hash, &mut keys, &mut values, 7, 2).unwrap();

        assert_eq!(linear_probing_get(hash, &keys, &values, 7), Some(&2));
        assert_eq!(keys.iter().filter(|&&k| k == 7).count(), 1);
        // Three slots still free, so three more keys must fit.
        for key in [11u32, 12, 13] {
            linear_probing_set(hash, &mut keys, &mut values, key, key).unwrap();
        }
    }

    /// `0` is the empty sentinel, so an entry stored under the key `0` occupies
    /// a slot that still *looks* empty — and the next colliding insert
    /// overwrites it without probing past. This is the module's sharpest
    /// untested edge, and it is not the naive "key 0 cannot be read": `get`
    /// checks `c === key` before `c === 0`, so the read works right up until
    /// something else lands on the slot.
    ///
    /// Values from Node 24.18.1.
    #[test]
    fn the_key_zero_occupies_a_slot_that_still_reads_as_empty() {
        // A hash that sends everything to slot 0, so the collision is forced.
        let hash = |_key: u32| 0;
        let mut keys = [0u32; 4];
        let mut values = [0u32; 4];

        linear_probing_set(hash, &mut keys, &mut values, 0, 42).unwrap();
        // Indistinguishable from an untouched table...
        assert_eq!(keys, [0, 0, 0, 0]);
        // ...yet readable, because `c === key` is tested first.
        assert_eq!(linear_probing_get(hash, &keys, &values, 0), Some(&42));
        assert!(linear_probing_has(hash, &keys, 0));

        // A later key does not probe past it, it *overwrites* it, because
        // slot 0 still looks empty to `set`.
        linear_probing_set(hash, &mut keys, &mut values, 5, 43).unwrap();
        assert_eq!(keys, [5, 0, 0, 0]);
        assert_eq!(linear_probing_get(hash, &keys, &values, 5), Some(&43));
        // The 42 is gone; the read now lands on the next still-empty slot.
        assert_eq!(linear_probing_get(hash, &keys, &values, 0), Some(&0));
    }

    /// A full table where the key is absent: `get` and `has` must complete the
    /// turn and report a miss rather than spin. Upstream's suite covers this
    /// once; here it is checked for every starting slot.
    #[test]
    fn a_full_table_terminates_from_every_starting_slot() {
        let mut keys = [1u32, 2, 3, 4];
        let mut values = [10u32, 20, 30, 40];

        for start in 0..4u32 {
            let hash = move |_key: u32| start as i32;

            assert_eq!(linear_probing_get(hash, &keys, &values, 99), None);
            assert!(!linear_probing_has(hash, &keys, 99));
            assert_eq!(
                linear_probing_set(hash, &mut keys, &mut values, 99, 0),
                Err(TABLE_IS_FULL)
            );
        }
    }

    /// A zero-length table. Upstream hangs here; the port refuses. Documented
    /// divergence — see the module docs.
    #[test]
    fn a_zero_length_table_is_refused_rather_than_hung() {
        let hash = |key: u32| jenkins_int32(key as i32);
        let mut keys: [u32; 0] = [];
        let mut values: [u32; 0] = [];

        assert_eq!(linear_probing_get(hash, &keys, &values, 1), None);
        assert!(!linear_probing_has(hash, &keys, 1));
        assert_eq!(
            linear_probing_set(hash, &mut keys, &mut values, 1, 1),
            Err(TABLE_IS_FULL)
        );
    }

    /// A non-power-of-two table, where `hash(key) & (n - 1)` is not a modulo
    /// and can point past the end. The probe walks off, wraps, and still
    /// terminates; nothing is written out of bounds.
    #[test]
    fn a_non_power_of_two_table_still_terminates() {
        // n = 5, so `& (n - 1)` is `& 4`: it can only ever select slot 0 or
        // slot 4, and everything else is reached by probing. Layout verified
        // against Node 24.18.1.
        let hash = |key: u32| key as i32;
        let mut keys = [0u32; 5];
        let mut values = [0u32; 5];

        for key in 1..=5u32 {
            linear_probing_set(hash, &mut keys, &mut values, key, key * 10).unwrap();
        }

        assert_eq!(keys, [1, 2, 3, 5, 4]);

        for key in 1..=5u32 {
            assert!(linear_probing_has(hash, &keys, key));
        }

        assert_eq!(
            linear_probing_set(hash, &mut keys, &mut values, 6, 60),
            Err(TABLE_IS_FULL)
        );
    }

    /// A round trip over the whole table, driven by the real hash: every key
    /// that fits must be findable, and the table must refuse the one that does
    /// not. This is the property upstream checks once, at one size.
    #[test]
    fn round_trips_at_every_power_of_two_size() {
        let hash = |key: u32| jenkins_int32(key as i32);

        for bits in 1..=6u32 {
            let n = 1usize << bits;
            let mut keys = vec![0u32; n];
            let mut values = vec![0u32; n];
            // Keys are 1-based: `0` is the sentinel and cannot be stored.
            let inserted: Vec<u32> = (1..=n as u32).collect();

            for &key in &inserted {
                linear_probing_set(hash, &mut keys, &mut values, key, key * 3)
                    .unwrap_or_else(|error| panic!("n = {n}, key = {key}: {error}"));
            }

            for &key in &inserted {
                assert_eq!(
                    linear_probing_get(hash, &keys, &values, key),
                    Some(&(key * 3)),
                    "n = {n}, key = {key}"
                );
            }

            assert_eq!(
                linear_probing_set(hash, &mut keys, &mut values, n as u32 + 1, 0),
                Err(TABLE_IS_FULL),
                "n = {n}"
            );
        }
    }
}

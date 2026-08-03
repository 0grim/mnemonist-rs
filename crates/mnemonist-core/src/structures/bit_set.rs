//! Port of upstream `bit-set.js` (mnemonist v0.40.4).
//!
//! A fixed-length bit set over a `Uint32Array`. Everything structural lives in
//! [`crate::structures::bits`], which is shared with
//! [`crate::structures::bit_vector`] because upstream ships the two files with
//! seven methods copy-pasted between them. Read that module first: `size` is an
//! incrementally maintained counter that can go **negative** (BUG-SPARSE-QUEUE-SET-2), `select`
//! loses 32 positions per skipped word (BUG-SPARSE-QUEUE-SET-3), and a cursor keeps the array it
//! was opened over across a `clear` — all three are documented there and all
//! three are upstream's, present twice.
//!
//! What is genuinely `BitSet`'s own is short: a `length` that never changes, a
//! `clear` that reallocates, and `toJSON`.
//!
//! # Out of range is inert here — the `SparseSet` family does not recur
//!
//! Worth stating explicitly, because the two modules are both typed-array
//! backed and the question is the obvious one to ask. `SparseSet.add(m)` past
//! its length corrupts the set three ways (BUG-SPARSE-SET-1/BUG-SPARSE-SET-2/BUG-SPARSE-SET-3). `BitSet` does not:
//!
//! | call, index past the array | upstream | why |
//! |---|---|---|
//! | `set` / `reset` / `flip` | **no-op**, `size` unchanged | the store is dropped, and every comparison against `undefined` is false |
//! | `get` | `0` | `undefined >> pos` is `0` |
//! | `test` | `false` | — |
//!
//! Verified on Node 24.18.1: `new BitSet(10).set(1000)` leaves `size === 0` and
//! the array untouched. The difference is that `SparseSet` **increments a
//! counter unconditionally** after its dropped store, while `BitSet` derives
//! its counter from a before/after comparison that an `undefined` read makes
//! inert.
//!
//! There is one real gap in the same family, and it is narrower. An index in
//! `length .. 32 * ceil(length / 32)` — past the length but inside the last
//! allocated word — is **accepted**: `size` counts the bit, while `rank`,
//! `select` and iteration all stop at `length` and cannot see it. Measured:
//! `new BitSet(10); s.set(20)` gives `size === 1`, `rank(10) === 0` and
//! `select(1) === undefined`. See NOTES.md BUG-UTILS-BITWISE-1.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::structures::bit_set::BitSet;
//!
//! let mut set = BitSet::new(10);
//!
//! set.set(2);
//! set.set(8);
//! set.set(9);
//!
//! assert_eq!(set.size(), 3);
//! assert_eq!(set.to_json(), vec![772]);
//! assert_eq!(set.values().collect::<Vec<_>>(), vec![0, 0, 1, 0, 0, 0, 0, 0, 1, 1]);
//! ```

use crate::structures::bits::{bits_in_word, BitEntries, BitWalk, Words};

/// A fixed-length bit set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitSet {
    words: Words,
}

impl BitSet {
    /// `new BitSet(length)` — `Math.ceil(length / 32)` zeroed words.
    pub fn new(length: usize) -> Self {
        Self {
            words: Words::new(length),
        }
    }

    /// Bits the set covers. Fixed for the set's whole life.
    pub fn length(&self) -> usize {
        self.words.length
    }

    /// Upstream's `size` counter, **not** a population count.
    ///
    /// Signed, because BUG-SPARSE-QUEUE-SET-2 takes it below zero. `rank(length)` is the honest
    /// count; the two disagree exactly when BUG-SPARSE-QUEUE-SET-2 or BUG-UTILS-BITWISE-1 has fired.
    pub fn size(&self) -> i64 {
        self.words.size
    }

    /// The backing `Uint32Array`, exposed because it is a public property
    /// upstream and the differential fuzzer compares it word for word.
    pub fn words(&self) -> &Words {
        &self.words
    }

    /// `clear()` — zero `size` and **allocate a new array**.
    ///
    /// The reallocation is observable: a cursor opened beforehand keeps reading
    /// the old words. Measured on Node. Zeroing in place would be the natural
    /// Rust translation and would be a silent divergence.
    pub fn clear(&mut self) {
        let length = self.words.length;

        self.words.size = 0;
        self.words
            .replace_words(vec![0; crate::structures::bits::words_for(length)]);
    }

    /// `set(index)` — set the bit.
    pub fn set(&mut self, index: i64) {
        self.words.set_bit(index, true);
    }

    /// `set(index, value)` — upstream's `value === 0 || value === false` test.
    pub fn set_to(&mut self, index: i64, value: bool) {
        self.words.set_bit(index, value);
    }

    /// `reset(index)` — clear the bit. Carries BUG-SPARSE-QUEUE-SET-2; see the module docs.
    pub fn reset(&mut self, index: i64) {
        self.words.reset_bit(index);
    }

    /// `flip(index)`.
    pub fn flip(&mut self, index: i64) {
        self.words.flip_bit(index);
    }

    /// `get(index)` — `0` or `1`. Never `undefined`, unlike `BitVector::get`:
    /// `BitSet` has no length guard at all.
    pub fn get(&self, index: i64) -> u32 {
        self.words.get_bit(index)
    }

    /// `test(index)`.
    pub fn test(&self, index: i64) -> bool {
        self.get(index) != 0
    }

    /// `rank(i)` — set bits strictly before `i`.
    pub fn rank(&self, i: i64) -> i64 {
        self.words.rank(i)
    }

    /// `select(r)` — position of the `r`th set bit.
    ///
    /// `Some(-1)` for an empty set or `r >= length`, `None` for upstream's
    /// `undefined` fall-through. Carries BUG-SPARSE-QUEUE-SET-3; see the module docs.
    pub fn select(&self, r: i64) -> Option<i64> {
        self.words.select(r)
    }

    /// `forEach(callback)` — `(bit, index)` per step.
    ///
    /// Written here rather than left to [`values`](BitSet::values) because
    /// upstream's `forEach` and its iterator differ: `forEach` re-reads
    /// `this.array` on every word, so a `clear` from inside the callback is
    /// visible to it, while a cursor's is not.
    pub fn for_each(&self, mut callback: impl FnMut(u32, usize)) {
        let word_count = self.words.word_count();
        let length = self.words.length;

        for index in 0..word_count {
            let word = self.words.word(index).unwrap_or(0);
            let bits = bits_in_word(index, word_count, length);

            for bit in 0..bits {
                callback((((word as i32) >> bit) & 1) as u32, index * 32 + bit);
            }
        }
    }

    /// `values()` — a fresh, non-restartable cursor over the bits.
    pub fn values(&self) -> BitWalk {
        self.words.walk()
    }

    /// `entries()` — the same walk, yielding `(index, bit)`.
    pub fn entries(&self) -> BitEntries {
        BitEntries(self.words.walk())
    }

    /// `toJSON()` — `Array.from(this.array)`, the whole backing array.
    pub fn to_json(&self) -> Vec<u32> {
        self.words.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::Step;

    /// 1:1 port of every upstream `it` block, as a baseline.
    #[test]
    fn reproduces_the_upstream_suite() {
        let set = BitSet::new(74);
        assert_eq!(set.length(), 74);
        assert_eq!(set.words().word_count(), 3);
        assert_eq!(set.size(), 0);

        let mut set = BitSet::new(17);
        set.set(13);
        assert_eq!(set.size(), 1);
        assert_eq!(set.get(13), 1);
        assert!(set.test(13));
        assert_eq!(set.get(2), 0);
        assert!(!set.test(2));
        set.set(2);
        assert_eq!(set.size(), 2);
        set.set_to(2, false);
        assert_eq!(set.size(), 1);
        assert!(!set.test(2));
        set.flip(3);
        assert_eq!(set.size(), 2);
        assert!(set.test(3));
        set.flip(3);
        assert_eq!(set.size(), 1);
        assert!(!set.test(3));

        let mut set = BitSet::new(32);
        set.set(31);
        assert_eq!(set.size(), 1);

        let mut set = BitSet::new(32);
        for i in 0..32 {
            set.set(i);
            assert_eq!(set.size(), i + 1);
        }

        let mut set = BitSet::new(32);
        for i in 0..32 {
            set.flip(i);
            assert_eq!(set.size(), i + 1);
        }

        let mut set = BitSet::new(32);
        set.set(31);
        set.reset(31);
        assert_eq!(set.size(), 0);

        let mut set = BitSet::new(4);
        set.set(0);
        set.set(1);
        set.reset(0);
        set.reset(1);
        assert_eq!(set.get(0), 0);
        assert_eq!(set.get(1), 0);

        // rank, over the deliberately non-32-aligned 8010.
        let mut set = BitSet::new(8010);
        for i in (0..8000).step_by(100) {
            set.set(i);
        }
        for (j, i) in (0..=8000).step_by(2000).enumerate() {
            assert_eq!(set.rank(i), [0, 20, 40, 60, 80][j]);
        }

        let mut set = BitSet::new(2);
        set.set(1);
        assert_eq!(set.rank(0), 0);
        assert_eq!(set.rank(1), 0);
        assert_eq!(set.rank(2), 1);

        // select.
        let mut set = BitSet::new(11);
        for index in [1, 3, 4, 5, 9, 10] {
            set.set(index);
        }
        assert_eq!(set.rank(set.length() as i64), 6);
        for (r, expected) in [(1, 1), (2, 3), (3, 4), (4, 5), (5, 9), (6, 10)] {
            assert_eq!(set.select(r), Some(expected), "select({r})");
        }

        // iteration.
        let mut set = BitSet::new(10);
        set.set(2);
        set.set(8);
        set.set(9);
        let expected = vec![0, 0, 1, 0, 0, 0, 0, 0, 1, 1];
        let mut seen = Vec::new();
        set.for_each(|bit, index| {
            assert_eq!(index, seen.len());
            seen.push(bit);
        });
        assert_eq!(seen, expected);
        assert_eq!(set.values().collect::<Vec<_>>(), expected);
        assert_eq!(
            set.entries().collect::<Vec<_>>(),
            expected
                .iter()
                .enumerate()
                .map(|(i, bit)| (i, *bit))
                .collect::<Vec<_>>()
        );

        // issue #117: a length divisible by 32.
        let set = BitSet::new(64);
        let entries: Vec<(usize, u32)> = set.entries().collect();
        assert_eq!(entries.len(), 64);
        for (index, (i, bit)) in entries.iter().enumerate() {
            assert_eq!((*i, *bit), (index, 0));
        }

        // toJSON.
        let mut set = BitSet::new(10);
        set.set(2);
        set.set(8);
        set.set(9);
        assert_eq!(set.to_json(), vec![772]);
    }

    /// Gap: `size` is a counter, and BUG-SPARSE-QUEUE-SET-2 drives it negative. Upstream's suite
    /// only ever resets a bit that is actually set.
    ///
    /// Measured on Node: `new BitSet(32); set(31); reset(0)` gives `size === 0`
    /// with bit 31 still set.
    #[test]
    fn a_reset_that_clears_nothing_still_decrements_size() {
        let mut set = BitSet::new(32);

        set.set(31);
        set.reset(0);

        assert_eq!(set.size(), 0);
        assert_eq!(set.get(31), 1);
        assert!(set.test(31));

        set.reset(1);
        set.reset(2);
        assert_eq!(set.size(), -2);
    }

    /// The second-order consequence: `rank` early-returns on `size === 0`, so a
    /// set that demonstrably holds a bit reports a rank of zero.
    #[test]
    fn a_corrupted_size_makes_rank_lie() {
        let mut set = BitSet::new(32);

        set.set(31);
        assert_eq!(set.rank(32), 1);

        set.reset(0);

        assert_eq!(set.rank(32), 0);
        assert_eq!(set.get(31), 1);
        // select bails on the same counter.
        assert_eq!(set.select(1), Some(-1));
    }

    /// And the control that shows why upstream never notices: with bit 31 clear
    /// the same call is harmless.
    #[test]
    fn the_same_reset_is_harmless_while_the_words_top_bit_is_clear() {
        let mut set = BitSet::new(32);

        set.set(0);
        set.reset(5);

        assert_eq!(set.size(), 1);
    }

    /// Gap: BUG-SPARSE-QUEUE-SET-3. Upstream's `select` test uses a length of 11, so every bit is
    /// in word 0 and no word is ever skipped.
    #[test]
    fn select_loses_a_word_of_positions_for_every_empty_word_it_skips() {
        let mut set = BitSet::new(64);

        set.set(40);

        // 40 is the true answer.
        assert_eq!(set.select(1), Some(8));

        let mut set = BitSet::new(96);
        set.set(3);
        set.set(70);

        assert_eq!(set.select(1), Some(3));
        assert_eq!(set.select(2), Some(38));
    }

    /// Gap: the two `select` results upstream never produces. `undefined` for
    /// an `r` past the population, and `-1` for `r >= length`.
    #[test]
    fn select_off_the_end_is_undefined_and_out_of_range_is_minus_one() {
        let mut set = BitSet::new(11);

        assert_eq!(set.select(1), Some(-1), "empty set");

        set.set(1);

        assert_eq!(set.select(5), None, "past the population");
        assert_eq!(set.select(11), Some(-1), "r >= length");
        assert_eq!(set.select(0), Some(0), "r == 0 matches before any bit");
    }

    /// Gap: out-of-range indices, and the answer to whether the `SparseSet`
    /// corruption family recurs. It does not.
    ///
    /// Verified on Node: every one of these is inert.
    #[test]
    fn indices_past_the_backing_array_are_inert() {
        let mut set = BitSet::new(10);

        set.set(1000);
        set.reset(1000);
        set.flip(1000);
        set.set(i64::from(u32::MAX));
        set.set(-1);
        set.set(-1000);

        assert_eq!(set.size(), 0);
        assert_eq!(set.to_json(), vec![0]);
        assert_eq!(set.get(1000), 0);
        assert!(!set.test(1000));
    }

    /// But the narrow gap in the same family is real: an index past `length`
    /// yet inside the last allocated word is accepted and then invisible.
    /// BUG-UTILS-BITWISE-1, measured on Node.
    #[test]
    fn a_bit_between_length_and_the_end_of_its_word_is_counted_but_unreachable() {
        let mut set = BitSet::new(10);

        set.set(20);

        assert_eq!(set.size(), 1);
        assert_eq!(set.to_json(), vec![1 << 20]);
        assert_eq!(set.get(20), 1);
        // Everything that respects `length` cannot see it.
        assert_eq!(set.rank(10), 0);
        assert_eq!(set.rank(set.length() as i64), 0);
        assert_eq!(set.select(1), None);
        assert_eq!(set.values().collect::<Vec<_>>(), vec![0; 10]);
    }

    /// Gap: `clear()` allocates a new array, which detaches an open cursor.
    /// Upstream's suite never calls `clear` at all.
    #[test]
    fn clear_detaches_an_open_cursor_from_the_words_it_zeroes() {
        let mut set = BitSet::new(64);

        set.set(0);
        set.set(33);

        let mut cursor = set.values();
        assert_eq!(cursor.step(), Step::Item(1));

        set.clear();

        assert_eq!(set.size(), 0);
        assert_eq!(set.to_json(), vec![0, 0]);
        // The cursor is still on the pre-clear array.
        let rest: Vec<u32> = cursor.collect();
        assert_eq!(rest.len(), 63);
        assert_eq!(rest[32], 1);

        // A cursor opened after the clear sees the new array.
        assert_eq!(set.values().sum::<u32>(), 0);
    }

    /// Gap: `clear` is never called at all upstream, let alone followed by a
    /// re-use.
    #[test]
    fn clear_resets_size_and_the_set_is_reusable() {
        let mut set = BitSet::new(40);

        set.set(3);
        set.set(35);
        assert_eq!(set.size(), 2);

        set.clear();

        assert_eq!(set.size(), 0);
        assert_eq!(set.rank(40), 0);
        assert!(!set.test(3));

        set.set(3);
        assert_eq!(set.size(), 1);
        assert_eq!(set.rank(40), 1);
    }

    /// Gap: `set(index, value)` with a truthy value other than `1`, and the
    /// idempotence of a repeated `set`. Upstream asserts `size` after
    /// `set(2, 0)` and nothing else.
    #[test]
    fn repeated_sets_and_resets_are_idempotent_in_size() {
        let mut set = BitSet::new(64);

        set.set(5);
        set.set(5);
        set.set(5);
        assert_eq!(set.size(), 1);

        set.reset(5);
        assert_eq!(set.size(), 0);
        // The second reset clears nothing AND word 0's top bit is clear, so it
        // is harmless -- the BUG-SPARSE-QUEUE-SET-2 precondition is specifically bit 31.
        set.reset(5);
        assert_eq!(set.size(), 0);

        set.set_to(5, false);
        assert_eq!(set.size(), 0);
    }

    /// Gap: a length of zero. The array is empty, so nothing iterates and every
    /// index is out of range.
    #[test]
    fn a_zero_length_set_holds_and_yields_nothing() {
        let mut set = BitSet::new(0);

        assert_eq!(set.words().word_count(), 0);
        assert_eq!(set.values().count(), 0);
        assert_eq!(set.entries().count(), 0);
        assert_eq!(set.to_json(), Vec::<u32>::new());

        set.set(0);

        assert_eq!(set.size(), 0);
        assert_eq!(set.get(0), 0);
        assert_eq!(set.rank(0), 0);
        assert_eq!(set.select(0), Some(-1));

        let mut seen = 0;
        set.for_each(|_, _| seen += 1);
        assert_eq!(seen, 0);
    }

    /// Gap: iteration over a length that exactly fills its words, and one that
    /// does not, both checked against `length` rather than against the array.
    #[test]
    fn iteration_yields_exactly_length_bits() {
        for length in [1usize, 10, 31, 32, 33, 63, 64, 65, 74] {
            let set = BitSet::new(length);

            assert_eq!(set.values().count(), length, "length {length}");
            assert_eq!(set.entries().count(), length, "length {length}");

            let mut seen = 0;
            set.for_each(|_, _| seen += 1);
            assert_eq!(seen, length, "forEach at length {length}");
        }
    }

    /// Gap: DIV-STACK-1/DIV-STACK-2 on this module. Upstream drains each cursor once, in one
    /// expression, so neither half is observed.
    #[test]
    fn cursors_do_not_restart_but_the_set_can_be_walked_again() {
        let mut set = BitSet::new(4);
        set.set(1);

        let mut cursor = set.values();
        assert_eq!(cursor.by_ref().collect::<Vec<_>>(), vec![0, 1, 0, 0]);
        assert_eq!(cursor.collect::<Vec<_>>(), Vec::<u32>::new());

        assert_eq!(set.values().collect::<Vec<_>>(), vec![0, 1, 0, 0]);
        assert_eq!(set.values().collect::<Vec<_>>(), vec![0, 1, 0, 0]);
    }

    /// Gap: DIV-PROJ-10. A write lands mid-walk, and whether the cursor sees it
    /// depends on which word it is in — because upstream reads a word into a
    /// local once, on entry.
    #[test]
    fn writes_during_iteration_are_visible_only_beyond_the_current_word() {
        let mut set = BitSet::new(64);

        let mut cursor = set.values();
        assert_eq!(cursor.step(), Step::Item(0));

        set.set(5);
        set.set(40);

        let rest: Vec<u32> = cursor.collect();

        assert_eq!(rest[4], 0, "bit 5 is in the already-loaded word 0");
        assert_eq!(rest[39], 1, "bit 40 is in word 1, read on entry");
    }

    /// Gap: `rank` at and past the length, which upstream reaches only at
    /// exactly `length` in the select test.
    #[test]
    fn rank_saturates_past_the_end_rather_than_reading_off_it() {
        let mut set = BitSet::new(40);

        set.set(0);
        set.set(39);

        assert_eq!(set.rank(0), 0);
        assert_eq!(set.rank(1), 1);
        assert_eq!(set.rank(40), 2);
        // Past the array entirely: the missing words popcount as zero.
        assert_eq!(set.rank(1000), 2);
    }

    /// Gap: the width machinery. Upstream only ever allocates 1, 2 or 3 words.
    #[test]
    fn allocates_one_word_per_thirty_two_bits_rounded_up() {
        for (length, words) in [(0usize, 0usize), (1, 1), (32, 1), (33, 2), (64, 2), (74, 3)] {
            assert_eq!(BitSet::new(length).words().word_count(), words, "{length}");
        }
    }
}

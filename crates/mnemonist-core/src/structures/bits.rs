//! The `Uint32Array` word store shared by `bit-set.js` and `bit-vector.js`.
//!
//! Upstream ships the two as separate files with **the same seven methods
//! copy-pasted between them** — `reset`, `flip`, `rank`, `select`, `forEach`,
//! `values`, `entries` are byte-identical, and `set` differs only by a bounds
//! guard. Every defect below is therefore present twice upstream and is written
//! once here. `bit_set` and `bit_vector` each own a [`Words`] and add only what
//! is genuinely theirs: a fixed length and `clear` for one, the whole
//! capacity/policy/push/pop machinery for the other.
//!
//! # `size` is a counter, not a popcount — and it can go negative
//!
//! Neither structure ever counts bits to answer `size`. It is maintained
//! incrementally by comparing the word before and after each write, which works
//! only if both readings are unsigned. `set` and `flip` say so:
//!
//! ```js
//! newBytes = this.array[byteIndex] |= (1 << pos);
//! newBytes = newBytes >>> 0;              // <-- and reset() does NOT do this
//! if (newBytes > oldBytes) this.size++;
//! ```
//!
//! `reset` omits the `>>> 0`. `oldBytes` came out of a `Uint32Array` and is
//! unsigned; `newBytes` is the value of a compound assignment, which is the
//! *signed* `i32` result. So on any word whose bit 31 is set, `newBytes` is
//! negative and `newBytes < oldBytes` is true **whether or not the reset
//! changed anything**. Measured on Node 24.18.1:
//!
//! ```js
//! var s = new BitSet(32);
//! s.set(31);      // size 1
//! s.reset(0);     // bit 0 was already clear
//! s.size          // 0        -- and bit 31 is still set
//! s.rank(32)      // 0        -- because rank early-returns on size === 0
//! ```
//!
//! Three no-op resets take `size` to `-2`. That is NOTES.md B-13, and it is
//! reproduced here, which is why [`Words::size`] is an `i64` rather than a
//! `usize`: a `usize` could not hold the state upstream reaches.
//!
//! # `select` does not advance its position across skipped words
//!
//! ```js
//! for (var i = 0; i < l; i++) {
//!   byte = this.array[i];
//!   if (byte === 0) continue;              // <-- p is not advanced by 32 here
//!   for (var j = 0; j < b; j++, p++) { … }
//! }
//! ```
//!
//! `p` only moves inside the inner loop, so every all-zero word before the
//! answer costs the result 32. Measured: a `BitSet(64)` with only bit 40 set
//! answers `select(1) === 8`. NOTES.md B-14, likewise present in both files.
//!
//! # The last word's width, and why a length of 0 is not empty
//!
//! Both iteration paths compute the last word's bit count as
//! `length % 32 || 32`. The `|| 32` is there for a length that fills its last
//! word exactly — but `0 % 32` is also falsy, so a **length of 0 with a
//! non-empty array yields 32 bits**. Unreachable for `BitSet`, whose array is
//! empty when its length is; reachable for `BitVector`, where capacity outlives
//! length. Measured: `new BitVector(); v.grow();` then `forEach` calls back 32
//! times on a vector of length 0. NOTES.md B-18.
//!
//! # Why the words live behind `Rc<RefCell<…>>`
//!
//! `BitSet.clear()` and `BitVector.reallocate()` both **replace** `this.array`
//! with a new object, and `values()`/`entries()` capture the old one in their
//! closure:
//!
//! ```js
//! BitSet.prototype.values = function () {
//!   var length = this.length, array = this.array, l = array.length, …
//! ```
//!
//! So a cursor opened before a `clear` keeps reading the pre-clear words —
//! measured, and comfortably reachable by the differential fuzzer's grammar.
//! Reading through `&self` in [`crate::cursor::Sequence::slot`] would have
//! shown the *new* array instead. A shared handle reproduces it exactly: the
//! structure replaces its own handle, and the cursor holds the old one, which
//! stays alive and stays live. Writes to a word the cursor has not yet reached
//! are still visible, which is also upstream (the word is read into a local
//! only when the walk enters it — see [`BitWindow`]).
//!
//! `Rc<RefCell<…>>` rather than `Rc<Vec<…>>`: copy-on-write would make element
//! writes *invisible* to an open cursor, which is the opposite divergence.
//! Every borrow here is confined to one method call, so the two can never
//! overlap.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::cursor::{CursorState, Sequence, Step};
use crate::utils::bitwise::{table8_popcount, to_int32};

/// Bits per backing word. `i >> 5` and `i & 0x1f` throughout, as upstream.
pub const WORD_BITS: usize = 32;

/// `[index >> 5, index & 0x0000001f]`, with JavaScript's ToInt32 in front.
///
/// Indices arrive as `i64` rather than `usize` because every upstream use is a
/// bitwise expression, so a negative index yields a **negative word index** and
/// the store or read is simply dropped. Coercing to `usize` at the boundary
/// instead would turn `set(-1)` into `set(4294967295)`, which lands in the same
/// place only by accident and would loop 134 million times inside `rank`.
///
/// Returns `None` when the word index is negative, i.e. where JavaScript reads
/// `undefined` off the front of the array.
fn split(index: i64) -> (Option<usize>, u32) {
    let index = to_int32(index as f64);
    let word = index >> 5;

    (usize::try_from(word).ok(), (index & 0x1f) as u32)
}

/// `Math.ceil(bits / 32)` — the `Uint32Array` length upstream allocates.
pub fn words_for(bits: usize) -> usize {
    bits.div_ceil(WORD_BITS)
}

/// Bits the walk takes from word `word_index`.
///
/// `32` for every word but the last, and `length % 32 || 32` for the last —
/// including the `|| 32` misfire for a length that is a multiple of 32. See the
/// module docs.
pub fn bits_in_word(word_index: usize, word_count: usize, length: usize) -> usize {
    if word_index + 1 != word_count {
        return WORD_BITS;
    }

    match length % WORD_BITS {
        0 => WORD_BITS,
        remainder => remainder,
    }
}

/// Total steps `forEach`/`values`/`entries` take.
pub fn walk_len(word_count: usize, length: usize) -> usize {
    if word_count == 0 {
        return 0;
    }

    (word_count - 1) * WORD_BITS + bits_in_word(word_count - 1, word_count, length)
}

/// The `Uint32Array` plus the `size` counter and the `length` in bits.
#[derive(Debug)]
pub struct Words {
    words: Rc<RefCell<Vec<u32>>>,
    /// Bits the structure claims to hold. Fixed for `BitSet`, mutable for
    /// `BitVector`.
    pub length: usize,
    /// Upstream's `size`. Signed because upstream's own arithmetic takes it
    /// negative; see the module docs.
    pub size: i64,
}

/// Deep copy, not a shared handle. Cloning a structure must not alias its
/// backing store, and must not detach an existing cursor either.
impl Clone for Words {
    fn clone(&self) -> Self {
        Self {
            words: Rc::new(RefCell::new(self.words.borrow().clone())),
            length: self.length,
            size: self.size,
        }
    }
}

impl PartialEq for Words {
    fn eq(&self, other: &Self) -> bool {
        self.length == other.length
            && self.size == other.size
            && *self.words.borrow() == *other.words.borrow()
    }
}

impl Eq for Words {}

impl Words {
    /// `new Uint32Array(Math.ceil(length / 32))`, `size = 0`.
    pub fn new(length: usize) -> Self {
        Self::with_words(length, vec![0; words_for(length)])
    }

    /// The same, over a pre-built word vector — `BitVector`'s capacity is not
    /// derived from its length.
    pub fn with_words(length: usize, words: Vec<u32>) -> Self {
        Self {
            words: Rc::new(RefCell::new(words)),
            length,
            size: 0,
        }
    }

    /// `this.array.length`.
    pub fn word_count(&self) -> usize {
        self.words.borrow().len()
    }

    /// `this.array[index]`, or `None` where JS reads `undefined`.
    pub fn word(&self, index: usize) -> Option<u32> {
        self.words.borrow().get(index).copied()
    }

    /// A copy of the backing array, for `toJSON` and for the fuzzer.
    pub fn to_vec(&self) -> Vec<u32> {
        self.words.borrow().clone()
    }

    /// Replace the backing array, **detaching every open cursor** — which is
    /// what `clear()` and `reallocate()` do upstream by assigning to
    /// `this.array`.
    pub fn replace_words(&mut self, words: Vec<u32>) {
        self.words = Rc::new(RefCell::new(words));
    }

    /// Grow or shrink in place, preserving the leading words.
    ///
    /// Still a *replacement*: both of upstream's branches (`new Uint32Array` +
    /// `set`, and `oldArray.slice`) produce a new object, so an open cursor
    /// detaches either way.
    pub fn resize_words(&mut self, word_count: usize) {
        let mut words = self.to_vec();
        words.resize(word_count, 0);
        self.replace_words(words);
    }

    /// `set(index, value)` — write one bit and maintain `size`.
    ///
    /// Out of range the store is dropped and `size` does not move, because
    /// `oldBytes` is `undefined` and every comparison against it is false.
    pub fn set_bit(&mut self, index: i64, value: bool) {
        let (word_index, pos) = split(index);
        let mut words = self.words.borrow_mut();

        let Some(slot) = word_index.and_then(|index| words.get_mut(index)) else {
            return;
        };

        let old = *slot;
        let bit = 1i32 << pos;
        let updated = if value {
            (old as i32) | bit
        } else {
            (old as i32) & !bit
        };

        *slot = updated as u32;

        // `newBytes >>> 0` before the comparison, so both sides are unsigned.
        let new = updated as u32;

        if new > old {
            self.size += 1;
        } else if new < old {
            self.size -= 1;
        }
    }

    /// `reset(index)` — **including the missing `>>> 0`**. See the module docs.
    pub fn reset_bit(&mut self, index: i64) {
        let (word_index, pos) = split(index);
        let mut words = self.words.borrow_mut();

        let Some(slot) = word_index.and_then(|index| words.get_mut(index)) else {
            return;
        };

        let old = *slot;
        let updated = (old as i32) & !(1i32 << pos);

        *slot = updated as u32;

        // The comparison upstream performs: a SIGNED new value against an
        // UNSIGNED old one, both widened to JavaScript Numbers. This is B-13,
        // and writing `updated as u32` here instead would silently fix it.
        if i64::from(updated) < i64::from(old) {
            self.size -= 1;
        }
    }

    /// `flip(index)` — has the `>>> 0`, so its accounting is correct.
    pub fn flip_bit(&mut self, index: i64) {
        let (word_index, pos) = split(index);
        let mut words = self.words.borrow_mut();

        let Some(slot) = word_index.and_then(|index| words.get_mut(index)) else {
            return;
        };

        let old = *slot;
        let updated = (old as i32) ^ (1i32 << pos);

        *slot = updated as u32;

        let new = updated as u32;

        if new > old {
            self.size += 1;
        } else if new < old {
            self.size -= 1;
        }
    }

    /// `get(index)` — `0` or `1`, and `0` for any index past the array.
    pub fn get_bit(&self, index: i64) -> u32 {
        let (word_index, pos) = split(index);

        // `undefined >> pos` is 0, so an out-of-range read is a clean 0 rather
        // than the corruption `SparseSet` suffers in the same position.
        word_index
            .and_then(|index| self.words.borrow().get(index).copied())
            .map_or(0, |word| (((word as i32) >> pos) & 1) as u32)
    }

    /// `rank(i)` — set bits strictly before index `i`.
    ///
    /// Early-returns `0` when `size` is `0`, using the *counter*, not a real
    /// count — so a `size` corrupted by B-13 makes `rank` answer `0` for a set
    /// that demonstrably holds bits.
    pub fn rank(&self, i: i64) -> i64 {
        if self.size == 0 {
            return 0;
        }

        let (word_index, pos) = split(i);
        let words = self.words.borrow();
        let mut rank = 0i64;

        // A negative word index makes upstream's `for (j = 0; j < byteIndex; j++)`
        // run zero times, so nothing is summed.
        for index in 0..word_index.unwrap_or(0) {
            // Past the end this reads `undefined`, whose popcount is 0.
            let word = words.get(index).copied().unwrap_or(0);
            rank += i64::from(table8_popcount(f64::from(word)));
        }

        // `(1 << pos) - 1`, which for pos == 31 is ToInt32(-2147483649) ==
        // 0x7fffffff -- so the mask is right even there.
        let mask = ((1u64 << pos) - 1) as u32;
        let masked = word_index
            .and_then(|index| words.get(index).copied())
            .unwrap_or(0)
            & mask;

        rank + i64::from(table8_popcount(f64::from(masked)))
    }

    /// `select(r)` — the position of the `r`th set bit.
    ///
    /// Three outcomes, all upstream's: `Some(-1)` for an empty set or an `r`
    /// past `length`, `Some(position)` on a hit, and **`None` — `undefined` —
    /// when the scan runs off the end**, which upstream reaches by falling out
    /// of the loop with no `return`.
    ///
    /// Reproduces B-14: `p` is not advanced across the words `byte === 0`
    /// skips.
    pub fn select(&self, r: i64) -> Option<i64> {
        if self.size == 0 {
            return Some(-1);
        }

        if r >= self.length as i64 {
            return Some(-1);
        }

        let words = self.words.borrow();
        let word_count = words.len();
        // `b` is hoisted out of the loop upstream and only ever assigned on the
        // last word, so it stays 32 until then -- and stays reassigned after.
        let mut b = WORD_BITS;
        let mut position = 0i64;
        // `c` counts up from 0 and is compared with `===` against `r`, so a
        // negative `r` simply never matches and the scan falls off the end.
        let mut count = 0i64;

        for (index, word) in words.iter().enumerate() {
            if *word == 0 {
                // `continue` WITHOUT advancing `position`. The defect.
                continue;
            }

            if index + 1 == word_count {
                b = bits_in_word(index, word_count, self.length);
            }

            for bit in 0..b {
                count += i64::from(((*word as i32) >> bit) & 1);

                if count == r {
                    return Some(position);
                }

                position += 1;
            }
        }

        None
    }

    /// Steps a full walk over this store takes.
    pub fn walk_len(&self) -> usize {
        walk_len(self.word_count(), self.length)
    }

    /// Open a cursor, capturing the array *identity*, the length and the word
    /// count — exactly the four locals upstream's closure captures.
    pub fn walk(&self) -> BitWalk {
        BitWalk::open(BitWindow {
            words: Rc::clone(&self.words),
            length: self.length,
            word_count: self.words.borrow().len(),
            loaded: Cell::new(None),
        })
    }
}

/// The closure state of `BitSet.prototype.values`, as a [`Sequence`].
///
/// Self-contained on purpose: upstream's closure never touches `this` again
/// after construction, so this borrows nothing from the structure and a cursor
/// can outlive any borrow of it without the [`CursorState`] gymnastics
/// `SparseSet` needs.
pub struct BitWindow {
    words: Rc<RefCell<Vec<u32>>>,
    length: usize,
    word_count: usize,
    /// `byte`, the word upstream reads into a local **once per word** rather
    /// than once per bit. Without this a write landing mid-word would be
    /// visible to the current word's remaining bits, and upstream's would not.
    loaded: Cell<Option<(usize, u32)>>,
}

impl Sequence for BitWindow {
    type Item = u32;
    type Frozen = ();

    fn freeze(&self) -> ((), usize) {
        ((), walk_len(self.word_count, self.length))
    }

    fn slot(&self, _frozen: &(), ordinal: usize) -> Option<u32> {
        // Ordinals are the walk's own counter, always non-negative and bounded
        // by the frozen length, so `split`'s ToInt32 has nothing to do here.
        let (word_index, pos) = (ordinal >> 5, (ordinal & 0x1f) as u32);

        let word = match self.loaded.get() {
            Some((cached, word)) if cached == word_index => word,
            _ => {
                // `byte = array[i++]` -- the live read, once, on entry.
                let word = self.words.borrow().get(word_index).copied().unwrap_or(0);
                self.loaded.set(Some((word_index, word)));
                word
            }
        };

        Some((((word as i32) >> pos) & 1) as u32)
    }
}

/// A non-restartable walk over a bit store, yielding one bit per step.
///
/// [`Step::Gap`] is unreachable here, unlike `SparseSet`: the frozen array is
/// held alive by the cursor's own handle and no upstream method resizes a word
/// vector in place, so every ordinal below the frozen length has a word behind
/// it. Growth and `clear` both *replace* the array, which the cursor does not
/// follow.
pub struct BitWalk {
    window: BitWindow,
    state: CursorState<BitWindow>,
}

impl BitWalk {
    fn open(window: BitWindow) -> Self {
        let state = CursorState::open(&window);

        Self { window, state }
    }

    /// The next bit, faithfully.
    pub fn step(&mut self) -> Step<u32> {
        self.state.step(&self.window)
    }

    /// The next `[index, bit]` pair, which is what `entries()` yields.
    ///
    /// The index is the step ordinal. Upstream computes `(~-i) * 32 + j` — `i`
    /// having already been incremented past the current word — which is the
    /// same number by a more scenic route.
    pub fn step_entry(&mut self) -> Step<(usize, u32)> {
        let index = self.state.position();

        match self.state.step(&self.window) {
            Step::Item(bit) => Step::Item((index, bit)),
            Step::Gap => Step::Gap,
            Step::Done => Step::Done,
        }
    }

    /// Steps captured at construction.
    pub fn frozen_len(&self) -> usize {
        self.state.frozen_len()
    }

    pub fn position(&self) -> usize {
        self.state.position()
    }
}

impl Iterator for BitWalk {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        self.step().item()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.state.remaining()))
    }
}

/// `entries()`: the same walk, pairing each bit with its index.
pub struct BitEntries(pub BitWalk);

impl Iterator for BitEntries {
    type Item = (usize, u32);

    fn next(&mut self) -> Option<(usize, u32)> {
        self.0.step_entry().item()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_word_is_full_when_the_length_is_a_multiple_of_thirty_two() {
        // `length % 32 || 32`: the `|| 32` is meant for an exactly-filled last
        // word and also fires for zero.
        assert_eq!(bits_in_word(0, 1, 32), 32);
        assert_eq!(bits_in_word(0, 1, 0), 32);
        assert_eq!(bits_in_word(0, 1, 10), 10);
        assert_eq!(bits_in_word(0, 2, 33), 32);
        assert_eq!(bits_in_word(1, 2, 33), 1);

        assert_eq!(walk_len(0, 0), 0);
        assert_eq!(walk_len(1, 0), 32);
        assert_eq!(walk_len(1, 10), 10);
        assert_eq!(walk_len(2, 64), 64);
        assert_eq!(walk_len(2, 33), 33);
    }

    #[test]
    fn words_for_rounds_up() {
        assert_eq!(words_for(0), 0);
        assert_eq!(words_for(1), 1);
        assert_eq!(words_for(32), 1);
        assert_eq!(words_for(33), 2);
        assert_eq!(words_for(74), 3);
    }

    /// B-13, in isolation: a reset that clears nothing still decrements `size`
    /// whenever bit 31 of the word is set.
    #[test]
    fn a_no_op_reset_decrements_size_when_the_top_bit_of_the_word_is_set() {
        let mut words = Words::new(32);

        words.set_bit(31, true);
        assert_eq!(words.size, 1);

        words.reset_bit(0);
        assert_eq!(words.size, 0);
        assert_eq!(words.get_bit(31), 1);

        // And it keeps going, straight past zero.
        words.reset_bit(1);
        words.reset_bit(2);
        assert_eq!(words.size, -2);
    }

    /// The control: with bit 31 clear the same no-op reset is harmless, which
    /// is why nothing upstream notices.
    #[test]
    fn a_no_op_reset_is_harmless_while_the_top_bit_is_clear() {
        let mut words = Words::new(32);

        words.set_bit(0, true);
        words.reset_bit(1);

        assert_eq!(words.size, 1);
    }

    /// B-14: every all-zero word skipped before the answer costs 32.
    #[test]
    fn select_loses_thirty_two_positions_per_skipped_word() {
        let mut words = Words::new(64);

        words.set_bit(40, true);

        // The true answer is 40.
        assert_eq!(words.select(1), Some(8));

        let mut words = Words::new(96);
        words.set_bit(3, true);
        words.set_bit(70, true);

        // The first is right because nothing was skipped before it.
        assert_eq!(words.select(1), Some(3));
        // The second loses the one empty word between them.
        assert_eq!(words.select(2), Some(38));
    }

    /// The three shapes `select` returns, which upstream's test reaches only
    /// the middle of.
    #[test]
    fn select_answers_minus_one_a_position_or_undefined() {
        let empty = Words::new(11);
        assert_eq!(empty.select(1), Some(-1));

        let mut words = Words::new(11);
        words.set_bit(1, true);

        // r past the length.
        assert_eq!(words.select(11), Some(-1));
        // A real hit.
        assert_eq!(words.select(1), Some(1));
        // Past the population: falls out of the loop with no `return`.
        assert_eq!(words.select(5), None);
        // r == 0 matches before any bit is counted, so it answers the position
        // of the first ZERO bit in the first non-empty word.
        assert_eq!(words.select(0), Some(0));
    }

    /// `rank` trusts the `size` counter, so B-13 propagates into it.
    #[test]
    fn rank_returns_zero_whenever_the_size_counter_is_zero() {
        let mut words = Words::new(32);

        words.set_bit(31, true);
        assert_eq!(words.rank(32), 1);

        words.reset_bit(0);

        assert_eq!(words.size, 0);
        assert_eq!(words.get_bit(31), 1);
        // A bit is set and rank says there are none.
        assert_eq!(words.rank(32), 0);
    }

    /// Out-of-range reads and writes are inert here, which is the contrast with
    /// `SparseSet`'s B-8/B-9/B-10 family.
    #[test]
    fn out_of_range_indices_are_inert_rather_than_corrupting() {
        let mut words = Words::new(10);

        words.set_bit(1000, true);
        assert_eq!(words.size, 0);
        assert_eq!(words.to_vec(), vec![0]);
        assert_eq!(words.get_bit(1000), 0);

        words.reset_bit(1000);
        words.flip_bit(1000);
        assert_eq!(words.size, 0);
        assert_eq!(words.to_vec(), vec![0]);
    }

    /// But an index inside the last *word* and past `length` is accepted, and
    /// then `size` disagrees with everything else. B-19.
    #[test]
    fn a_bit_past_length_but_inside_the_word_is_counted_yet_invisible() {
        let mut words = Words::new(10);

        words.set_bit(20, true);

        assert_eq!(words.size, 1);
        assert_eq!(words.to_vec(), vec![1 << 20]);
        // rank only ever looks at the first `length` bits.
        assert_eq!(words.rank(10), 0);
        // The walk only yields `length` bits.
        assert_eq!(words.walk().count(), 10);
        assert_eq!(words.walk().sum::<u32>(), 0);
        // And select cannot find it, so it runs off the end.
        assert_eq!(words.select(1), None);
    }

    /// Replacing the array detaches an open cursor, which is what `clear` and
    /// `reallocate` do upstream. Measured on Node: a cursor opened before
    /// `clear()` still yields the pre-clear bits.
    #[test]
    fn a_cursor_keeps_the_array_it_was_opened_over() {
        let mut words = Words::new(64);

        words.set_bit(0, true);
        words.set_bit(33, true);

        let mut walk = words.walk();
        assert_eq!(walk.step(), Step::Item(1));

        words.replace_words(vec![0; 2]);
        words.size = 0;

        // The cursor is still walking the old array.
        let rest: Vec<u32> = walk.collect();
        assert_eq!(rest.len(), 63);
        assert_eq!(rest.iter().sum::<u32>(), 1);
        // Index 33 is ordinal 33, i.e. rest[32].
        assert_eq!(rest[32], 1);
    }

    /// Writes to a word the cursor has not yet entered ARE visible, because
    /// upstream reads the word into a local only on entry.
    #[test]
    fn writes_ahead_of_the_cursor_are_visible_but_not_within_the_current_word() {
        let mut words = Words::new(64);

        let mut walk = words.walk();
        assert_eq!(walk.step(), Step::Item(0));

        // Bit 5 is in the word the walk has already loaded: invisible.
        words.set_bit(5, true);
        // Bit 40 is in the next word: visible.
        words.set_bit(40, true);

        let rest: Vec<u32> = walk.collect();
        assert_eq!(rest[4], 0, "the current word was captured on entry");
        assert_eq!(rest[39], 1, "the next word is read live");
    }

    /// D-06 on this module: the walk is not restartable.
    #[test]
    fn a_walk_is_not_restartable() {
        let mut words = Words::new(4);
        words.set_bit(2, true);

        let mut walk = words.walk();
        assert_eq!(walk.by_ref().collect::<Vec<_>>(), vec![0, 0, 1, 0]);
        assert_eq!(walk.collect::<Vec<_>>(), Vec::<u32>::new());

        // But the structure can be walked again.
        assert_eq!(words.walk().collect::<Vec<_>>(), vec![0, 0, 1, 0]);
    }

    #[test]
    fn entries_pair_each_bit_with_its_ordinal() {
        let mut words = Words::new(3);
        words.set_bit(1, true);

        let entries: Vec<(usize, u32)> = BitEntries(words.walk()).collect();

        assert_eq!(entries, vec![(0, 0), (1, 1), (2, 0)]);
    }

    /// Cloning must not alias the backing store, or a clone would follow the
    /// original's writes.
    #[test]
    fn cloning_copies_the_backing_store() {
        let mut words = Words::new(32);
        words.set_bit(1, true);

        let copy = words.clone();
        words.set_bit(2, true);

        assert_eq!(copy.get_bit(2), 0);
        assert_eq!(words.get_bit(2), 1);
        assert_ne!(words, copy);
    }
}

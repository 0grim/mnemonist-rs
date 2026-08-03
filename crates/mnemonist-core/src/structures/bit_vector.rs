//! Port of upstream `bit-vector.js` (mnemonist v0.40.4).
//!
//! A growable bit set. The bit-level half is [`crate::structures::bits`], shared
//! with [`crate::structures::bit_set`] because upstream copy-pastes seven
//! methods between the two files — **including all of BUG-SPARSE-QUEUE-SET-2 and BUG-SPARSE-QUEUE-SET-3**, which are
//! documented there and which this module inherits unchanged. What is added
//! here is the capacity machinery: a user-supplied growth policy, `push`/`pop`,
//! `grow`, `resize` and `reallocate`.
//!
//! # `push`/`pop` do not maintain the bits they claim to
//!
//! Three defects, all verified against Node 24.18.1 and all in six lines:
//!
//! ```js
//! BitVector.prototype.push = function (value) {
//!   if (this.capacity === this.length) this.grow();
//!   if (value === 0 || value === false) return ++this.length;   // (1)
//!   this.size++;                                                // (2)
//!   var index = this.length++, …
//!   this.array[byteIndex] |= (1 << pos);
//! };
//! BitVector.prototype.pop = function () {
//!   if (this.length === 0) return;
//!   var index = --this.length;                                  // (3)
//!   return (this.array[byteIndex] >> pos) & 1;
//! };
//! ```
//!
//! 1. **`push(0)` never clears the slot.** It only bumps `length`. Over a
//!    region a `pop` has released, the stale `1` is still there.
//! 2. **`push(1)` increments `size` unconditionally**, even onto a slot that
//!    already holds a `1`.
//! 3. **`pop` never decrements `size`** and never clears the bit.
//!
//! So `size` drifts from the true population as soon as anything is popped.
//! Measured, and *nearly* caught by upstream's own `pop` test, which stops one
//! assertion short:
//!
//! ```js
//! var v = new BitVector();
//! v.push(1); v.push(1);      // size 2, array [0b11]
//! v.pop(); v.pop();          // length 0 -- size STILL 2, bits STILL set
//! v.push(0);                 // length 1, and bit 0 is still 1
//! v.get(0)                   // 1, not 0
//! v.push(1);                 // size 3, with two bits set
//! ```
//!
//! The upstream test asserts `v.get(1) === 1` at that point and never asks
//! about `v.get(0)`.
//!
//! # The constructor prefers `initialLength`, then `initialCapacity`
//!
//! ```js
//! initialLength = opts.initialLength || opts.initialCapacity || 0;
//! ```
//!
//! So `{initialCapacity: 30}` sets the **length** to 30, not the capacity — the
//! capacity is then derived from it. Upstream's own "custom policy" test relies
//! on this without saying so.
//!
//! # A length of 0 still iterates 32 bits
//!
//! `length % 32 || 32` (see [`crate::structures::bits`]) treats a length that is
//! any multiple of 32 as a full final word, zero included. `BitSet` cannot reach
//! it — its array is empty when its length is — but a `BitVector` whose capacity
//! outlives its length can: `new BitVector(); v.grow();` then `forEach` calls
//! back **32 times** on a vector of length 0. NOTES.md BUG-BIT-SET-2.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::structures::bit_vector::BitVector;
//!
//! let mut vector = BitVector::new(0);
//!
//! for value in 0..250 {
//!     vector.push(value % 2 == 1).unwrap();
//! }
//!
//! assert_eq!(vector.length(), 250);
//! assert_eq!(vector.capacity(), 256);
//! assert_eq!(vector.get(34), Some(0));
//! assert_eq!(vector.get(35), Some(1));
//! ```

use core::fmt;

use crate::structures::bits::{bits_in_word, words_for, BitEntries, BitWalk, Words, WORD_BITS};

/// `DEFAULT_GROWING_POLICY`: `Math.max(1, Math.ceil(capacity * 1.5))`.
pub fn default_policy(capacity: f64) -> Option<f64> {
    Some((capacity * 1.5).ceil().max(1.0))
}

/// A growth policy: capacity in, requested capacity out.
///
/// `None` stands for upstream's `typeof newCapacity !== 'number'` — a policy
/// that returned something that is not a number at all. A JS policy can do
/// that, and upstream checks for it, so the type has to be able to say it.
pub type Policy = Box<dyn Fn(f64) -> Option<f64>>;

/// Upstream's two `applyPolicy` throws, plus one refusal of our own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `policy returned an invalid value (expecting a positive integer).`
    PolicyInvalidValue,
    /// `policy returned a less or equal capacity to allocate.`
    PolicyTooSmall,
    /// `BitVector.set: index out of bounds.`
    IndexOutOfBounds,
    /// A policy result upstream propagates as `NaN`/`Infinity` into an
    /// allocation size. Not an upstream message; see the divergence doc.
    PolicyNotRepresentable,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PolicyInvalidValue => formatter.write_str(
                "mnemonist/bit-vector.applyPolicy: policy returned an invalid value \
                 (expecting a positive integer).",
            ),
            Self::PolicyTooSmall => formatter.write_str(
                "mnemonist/bit-vector.applyPolicy: policy returned a less or equal \
                 capacity to allocate.",
            ),
            Self::IndexOutOfBounds => formatter.write_str("BitVector.set: index out of bounds."),
            Self::PolicyNotRepresentable => formatter.write_str(
                "mnemonist-rs/bit-vector.applyPolicy: policy returned a value that is \
                 not a finite capacity.",
            ),
        }
    }
}

/// A growable bit vector.
pub struct BitVector {
    words: Words,
    capacity: usize,
    policy: Policy,
}

impl fmt::Debug for BitVector {
    /// Hand-written: a `Box<dyn Fn>` has no `Debug`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BitVector")
            .field("length", &self.words.length)
            .field("size", &self.words.size)
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl BitVector {
    /// `new BitVector(initialLength)` with the default growth policy.
    pub fn new(initial_length: usize) -> Self {
        Self::with_policy(initial_length, Box::new(default_policy))
    }

    /// `new BitVector({initialLength, policy})`.
    ///
    /// Note the argument is a *length*: upstream's
    /// `opts.initialLength || opts.initialCapacity || 0` means an
    /// `initialCapacity` is used as the length too. The bridge resolves that
    /// union; by the time it reaches here there is only a length.
    pub fn with_policy(initial_length: usize, policy: Policy) -> Self {
        // `capacity = Math.ceil(length / 32) * 32`, then
        // `array = new Uint32Array(Math.ceil(capacity / 32))`.
        let capacity = words_for(initial_length) * WORD_BITS;

        Self {
            words: Words::with_words(initial_length, vec![0; words_for(capacity)]),
            capacity,
            policy,
        }
    }

    /// Upstream's `length` — the number of bits currently considered part of
    /// the vector, which is distinct from [`capacity`](BitVector::capacity):
    /// bits between the two exist in the backing words and can be written, but
    /// are not counted.
    pub fn length(&self) -> usize {
        self.words.length
    }

    /// Upstream's `size` counter. Signed, and unreliable after any `pop`; see
    /// the module docs and BUG-SPARSE-QUEUE-SET-2.
    pub fn size(&self) -> i64 {
        self.words.size
    }

    /// Upstream's `capacity` — the number of bits the backing words can hold
    /// before a reallocation is required.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The backing `Uint32Array`, compared word for word by the fuzzer.
    pub fn words(&self) -> &Words {
        &self.words
    }

    /// `set(index, value)`.
    ///
    /// # Errors
    ///
    /// [`Error::IndexOutOfBounds`] for `index > length` — note the strict
    /// comparison, which admits `index == length` and writes into the capacity
    /// region without moving `length`, exactly as `HashedArrayTree` does.
    pub fn set(&mut self, index: i64, value: bool) -> Result<(), Error> {
        // `if (this.length < index) throw`.
        if (self.words.length as i64) < index {
            return Err(Error::IndexOutOfBounds);
        }

        self.words.set_bit(index, value);

        Ok(())
    }

    /// `reset(index)` — no bounds check at all upstream. Carries BUG-SPARSE-QUEUE-SET-2.
    pub fn reset(&mut self, index: i64) {
        self.words.reset_bit(index);
    }

    /// `flip(index)` — likewise unguarded.
    pub fn flip(&mut self, index: i64) {
        self.words.flip_bit(index);
    }

    /// `get(index)` — `None` where upstream returns `undefined`.
    ///
    /// The guard is `this.length < index`, so `get(length)` reads the capacity
    /// region rather than reporting absence.
    pub fn get(&self, index: i64) -> Option<u32> {
        if (self.words.length as i64) < index {
            return None;
        }

        Some(self.words.get_bit(index))
    }

    /// `test(index)` — `false` past `length`, else the bit.
    pub fn test(&self, index: i64) -> bool {
        self.get(index).is_some_and(|bit| bit != 0)
    }

    /// `rank(i)` — the number of set bits strictly before index `i`.
    pub fn rank(&self, i: i64) -> i64 {
        self.words.rank(i)
    }

    /// `select(r)`. Carries BUG-SPARSE-QUEUE-SET-3; see [`crate::structures::bits`].
    pub fn select(&self, r: i64) -> Option<i64> {
        self.words.select(r)
    }

    /// `applyPolicy(override)` — the next capacity, rounded up to a word.
    ///
    /// `override || this.capacity`: a `0` override is falsy and falls back to
    /// the current capacity, which is upstream and is why `grow()` works on a
    /// zero-capacity vector at all.
    ///
    /// The `<=` check compares against `this.capacity`, **not** against the
    /// override — so inside [`grow`](BitVector::grow)'s loop only the first
    /// iteration can throw.
    pub fn apply_policy(&self, override_capacity: Option<usize>) -> Result<usize, Error> {
        let input = match override_capacity {
            Some(capacity) if capacity != 0 => capacity,
            _ => self.capacity,
        };

        let Some(requested) = (self.policy)(input as f64) else {
            return Err(Error::PolicyInvalidValue);
        };

        // `typeof newCapacity !== 'number' || newCapacity < 0`. NaN passes this
        // upstream, because every NaN comparison is false -- and then propagates
        // into `new Uint32Array(NaN)`. Refused here; see the divergence doc.
        if requested < 0.0 {
            return Err(Error::PolicyInvalidValue);
        }

        if !requested.is_finite() {
            return Err(Error::PolicyNotRepresentable);
        }

        if requested <= self.capacity as f64 {
            return Err(Error::PolicyTooSmall);
        }

        // `Math.ceil(newCapacity / 32) * 32` -- a non-integer result is
        // accepted upstream and rounded here, which is why the policy returns
        // an `f64` rather than a `usize`.
        Ok((requested / WORD_BITS as f64).ceil() as usize * WORD_BITS)
    }

    /// `reallocate(capacity)` — resize the backing array to fit `capacity`.
    ///
    /// Three subtleties, all upstream's:
    ///
    /// * `length` is clamped to the **unrounded** capacity, before anything
    ///   else happens and before either early return.
    /// * the first early return fires when the *rounded* capacity is unchanged,
    ///   so a `length` clamp can land with `capacity` untouched.
    /// * both branches allocate a **new** array, so an open cursor detaches
    ///   either way.
    pub fn reallocate(&mut self, capacity: usize) {
        let virtual_capacity = capacity;
        let capacity = words_for(capacity) * WORD_BITS;

        if virtual_capacity < self.words.length {
            self.words.length = virtual_capacity;
        }

        if capacity == self.capacity {
            return;
        }

        let storage_length = capacity / WORD_BITS;

        if storage_length == self.words.word_count() {
            return;
        }

        self.words.resize_words(storage_length);
        self.capacity = capacity;
    }

    /// `grow(capacity)` — reach at least `capacity`, applying the policy as
    /// many times as it takes. `None` applies it exactly once.
    pub fn grow(&mut self, capacity: Option<usize>) -> Result<(), Error> {
        let Some(target) = capacity else {
            let new_capacity = self.apply_policy(None)?;
            self.reallocate(new_capacity);

            return Ok(());
        };

        if self.capacity >= target {
            return Ok(());
        }

        let mut new_capacity = self.capacity;

        while new_capacity < target {
            new_capacity = self.apply_policy(Some(new_capacity))?;
        }

        self.reallocate(new_capacity);

        Ok(())
    }

    /// `resize(length)` — set the length, reallocating upward if needed.
    /// Shrinking never deallocates.
    pub fn resize(&mut self, length: usize) {
        if length == self.words.length {
            return;
        }

        if length < self.words.length {
            self.words.length = length;
            return;
        }

        self.words.length = length;
        self.reallocate(length);
    }

    /// `push(value)` — returns the new length.
    ///
    /// Carries all three `push`/`pop` defects; see the module docs. In
    /// particular pushing `false` does **not** clear the slot.
    ///
    /// # Errors
    ///
    /// Propagates whatever the growth policy raises when the vector is full.
    pub fn push(&mut self, value: bool) -> Result<usize, Error> {
        if self.capacity == self.words.length {
            self.grow(None)?;
        }

        if !value {
            // `return ++this.length` -- no store, no size change, and no
            // clearing of whatever was in the slot.
            self.words.length += 1;

            return Ok(self.words.length);
        }

        // `this.size++` happens before the store and is unconditional, so
        // pushing 1 onto a slot that already holds 1 counts it twice.
        self.words.size += 1;

        let index = self.words.length;
        self.words.length += 1;

        // `this.array[byteIndex] |= (1 << pos)`, deliberately NOT the
        // size-maintaining `set_bit`.
        self.or_bit(index);

        Ok(self.words.length)
    }

    /// `pop()` — the last bit, or `None` on an empty vector.
    ///
    /// Does not clear the bit and does not decrement `size`. Both are
    /// upstream's; see the module docs.
    pub fn pop(&mut self) -> Option<u32> {
        if self.words.length == 0 {
            return None;
        }

        self.words.length -= 1;

        Some(self.words.get_bit(self.words.length as i64))
    }

    /// `forEach(callback)` — `(bit, index)`, over `length` bits.
    pub fn for_each(&self, mut callback: impl FnMut(u32, usize)) {
        let word_count = self.words.word_count();
        let length = self.words.length;

        for index in 0..word_count {
            let word = self.words.word(index).unwrap_or(0);

            for bit in 0..bits_in_word(index, word_count, length) {
                callback((((word as i32) >> bit) & 1) as u32, index * WORD_BITS + bit);
            }
        }
    }

    /// `values()` — a fresh, non-restartable cursor.
    pub fn values(&self) -> BitWalk {
        self.words.walk()
    }

    /// `entries()` — the same walk, yielding `(index, bit)`.
    pub fn entries(&self) -> BitEntries {
        BitEntries(self.words.walk())
    }

    /// `toJSON()` — `Array.from(this.array.slice(0, (this.length >> 5) + 1))`.
    ///
    /// One word *more* than the length strictly needs, clamped by the array's
    /// own length. So a length of 64 over a two-word array yields two words,
    /// and a length of 10 yields one.
    pub fn to_json(&self) -> Vec<u32> {
        let end = ((self.words.length >> 5) + 1).min(self.words.word_count());

        self.words.to_vec()[..end].to_vec()
    }

    /// `this.array[byteIndex] |= (1 << pos)` with no size accounting, which is
    /// what `push` does directly rather than going through `set`.
    fn or_bit(&mut self, index: usize) {
        let before = self.words.size;

        self.words.set_bit(index as i64, true);
        // `set_bit` maintains `size`; `push` has already done so itself, and
        // unconditionally. Undoing it here keeps the one place that knows the
        // word layout without duplicating it.
        self.words.size = before;
    }
}

impl Clone for BitVector {
    /// Clones the bits and the capacity, and resets the policy to the default.
    ///
    /// A `Box<dyn Fn>` cannot be cloned. Nothing upstream clones a vector, and
    /// silently sharing a policy would be worse than the documented reset.
    fn clone(&self) -> Self {
        Self {
            words: self.words.clone(),
            capacity: self.capacity,
            policy: Box::new(default_policy),
        }
    }
}

impl PartialEq for BitVector {
    /// Bits, length, size and capacity. Policies are not comparable.
    fn eq(&self, other: &Self) -> bool {
        self.words == other.words && self.capacity == other.capacity
    }
}

impl Eq for BitVector {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::Step;

    fn with(initial_length: usize, policy: impl Fn(f64) -> Option<f64> + 'static) -> BitVector {
        BitVector::with_policy(initial_length, Box::new(policy))
    }

    /// 1:1 port of every upstream `it` block, as a baseline.
    #[test]
    fn reproduces_the_upstream_suite() {
        let vector = BitVector::new(74);
        assert_eq!(vector.length(), 74);
        assert_eq!(vector.capacity(), 96);
        assert_eq!(vector.words().word_count(), 3);
        assert_eq!(vector.size(), 0);

        let mut vector = BitVector::new(17);
        vector.set(13, true).unwrap();
        assert_eq!(vector.size(), 1);
        assert_eq!(vector.get(13), Some(1));
        assert!(vector.test(13));
        assert_eq!(vector.get(2), Some(0));
        assert!(!vector.test(2));
        vector.set(2, true).unwrap();
        assert_eq!(vector.size(), 2);
        vector.set(2, false).unwrap();
        assert_eq!(vector.size(), 1);
        assert!(!vector.test(2));
        vector.flip(3);
        assert_eq!(vector.size(), 2);
        assert!(vector.test(3));
        vector.flip(3);
        assert_eq!(vector.size(), 1);
        assert!(!vector.test(3));

        let mut vector = BitVector::new(32);
        vector.set(31, true).unwrap();
        assert_eq!(vector.size(), 1);

        let mut vector = BitVector::new(32);
        for i in 0..32 {
            vector.set(i, true).unwrap();
            assert_eq!(vector.size(), i + 1);
        }

        let mut vector = BitVector::new(32);
        for i in 0..32 {
            vector.flip(i);
            assert_eq!(vector.size(), i + 1);
        }

        let mut vector = BitVector::new(32);
        vector.set(31, true).unwrap();
        vector.reset(31);
        assert_eq!(vector.size(), 0);

        let mut vector = BitVector::new(4);
        vector.set(0, true).unwrap();
        vector.set(1, true).unwrap();
        vector.reset(0);
        vector.reset(1);
        assert_eq!(vector.get(0), Some(0));
        assert_eq!(vector.get(1), Some(0));

        let mut vector = BitVector::new(8010);
        for i in (0..8000).step_by(100) {
            vector.set(i, true).unwrap();
        }
        for (j, i) in (0..=8000).step_by(2000).enumerate() {
            assert_eq!(vector.rank(i), [0, 20, 40, 60, 80][j]);
        }

        let mut vector = BitVector::new(2);
        vector.set(1, true).unwrap();
        assert_eq!(vector.rank(0), 0);
        assert_eq!(vector.rank(1), 0);
        assert_eq!(vector.rank(2), 1);

        let mut vector = BitVector::new(11);
        for index in [1, 3, 4, 5, 9, 10] {
            vector.set(index, true).unwrap();
        }
        assert_eq!(vector.rank(vector.length() as i64), 6);
        for (r, expected) in [(1, 1), (2, 3), (3, 4), (4, 5), (5, 9), (6, 10)] {
            assert_eq!(vector.select(r), Some(expected), "select({r})");
        }

        let mut vector = BitVector::new(10);
        for index in [2, 8, 9] {
            vector.set(index, true).unwrap();
        }
        let expected = vec![0, 0, 1, 0, 0, 0, 0, 0, 1, 1];
        let mut seen = Vec::new();
        vector.for_each(|bit, index| {
            assert_eq!(index, seen.len());
            seen.push(bit);
        });
        assert_eq!(seen, expected);
        assert_eq!(vector.values().collect::<Vec<_>>(), expected);

        // issue #117.
        let vector = BitVector::new(64);
        assert_eq!(vector.entries().count(), 64);

        // out-of-bound values.
        let vector = BitVector::new(5);
        assert_eq!(vector.get(17), None);
        assert!(!vector.test(17));

        let mut vector = BitVector::new(0);
        assert_eq!(vector.set(3, true).unwrap_err(), Error::IndexOutOfBounds);

        // push.
        let mut vector = BitVector::new(0);
        for i in 0..250 {
            vector.push(i % 2 == 1).unwrap();
        }
        assert_eq!(vector.length(), 250);
        assert_eq!(vector.capacity(), 256);
        assert_eq!(vector.words().word_count(), 8);
        assert_eq!(vector.get(34), Some(0));
        assert_eq!(vector.get(35), Some(1));

        // pop.
        let mut vector = BitVector::new(0);
        vector.push(true).unwrap();
        vector.push(true).unwrap();
        assert_eq!(vector.pop(), Some(1));
        assert_eq!(vector.length(), 1);
        assert_eq!(vector.pop(), Some(1));
        assert_eq!(vector.length(), 0);
        assert_eq!(vector.pop(), None);
        assert_eq!(vector.length(), 0);
        vector.push(false).unwrap();
        vector.push(true).unwrap();
        assert_eq!(vector.get(1), Some(1));
        assert_eq!(vector.length(), 2);

        // reallocate.
        let mut vector = BitVector::new(0);
        for _ in 0..3 {
            vector.push(true).unwrap();
        }
        vector.reallocate(35);
        assert_eq!(vector.capacity(), 64);
        assert_eq!(vector.length(), 3);
        vector.reallocate(2);
        assert_eq!(vector.capacity(), 32);
        assert_eq!(vector.length(), 2);

        // grow, with a custom policy.
        let mut vector = with(2, |capacity| Some(capacity + 32.0));
        vector.grow(Some(37)).unwrap();
        assert_eq!(vector.capacity(), 64);
        vector.grow(Some(37)).unwrap();
        assert_eq!(vector.capacity(), 64);
        vector.grow(None).unwrap();
        assert_eq!(vector.capacity(), 96);

        // resize.
        let mut vector = BitVector::new(64);
        vector.resize(20);
        assert_eq!((vector.capacity(), vector.length()), (64, 20));
        vector.resize(87);
        assert_eq!((vector.capacity(), vector.length()), (96, 87));

        // a policy that returns the same capacity.
        let mut vector = with(32, Some);
        assert_eq!(vector.push(true).unwrap_err(), Error::PolicyTooSmall);

        // a custom policy of capacity + 2.
        let mut vector = with(30, |capacity| Some(capacity + 2.0));
        for _ in 0..3 {
            vector.push(true).unwrap();
        }
        assert_eq!(vector.length(), 33);
        assert_eq!(vector.capacity(), 64);

        // toJSON.
        let mut vector = BitVector::new(10);
        for index in [2, 8, 9] {
            vector.set(index, true).unwrap();
        }
        assert_eq!(vector.to_json(), vec![772]);
    }

    /// Gap: the three `push`/`pop` defects, in the sequence upstream's own test
    /// walks and then stops one assertion short of.
    ///
    /// Measured on Node: `get(0)` is `1` after the `push(0)`, and `size` ends
    /// at 3 with two bits set.
    #[test]
    fn pop_leaves_size_and_the_bits_behind() {
        let mut vector = BitVector::new(0);

        vector.push(true).unwrap();
        vector.push(true).unwrap();
        assert_eq!(vector.size(), 2);

        vector.pop();
        vector.pop();

        assert_eq!(vector.length(), 0);
        // Neither the counter nor the bits moved.
        assert_eq!(vector.size(), 2);
        assert_eq!(vector.words().to_vec(), vec![0b11]);

        // push(false) bumps the length and clears nothing.
        vector.push(false).unwrap();
        assert_eq!(vector.length(), 1);
        assert_eq!(vector.get(0), Some(1));

        // and push(true) counts a bit that was already set.
        vector.push(true).unwrap();
        assert_eq!(vector.size(), 3);
        assert_eq!(vector.rank(vector.length() as i64), 2);
    }

    /// The narrower half on its own: `size` counts a re-pushed bit twice.
    #[test]
    fn pushing_true_onto_an_already_set_slot_counts_it_twice() {
        let mut vector = BitVector::new(0);

        vector.push(true).unwrap();
        vector.pop();
        vector.push(true).unwrap();

        assert_eq!(vector.size(), 2);
        assert_eq!(vector.words().to_vec()[0].count_ones(), 1);
    }

    /// Gap: `set(length, v)` is admitted by the strict `<`, writing into the
    /// capacity region without moving `length`. Same shape as
    /// `HashedArrayTree`, and equally untested.
    ///
    /// Measured: `new BitVector(5); set(5, 1)` gives `size 1`, `get(5) === 1`,
    /// `test(5) === true` and `rank(5) === 0`.
    #[test]
    fn set_at_length_writes_a_bit_that_length_does_not_cover() {
        let mut vector = BitVector::new(5);

        vector.set(5, true).unwrap();

        assert_eq!(vector.size(), 1);
        assert_eq!(vector.get(5), Some(1));
        assert!(vector.test(5));
        assert_eq!(vector.words().to_vec(), vec![1 << 5]);
        // Nothing that respects `length` can see it.
        assert_eq!(vector.rank(5), 0);
        assert_eq!(vector.values().count(), 5);
        // And one past that really is out of bounds.
        assert_eq!(vector.set(6, true).unwrap_err(), Error::IndexOutOfBounds);
        assert_eq!(vector.get(6), None);
    }

    /// Gap: BUG-BIT-SET-2. A length of 0 over a non-empty array still walks 32 bits,
    /// because `0 % 32` is falsy and `|| 32` fires. `BitSet` cannot reach this.
    ///
    /// Measured on Node: `new BitVector(); v.grow();` then `forEach` runs 32
    /// times.
    #[test]
    fn a_zero_length_vector_with_capacity_still_iterates_a_whole_word() {
        let mut vector = BitVector::new(0);

        assert_eq!(vector.values().count(), 0, "no capacity, no bits");

        vector.grow(None).unwrap();

        assert_eq!(vector.length(), 0);
        assert_eq!(vector.capacity(), 32);
        assert_eq!(vector.values().count(), 32);
        assert_eq!(vector.entries().count(), 32);

        let mut seen = 0;
        vector.for_each(|_, _| seen += 1);
        assert_eq!(seen, 32);
    }

    /// The same misfire at every other multiple of 32, where it is invisible
    /// because it is also correct.
    #[test]
    fn a_length_that_exactly_fills_its_words_walks_all_of_them() {
        for length in [32usize, 64, 96] {
            let vector = BitVector::new(length);

            assert_eq!(vector.values().count(), length, "length {length}");
        }
    }

    /// Gap: `reallocate` clamps `length` before its early returns, so a shrink
    /// to a capacity that rounds to the current one still moves `length`.
    #[test]
    fn reallocate_clamps_length_even_when_the_capacity_does_not_change() {
        let mut vector = BitVector::new(32);

        assert_eq!((vector.length(), vector.capacity()), (32, 32));

        // 20 rounds up to 32, which is the current capacity, so the array is
        // untouched -- but `length` is clamped first.
        vector.reallocate(20);

        assert_eq!((vector.length(), vector.capacity()), (20, 32));
        assert_eq!(vector.words().word_count(), 1);
    }

    /// Gap: `reallocate(0)` empties the array outright, which upstream's test
    /// never does.
    #[test]
    fn reallocate_to_zero_drops_the_array_and_the_length() {
        let mut vector = BitVector::new(0);

        for _ in 0..3 {
            vector.push(true).unwrap();
        }

        vector.reallocate(0);

        assert_eq!((vector.length(), vector.capacity()), (0, 0));
        assert_eq!(vector.words().word_count(), 0);
        // `size` survives, because nothing decrements it.
        assert_eq!(vector.size(), 3);
        assert_eq!(vector.to_json(), Vec::<u32>::new());
    }

    /// Gap: shrinking `reallocate` truncates the words, and the bits above the
    /// cut are gone rather than merely hidden.
    #[test]
    fn a_shrinking_reallocate_discards_the_words_above_the_cut() {
        let mut vector = BitVector::new(96);

        vector.set(3, true).unwrap();
        vector.set(70, true).unwrap();
        assert_eq!(vector.words().word_count(), 3);

        vector.reallocate(40);

        assert_eq!(vector.capacity(), 64);
        assert_eq!(vector.length(), 40);
        assert_eq!(vector.words().word_count(), 2);
        assert_eq!(vector.get(3), Some(1));
        // Bit 70 is past the array now; the read is inert rather than a panic.
        assert_eq!(vector.get(70), None);
        // And growing back does not bring it back.
        vector.resize(96);
        assert_eq!(vector.get(70), Some(0));
    }

    /// Gap: `applyPolicy(0)` falls back to the current capacity, because
    /// `override || this.capacity` treats zero as absent. This is what makes
    /// `grow()` work on a fresh vector at all.
    #[test]
    fn a_zero_override_falls_back_to_the_current_capacity() {
        let vector = BitVector::new(32);

        assert_eq!(vector.apply_policy(Some(0)).unwrap(), 64);
        assert_eq!(vector.apply_policy(None).unwrap(), 64);

        // And on a fresh vector the default policy's Math.max(1, …) is what
        // stops it stalling at zero.
        let empty = BitVector::new(0);
        assert_eq!(empty.apply_policy(None).unwrap(), 32);
    }

    /// Gap: the two policy throws, and the values that reach each. Upstream
    /// tests only the "less or equal" one.
    #[test]
    fn a_policy_can_fail_three_ways() {
        // Not a number at all.
        let vector = with(32, |_| None);
        assert_eq!(
            vector.apply_policy(None).unwrap_err(),
            Error::PolicyInvalidValue
        );

        // Negative.
        let vector = with(32, |_| Some(-1.0));
        assert_eq!(
            vector.apply_policy(None).unwrap_err(),
            Error::PolicyInvalidValue
        );

        // Not larger than the current capacity.
        let vector = with(32, Some);
        assert_eq!(
            vector.apply_policy(None).unwrap_err(),
            Error::PolicyTooSmall
        );
        let vector = with(64, |_| Some(10.0));
        assert_eq!(
            vector.apply_policy(None).unwrap_err(),
            Error::PolicyTooSmall
        );

        // Our own refusal, where upstream would propagate NaN into an
        // allocation size.
        let vector = with(32, |_| Some(f64::NAN));
        assert_eq!(
            vector.apply_policy(None).unwrap_err(),
            Error::PolicyNotRepresentable
        );
        let vector = with(32, |_| Some(f64::INFINITY));
        assert_eq!(
            vector.apply_policy(None).unwrap_err(),
            Error::PolicyNotRepresentable
        );
    }

    /// Gap: a non-integer policy result is accepted and rounded up to a word.
    /// Measured: a policy returning 40.5 from capacity 32 gives capacity 64.
    #[test]
    fn a_non_integer_policy_result_is_rounded_up_to_a_word() {
        let vector = with(32, |_| Some(40.5));

        assert_eq!(vector.apply_policy(None).unwrap(), 64);
    }

    /// Gap: `grow(capacity)`'s loop applies the policy repeatedly, and the
    /// "less or equal" check compares against `this.capacity` rather than the
    /// running value — so only the first iteration can throw.
    #[test]
    fn grow_loops_the_policy_until_it_covers_the_target() {
        let mut vector = with(0, |capacity| Some(capacity + 32.0));

        vector.grow(Some(200)).unwrap();

        assert_eq!(vector.capacity(), 224);
        assert_eq!(vector.words().word_count(), 7);

        // A target already covered is a no-op and cannot throw, even with a
        // policy that would.
        let mut vector = with(64, Some);
        vector.grow(Some(32)).unwrap();
        assert_eq!(vector.capacity(), 64);
    }

    /// Gap: `toJSON` takes one word MORE than the length needs, clamped by the
    /// array. Upstream asserts it once, at length 10.
    #[test]
    fn to_json_takes_one_word_past_the_length_clamped_by_the_array() {
        // (10 >> 5) + 1 == 1.
        let vector = BitVector::new(10);
        assert_eq!(vector.to_json().len(), 1);

        // (64 >> 5) + 1 == 3, but the array only has 2 words.
        let vector = BitVector::new(64);
        assert_eq!(vector.to_json(), vec![0, 0]);

        // (32 >> 5) + 1 == 2, and the array has 1.
        let vector = BitVector::new(32);
        assert_eq!(vector.to_json().len(), 1);

        // An empty vector: slice(0, 1) of an empty array is empty.
        let vector = BitVector::new(0);
        assert_eq!(vector.to_json(), Vec::<u32>::new());

        // And a length whose extra word DOES exist.
        let vector = BitVector::new(40);
        assert_eq!(vector.to_json().len(), 2);
    }

    /// Gap: `reallocate` replaces the array, so an open cursor detaches — the
    /// `BitVector` counterpart of `BitSet::clear`. Measured on Node.
    #[test]
    fn reallocate_detaches_an_open_cursor() {
        let mut vector = BitVector::new(64);

        vector.set(0, true).unwrap();
        vector.set(33, true).unwrap();

        let mut cursor = vector.values();
        assert_eq!(cursor.step(), Step::Item(1));

        vector.reallocate(32);

        assert_eq!((vector.length(), vector.capacity()), (32, 32));
        assert_eq!(vector.words().word_count(), 1);

        // The cursor still walks the pre-reallocate array, all 64 steps of it.
        let rest: Vec<u32> = cursor.collect();
        assert_eq!(rest.len(), 63);
        assert_eq!(rest[32], 1);
    }

    /// Gap: growth during iteration is invisible for the same reason.
    #[test]
    fn growth_during_iteration_is_invisible_to_an_open_cursor() {
        let mut vector = BitVector::new(32);

        let cursor = vector.values();
        assert_eq!(cursor.frozen_len(), 32);

        vector.resize(96);

        // Still the frozen 32, not the new 96.
        assert_eq!(cursor.count(), 32);
        assert_eq!(vector.values().count(), 96);
    }

    /// Gap: DIV-STACK-1/DIV-STACK-2. Upstream drains each cursor once, in one expression.
    #[test]
    fn cursors_do_not_restart_but_the_vector_can_be_walked_again() {
        let mut vector = BitVector::new(4);
        vector.set(1, true).unwrap();

        let mut cursor = vector.values();
        assert_eq!(cursor.by_ref().collect::<Vec<_>>(), vec![0, 1, 0, 0]);
        assert_eq!(cursor.collect::<Vec<_>>(), Vec::<u32>::new());

        assert_eq!(vector.values().collect::<Vec<_>>(), vec![0, 1, 0, 0]);
        assert_eq!(vector.values().collect::<Vec<_>>(), vec![0, 1, 0, 0]);
    }

    /// Gap: `{initialCapacity: n}` sets the LENGTH, because upstream reads
    /// `initialLength || initialCapacity || 0`. The bridge resolves the union,
    /// so this pins the arithmetic that follows from it — which is what
    /// upstream's own "custom policy" test silently depends on.
    #[test]
    fn an_initial_length_of_thirty_derives_a_capacity_of_thirty_two() {
        let vector = BitVector::new(30);

        assert_eq!(vector.length(), 30);
        assert_eq!(vector.capacity(), 32);
        assert_eq!(vector.words().word_count(), 1);
    }

    /// Gap: `get`/`test` at exactly `length`, which the out-of-bounds test
    /// misses by twelve.
    #[test]
    fn get_is_undefined_only_strictly_past_the_length() {
        let vector = BitVector::new(5);

        assert_eq!(vector.get(4), Some(0));
        assert_eq!(vector.get(5), Some(0));
        assert_eq!(vector.get(6), None);
        assert!(!vector.test(5));
        assert!(!vector.test(6));
        assert_eq!(vector.get(17), None);
    }

    /// Gap: BUG-SPARSE-QUEUE-SET-2 and BUG-SPARSE-QUEUE-SET-3 reach `BitVector` too, because the code is
    /// copy-pasted. Verified independently on Node against `BitVector`, not
    /// inferred from `BitSet`.
    #[test]
    fn inherits_the_reset_and_select_defects_verbatim() {
        let mut vector = BitVector::new(32);
        vector.set(31, true).unwrap();
        vector.reset(0);

        assert_eq!(vector.size(), 0);
        assert_eq!(vector.get(31), Some(1));

        let mut vector = BitVector::new(64);
        vector.set(40, true).unwrap();

        // 40 is the true answer.
        assert_eq!(vector.select(1), Some(8));
    }

    /// Gap: out-of-range writes are inert, as they are for `BitSet` — the
    /// `SparseSet` corruption family does not recur here either.
    #[test]
    fn indices_past_the_backing_array_are_inert() {
        let mut vector = BitVector::new(64);

        // `set` guards, so reach the array through the unguarded pair.
        vector.reset(10_000);
        vector.flip(10_000);
        vector.reset(-1);
        vector.flip(-1);

        assert_eq!(vector.size(), 0);
        assert_eq!(vector.words().to_vec(), vec![0, 0]);
    }
}

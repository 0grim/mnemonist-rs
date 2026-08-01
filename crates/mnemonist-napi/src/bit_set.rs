//! JS bridge for [`mnemonist_core::structures::bit_set`].
//!
//! Thin translation only: every behavioural decision lives in the core crate,
//! and the interesting ones are documented on
//! [`mnemonist_core::structures::bits`] because upstream duplicates them into
//! `bit-vector.js`. Four adaptations here.
//!
//! 1. **`array` IS exposed**, unlike `SparseSet`'s `dense`/`sparse`. The
//!    original test file reads `set.array.length`, so it has to be. napi can
//!    only hand out a **copy**, so a JS caller writing through it — legal
//!    upstream — is a silent divergence. Stated in the divergence doc rather
//!    than hidden; the differential fuzzer compares the real backing store on
//!    the Rust side after every operation.
//! 2. **`set(index, value)` keys off a strict `value === 0 || value === false`.**
//!    Not truthiness: `set(i, null)` and `set(i, '')` both *set* the bit
//!    upstream. Reproduced by matching on the JS value's type rather than
//!    coercing it.
//! 3. **`size` and `rank` return `i64`**, because upstream's `size` counter can
//!    go negative (B-13) and a `u32` could not carry the state upstream
//!    reaches.
//! 4. **`select` yields `Either<i64, Undefined>`.** Upstream returns `-1`, a
//!    position, or falls out of its loop and returns `undefined` — three
//!    outcomes, and D-39 says `Option` renders the third as `null`.
//!
//! Like [`crate::queue`] and [`crate::stack`], the core structure is held in a
//! [`RefCell`] so that `&self` is not `noalias readonly` and a JS callback's
//! mutation is actually seen — see [`crate::cursor::CellCursor`] and B-31.
//! `forEach` is this module's re-entry point; the cursors need no cell,
//! because they own everything they read.

use std::cell::RefCell;

use mnemonist_core::structures::bit_set::BitSet as CoreSet;
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::cursor::BridgeBitCursor;

/// A fixed-length bit set over a `Uint32Array`.
#[napi(js_name = "BitSet")]
pub struct JsBitSet {
    inner: RefCell<CoreSet>,
}

#[napi]
impl JsBitSet {
    #[napi(constructor)]
    pub fn new(length: u32) -> Self {
        Self {
            inner: RefCell::new(CoreSet::new(length as usize)),
        }
    }

    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.borrow().length() as u32
    }

    /// Upstream's `size` counter — see B-13 for why this is signed.
    #[napi(getter)]
    pub fn size(&self) -> i64 {
        self.inner.borrow().size()
    }

    /// The backing `Uint32Array`. A **copy**; see adaptation 1.
    #[napi(getter)]
    pub fn array(&self) -> Uint32Array {
        Uint32Array::new(self.inner.borrow().to_json())
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    #[napi]
    pub fn set<'a>(&self, this: This<'a>, index: i64, value: Option<Unknown>) -> Result<This<'a>> {
        self.inner.borrow_mut().set_to(index, !clears(value)?);

        Ok(this)
    }

    #[napi]
    pub fn reset<'a>(&self, this: This<'a>, index: i64) -> This<'a> {
        self.inner.borrow_mut().reset(index);

        this
    }

    #[napi]
    pub fn flip<'a>(&self, this: This<'a>, index: i64) -> This<'a> {
        self.inner.borrow_mut().flip(index);

        this
    }

    #[napi]
    pub fn get(&self, index: i64) -> u32 {
        self.inner.borrow().get(index)
    }

    #[napi]
    pub fn test(&self, index: i64) -> bool {
        self.inner.borrow().test(index)
    }

    #[napi]
    pub fn rank(&self, i: i64) -> i64 {
        self.inner.borrow().rank(i)
    }

    /// `-1`, a position, or `undefined`. See adaptation 4.
    #[napi]
    pub fn select(&self, r: i64) -> Either<i64, Undefined> {
        match self.inner.borrow().select(r) {
            Some(position) => Either::A(position),
            None => Either::B(()),
        }
    }

    /// `forEach(callback, scope)` — `(bit, index)` per step.
    ///
    /// `scope` carries the same caveat as the `SparseSet` bridge: upstream
    /// keys off `arguments.length > 1`, which napi's typed signature cannot
    /// see, so `forEach(cb, undefined)` binds `this` to the set here where
    /// upstream would bind `undefined`. The omitted case — the only one the
    /// original suite uses — is exact.
    ///
    /// The word is snapshotted before the inner loop, exactly as upstream's
    /// `byte = this.array[i]` does, so a callback that writes to the word
    /// currently being walked does not affect the remaining bits of it.
    #[napi]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(u32, u32)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let (word_count, length) = {
            let inner = self.inner.borrow();

            (inner.words().word_count(), inner.length())
        };

        for index in 0..word_count {
            // Re-borrowed and dropped per word, before any callback runs:
            // upstream's `byte = this.array[i]` is a fresh read of the live
            // array each time round the outer loop, and a callback that
            // `set`s or `clear`s must not meet an outstanding borrow.
            let word = self.inner.borrow().words().word(index).unwrap_or(0);
            let bits = mnemonist_core::structures::bits::bits_in_word(index, word_count, length);

            for bit in 0..bits {
                let value = (((word as i32) >> bit) & 1) as u32;
                let position = (index * 32 + bit) as u32;

                match &scope {
                    Some(scope) => callback.apply(*scope, (value, position).into())?,
                    None => callback.apply(this, (value, position).into())?,
                };
            }
        }

        Ok(())
    }

    /// A fresh cursor over the bits — the factory half of D-07.
    ///
    /// Unlike `SparseSet::values`, this needs no `SharedReference`: upstream's
    /// closure captures the array *object* and never touches `this` again, so
    /// the core cursor is self-contained and owns everything it reads. That is
    /// also what makes a `clear()` invisible to an open cursor.
    #[napi]
    pub fn values(&self) -> JsBitSetValues {
        JsBitSetValues {
            cursor: BridgeBitCursor::new(self.inner.borrow().values()),
        }
    }

    #[napi]
    pub fn entries(&self) -> JsBitSetEntries {
        JsBitSetEntries {
            cursor: BridgeBitCursor::new(self.inner.borrow().values()),
        }
    }

    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Vec<u32> {
        self.inner.borrow().to_json()
    }
}

/// `BitSet.prototype.values()`.
#[napi(iterator, js_name = "BitSetValues")]
pub struct JsBitSetValues {
    cursor: BridgeBitCursor,
}

impl Generator for JsBitSetValues {
    type Yield = u32;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<u32> {
        self.cursor.next_bit()
    }
}

/// `BitSet.prototype.entries()`, yielding `[index, bit]`.
#[napi(iterator, js_name = "BitSetEntries")]
pub struct JsBitSetEntries {
    cursor: BridgeBitCursor,
}

impl Generator for JsBitSetEntries {
    type Yield = Vec<u32>;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Vec<u32>> {
        self.cursor.next_entry()
    }
}

/// Upstream's `value === 0 || value === false`, which is a **strict** test.
///
/// Truthiness would be wrong in both directions: `null`, `''`, `NaN` and
/// `undefined` are all falsy and all *set* the bit upstream. Only a numeric
/// zero or a boolean `false` clear it.
pub fn clears(value: Option<Unknown>) -> Result<bool> {
    let Some(value) = value else {
        return Ok(false);
    };

    match value.get_type()? {
        ValueType::Number => Ok(f64::from_unknown(value)? == 0.0),
        ValueType::Boolean => Ok(!bool::from_unknown(value)?),
        _ => Ok(false),
    }
}

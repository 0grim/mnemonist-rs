//! The backing array the fixed-capacity structures allocate up front.
//!
//! `FixedStack`, `FixedDeque` and `CircularBuffer` are all constructed the same
//! way upstream:
//!
//! ```js
//! function FixedStack(ArrayClass, capacity) {
//!   ...
//!   this.items = new this.ArrayClass(this.capacity);
//! }
//! ```
//!
//! so the *class* is a constructor argument and the structure's storage is
//! whatever that class produces. `Array` and `Uint8Array` are both used by the
//! original test files, and they are not interchangeable — they differ on two
//! points that the three modules' own code depends on:
//!
//! | | `new Array(n)` | `new Uint8Array(n)` |
//! |---|---|---|
//! | unwritten slot reads as | `undefined` (a *hole*) | `0` |
//! | store past the end | **grows** the array | silently **dropped** |
//!
//! Both differences are reachable through the public API. `FixedStack.forEach`
//! walks `this.items.length` rather than `this.size` (NOTES BUG-FIXED-STACK-1), so every
//! unused slot is handed to the callback — as `undefined` from an `Array` and
//! as `0` from a `Uint8Array`. And `X.from(iterable, ArrayClass, capacity)`
//! writes `items[i] = iterable[i]` for the *iterable's* whole length without
//! consulting `capacity`, so an oversized iterable grows an `Array` past its
//! own capacity and is truncated by a typed array:
//!
//! ```js
//! FixedStack.from([1, 2, 3], Array, 2).items       // [1, 2, 3], length 3
//! FixedStack.from([1, 2, 3], Uint8Array, 2).items  // Uint8Array(2) [1, 2]
//! ```
//!
//! Both measured on Node 24.18.1.
//!
//! # What this type is, and what it deliberately is not
//!
//! It is those two bits, and nothing else. It is **not** a model of a JS array
//! class: element coercion — `Uint8Array` storing `300` as `44` — is a
//! JavaScript-value semantic and lives at the napi boundary with the rest of
//! them (`docs/ARCHITECTURE.md`'s boundary rule). A Rust caller picks [`Backing::Holes`] for
//! "`undefined` where nothing was written" or [`Backing::Filled`] for "this
//! value where nothing was written, and a fixed length", and gets exactly the
//! two behaviours above.
//!
//! The slot type is `Option<T>` throughout, where `None` is the JavaScript
//! `undefined` that an unwritten or out-of-range read produces. For
//! [`Backing::Filled`] the `None`s are all replaced at allocation time, so a
//! `None` there can only come from a *dropped* store — which is exactly when
//! upstream reads off the end of a typed array and gets `undefined` too.

/// How the JS array class a structure was constructed with behaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backing<T> {
    /// `new Array(capacity)`: `capacity` holes, and a store past the end grows
    /// the array rather than being refused.
    Holes,
    /// `new SomeTypedArray(capacity)`: `capacity` copies of the class's zero
    /// element, and a store past the end is silently dropped.
    Filled(T),
}

impl<T: Clone> Backing<T> {
    /// `new ArrayClass(capacity)`.
    pub fn allocate(&self, capacity: usize) -> Vec<Option<T>> {
        match self {
            Self::Holes => vec![None; capacity],
            Self::Filled(zero) => vec![Some(zero.clone()); capacity],
        }
    }

    /// `items[index] = value`, with this class's out-of-range behaviour.
    ///
    /// Returns whether the store landed, which is the only thing the two
    /// variants disagree about once the index is in range.
    ///
    /// The growth path fills the skipped positions with holes rather than with
    /// the zero element, because that is what a JS `Array` does: `var a = [];
    /// a[3] = 1` leaves `a` as `[<3 empty items>, 1]`, not `[0, 0, 0, 1]`.
    pub fn store(&self, items: &mut Vec<Option<T>>, index: usize, value: T) -> bool {
        if let Some(slot) = items.get_mut(index) {
            *slot = Some(value);

            return true;
        }

        match self {
            Self::Filled(_) => false,
            Self::Holes => {
                items.resize(index, None);
                items.push(Some(value));

                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holes_allocate_as_undefined_and_a_typed_class_as_its_zero() {
        assert_eq!(Backing::<i32>::Holes.allocate(3), vec![None, None, None]);
        assert_eq!(
            Backing::Filled(0i32).allocate(3),
            vec![Some(0), Some(0), Some(0)]
        );
        assert_eq!(Backing::<i32>::Holes.allocate(0), Vec::<Option<i32>>::new());
    }

    #[test]
    fn an_in_range_store_lands_whatever_the_class() {
        let mut holes = Backing::<i32>::Holes.allocate(2);
        assert!(Backing::Holes.store(&mut holes, 1, 7));
        assert_eq!(holes, vec![None, Some(7)]);

        let mut filled = Backing::Filled(0i32).allocate(2);
        assert!(Backing::Filled(0).store(&mut filled, 0, 7));
        assert_eq!(filled, vec![Some(7), Some(0)]);
    }

    /// The `X.from(oversized, Array, small)` half: an `Array` grows.
    #[test]
    fn a_store_past_the_end_grows_a_plain_array() {
        let mut items = Backing::<i32>::Holes.allocate(2);

        assert!(Backing::Holes.store(&mut items, 2, 9));
        assert_eq!(items, vec![None, None, Some(9)]);

        // …and skipping positions leaves holes, not zeroes.
        assert!(Backing::Holes.store(&mut items, 5, 4));
        assert_eq!(items, vec![None, None, Some(9), None, None, Some(4)]);
    }

    /// The other half: a typed array drops it, without growing and without
    /// throwing.
    #[test]
    fn a_store_past_the_end_is_dropped_by_a_typed_class() {
        let mut items = Backing::Filled(0i32).allocate(2);

        assert!(!Backing::Filled(0).store(&mut items, 2, 9));
        assert_eq!(items, vec![Some(0), Some(0)]);
        assert!(!Backing::Filled(0).store(&mut items, usize::MAX, 9));
        assert_eq!(items.len(), 2);
    }
}

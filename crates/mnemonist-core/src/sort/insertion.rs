//! Port of upstream `sort/insertion.js`.
//!
//! Plain insertion sort over a half-open window `[lo, hi)`, in two flavours:
//! one permutes the values, one permutes a separate index array.
//!
//! # The shift is written as adjacent swaps, and that is exact
//!
//! Upstream lifts the element out (`k = array[i]`), shifts the sorted prefix
//! right one slot at a time, then drops `k` into the hole. Doing that in Rust
//! would need `T: Clone` for no reason, because the same permutation falls out
//! of swapping the travelling element with its left neighbour: after each swap
//! the element sits at `j`, so `array[j - 1] > array[j]` *is* upstream's
//! `array[j] > k`. Same comparisons, same order, same writes, no temporary.
//!
//! # B-80: upstream's loop counter is a global
//!
//! Both functions below open with `i = lo + 1` and **never declare `i`**.
//! `sort/insertion.js` is not a module in the ESM sense and runs sloppy, so
//! that assignment creates `globalThis.i` and every call shares it. Confirmed
//! against Node 24.18.1: after one `inplaceInsertionSort([3, 1, 2], 0, 3)`,
//! `global.i` is `3`.
//!
//! It is not merely untidy. `>` invokes `valueOf`, so an element can re-enter
//! the sorter mid-comparison, and the inner call then leaves the outer one's
//! counter wherever it finished. Measured: a four-element array whose second
//! element re-enters sorts to `[1, 5, 3, 2]` instead of `[1, 2, 3, 5]`.
//!
//! This port has no such state — the counter is a local, as it is in every
//! other language. The divergence is unreachable from the bridge, which
//! accepts numbers and nothing that can carry a `valueOf`; see
//! `docs/modules/sort.md`.

use super::{check_window, gt};
use crate::utils::typed_arrays::PointerVec;

/// Sort `array[lo..hi)` ascending, in place.
///
/// # Panics
///
/// Panics unless `lo <= hi <= array.len()`. See [`super::check_window`].
pub fn inplace_insertion_sort<T: PartialOrd>(array: &mut [T], lo: usize, hi: usize) {
    check_window(lo, hi, array.len(), "the array");

    for i in (lo + 1)..hi {
        // `j` is upstream's `j + 1`: the slot the travelling element occupies.
        // Upstream's `j >= lo` guard becomes `j > lo`, and its `array[j] > k`
        // becomes `array[j - 1] > array[j]`, because the element is already
        // sitting at `j`.
        let mut j = i;

        while j > lo && array[j - 1] > array[j] {
            array.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// Sort `indices[lo..hi)` by the `array` values they point at, in place.
///
/// `array` is read only. An index pointing past the end of `array` reads
/// `undefined` upstream, which loses every comparison it takes part in; see
/// [`super::gt`].
///
/// # Panics
///
/// Panics unless `lo <= hi <= indices.len()`. `array` is not bounds-checked,
/// because reading past its end is upstream behaviour rather than an error.
pub fn inplace_insertion_sort_indices<T: PartialOrd>(
    array: &[T],
    indices: &mut PointerVec,
    lo: usize,
    hi: usize,
) {
    check_window(lo, hi, indices.len(), "the indices array");

    for i in (lo + 1)..hi {
        let mut j = i;

        while j > lo
            && gt(
                array.get(indices.get(j - 1) as usize),
                array.get(indices.get(j) as usize),
            )
        {
            let left = indices.get(j - 1);
            let right = indices.get(j);

            indices.set(j - 1, right);
            indices.set(j, left);

            j -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upstream's own fixture, and the exact array `test/sort.js` asserts.
    const DATA: [i32; 11] = [2, 7, 1, 5, 8, 9, 1, -3, 3, 18, 6];

    fn indices_of(len: usize) -> PointerVec {
        crate::utils::typed_arrays::indices(len).expect("len fits a pointer array")
    }

    fn as_vec(indices: &PointerVec) -> Vec<u32> {
        (0..indices.len()).map(|slot| indices.get(slot)).collect()
    }

    #[test]
    fn sorts_the_whole_window_like_upstream() {
        let mut data = DATA;
        inplace_insertion_sort(&mut data, 0, DATA.len());

        assert_eq!(data, [-3, 1, 1, 2, 3, 5, 6, 7, 8, 9, 18]);
    }

    /// The three slices `test/sort.js` checks, byte for byte.
    #[test]
    fn sorts_only_the_requested_slice() {
        let mut data = DATA;
        inplace_insertion_sort(&mut data, 0, 3);
        assert_eq!(data, [1, 2, 7, 5, 8, 9, 1, -3, 3, 18, 6]);

        let mut data = DATA;
        inplace_insertion_sort(&mut data, 3, 7);
        assert_eq!(data, [2, 7, 1, 1, 5, 8, 9, -3, 3, 18, 6]);

        let mut data = DATA;
        inplace_insertion_sort(&mut data, 5, 11);
        assert_eq!(data, [2, 7, 1, 5, 8, -3, 1, 3, 6, 9, 18]);
    }

    #[test]
    fn degenerate_windows_do_nothing() {
        let mut data = [1];
        inplace_insertion_sort(&mut data, 0, 1);
        assert_eq!(data, [1]);

        let mut data = [2, 1];
        inplace_insertion_sort(&mut data, 0, 2);
        assert_eq!(data, [1, 2]);

        // Empty window, and an empty slice: both are no-ops, not panics.
        let mut data = DATA;
        inplace_insertion_sort(&mut data, 4, 4);
        assert_eq!(data, DATA);

        inplace_insertion_sort::<i32>(&mut [], 0, 0);
    }

    /// Insertion sort is stable, and the equal keys in `DATA` are the only
    /// place `test/sort.js` could have noticed. It does not: it compares the
    /// sorted values, where the two `1`s are indistinguishable. The indices
    /// flavour makes stability visible, and it is what separates the expected
    /// index arrays of this file and `quick`.
    #[test]
    fn is_stable_where_quick_sort_is_not() {
        let mut indices = indices_of(DATA.len());
        inplace_insertion_sort_indices(&DATA, &mut indices, 0, DATA.len());

        // Members 2 and 6 both hold `1`; the earlier one stays earlier.
        assert_eq!(as_vec(&indices), vec![7, 2, 6, 0, 8, 3, 10, 1, 4, 5, 9]);
    }

    #[test]
    fn sorts_only_the_requested_slice_of_indices() {
        for (lo, hi, expected) in [
            (0usize, 3usize, vec![2u32, 0, 1, 3, 4, 5, 6, 7, 8, 9, 10]),
            (3, 7, vec![0, 1, 2, 6, 3, 4, 5, 7, 8, 9, 10]),
            (5, 11, vec![0, 1, 2, 3, 4, 7, 6, 8, 10, 5, 9]),
        ] {
            let mut indices = indices_of(DATA.len());
            inplace_insertion_sort_indices(&DATA, &mut indices, lo, hi);

            assert_eq!(as_vec(&indices), expected, "window {lo}..{hi}");
        }
    }

    /// The regime `test/sort.js` never enters: an index pointing past the end
    /// of `array`. Upstream reads `undefined`, every comparison against it is
    /// false, and the element therefore never moves left. Reproduced here, and
    /// the point of [`super::gt`] taking `Option`.
    #[test]
    fn indices_past_the_end_of_the_array_never_move() {
        let array = [10i32, 20, 30];
        let mut indices = PointerVec::U8(vec![9, 2, 0]);

        inplace_insertion_sort_indices(&array, &mut indices, 0, 3);

        // 9 is out of range: `array[9] > array[2]` is false, so it stays put;
        // then 0 sinks past 2 normally.
        assert_eq!(as_vec(&indices), vec![9, 0, 2]);
    }

    /// A window at the very end is not the same as one past it.
    #[test]
    #[should_panic(expected = "is past the end")]
    fn a_window_past_the_indices_array_panics() {
        let array = [1i32, 2, 3];
        let mut indices = PointerVec::U8(vec![0, 1, 2]);

        inplace_insertion_sort_indices(&array, &mut indices, 0, 4);
    }

    /// `NaN` loses every comparison, so it never sinks and nothing sinks past
    /// it. `test/sort.js` sorts only finite numbers.
    #[test]
    fn nan_pins_the_elements_to_its_right() {
        let mut data = [3.0f64, f64::NAN, 1.0, 2.0];
        inplace_insertion_sort(&mut data, 0, 4);

        assert_eq!(data[0], 3.0);
        assert!(data[1].is_nan());
        assert_eq!(data[2], 1.0);
        assert_eq!(data[3], 2.0);
    }

    /// 1,000 random-ish values, the shape of upstream's "sanity test" but
    /// deterministic so a failure is reproducible.
    #[test]
    fn sanity_sorts_a_thousand_values() {
        let mut data: Vec<i64> = (0..1000).map(|k: i64| (k * 7919) % 1009).collect();
        let mut expected = data.clone();
        expected.sort_unstable();

        inplace_insertion_sort(&mut data, 0, 1000);

        assert_eq!(data, expected);
    }
}

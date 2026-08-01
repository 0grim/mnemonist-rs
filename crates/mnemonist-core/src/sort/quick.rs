//! Port of upstream `sort/quick.js`.
//!
//! An explicit-stack quicksort over a half-open window `[lo, hi)`, adapted
//! upstream from <https://alienryderflex.com/quicksort/>. Two flavours, as in
//! [`super::insertion`]: one permutes the values, one permutes an index array.
//!
//! # Why this is transcribed statement by statement
//!
//! It would be very easy to "port" this as `slice::sort_unstable_by`, and it
//! would pass four of the six `it` blocks in `test/sort.js`. The other two
//! assert the exact index permutation — `[7, 6, 2, 0, 8, 3, 10, 1, 4, 5, 9]`
//! — which is a property of *this* partition scheme and of the order it walks
//! its stack. Quicksort is not stable, so "sorted" does not pin the answer:
//! `DATA` holds `1` twice, and insertion sort puts them in one order while
//! this puts them in the other. The permutation is observable, so the
//! algorithm is the contract, not the postcondition.
//!
//! # The 64-slot stack, kept rather than replaced with a `Vec`
//!
//! Upstream's `LOS`/`HIS` are two `Float64Array(64)` allocated **once at
//! module scope**. The depth bound is real and not a guess: the block after
//! each partition swaps the two halves when the right one is larger, so the
//! entry the loop picks up next is always the *smaller* half and the larger
//! one waits below it. Sizes therefore at least double as the index falls, and
//! the stack cannot exceed `log2(hi - lo)` entries — 64 slots cover every
//! window a 64-bit machine can address.
//!
//! # B-81: upstream's stack is shared between calls, and corrupts under one
//!
//! Because `LOS`/`HIS` are module scope, all four exported sorts share them —
//! `inplaceQuickSort` and `inplaceQuickSortIndices` included. `>=` invokes
//! `valueOf`, so an element can re-enter the sorter mid-partition, and the
//! inner call overwrites the outer call's pending stack entries while the
//! outer call's `i` keeps pointing into them. Measured against Node 24.18.1: a
//! 40-element array whose first compared element re-enters comes back with 38
//! of its 40 elements out of order.
//!
//! The stack here is a local, so the port has no such state. Like B-80 the
//! divergence is unreachable through the bridge, which accepts numbers and
//! nothing that can carry a `valueOf`; see `docs/modules/sort.md`.

use super::{check_window, ge, le};
use crate::utils::typed_arrays::PointerVec;

/// Slots in the partition stack, matching upstream's `Float64Array(64)`.
const STACK: usize = 64;

/// Sort `array[lo..hi)` ascending, in place.
///
/// # Panics
///
/// Panics unless `lo <= hi <= array.len()`. See [`super::check_window`].
pub fn inplace_quick_sort<T: PartialOrd + Clone>(array: &mut [T], lo: usize, hi: usize) {
    check_window(lo, hi, array.len(), "the array");

    let mut los = [0isize; STACK];
    let mut his = [0isize; STACK];

    los[0] = lo as isize;
    his[0] = hi as isize;

    // Signed because `his[i] - 1` is `-1` for an empty window, which is
    // exactly the value that makes `l < r` false and pops the entry.
    let mut i: isize = 0;

    while i >= 0 {
        let top = i as usize;
        let mut l = los[top];
        let mut r = his[top] - 1;

        if l >= r {
            i -= 1;
            continue;
        }

        let pivot = array[l as usize].clone();

        while l < r {
            while array[r as usize] >= pivot && l < r {
                r -= 1;
            }

            if l < r {
                array[l as usize] = array[r as usize].clone();
                l += 1;
            }

            while array[l as usize] <= pivot && l < r {
                l += 1;
            }

            if l < r {
                array[r as usize] = array[l as usize].clone();
                r -= 1;
            }
        }

        array[l as usize] = pivot;

        push_halves(&mut los, &mut his, &mut i, l);
    }
}

/// Sort `indices[lo..hi)` by the `array` values they point at, in place.
///
/// `array` is read only. An index pointing past the end of `array` reads
/// `undefined` upstream, which loses every comparison; see [`super::ge`].
///
/// # Panics
///
/// Panics unless `lo <= hi <= indices.len()`. `array` is not bounds-checked,
/// because reading past its end is upstream behaviour rather than an error.
pub fn inplace_quick_sort_indices<T: PartialOrd>(
    array: &[T],
    indices: &mut PointerVec,
    lo: usize,
    hi: usize,
) {
    check_window(lo, hi, indices.len(), "the indices array");

    let mut los = [0isize; STACK];
    let mut his = [0isize; STACK];

    los[0] = lo as isize;
    his[0] = hi as isize;

    let mut i: isize = 0;

    while i >= 0 {
        let top = i as usize;
        let mut l = los[top];
        let mut r = his[top] - 1;

        if l >= r {
            i -= 1;
            continue;
        }

        let held = indices.get(l as usize);
        // `p = array[t]`, which is `undefined` when `t` is out of range.
        let pivot = array.get(held as usize);

        while l < r {
            while ge(array.get(indices.get(r as usize) as usize), pivot) && l < r {
                r -= 1;
            }

            if l < r {
                let moved = indices.get(r as usize);
                indices.set(l as usize, moved);
                l += 1;
            }

            while le(array.get(indices.get(l as usize) as usize), pivot) && l < r {
                l += 1;
            }

            if l < r {
                let moved = indices.get(l as usize);
                indices.set(r as usize, moved);
                r -= 1;
            }
        }

        indices.set(l as usize, held);

        push_halves(&mut los, &mut his, &mut i, l);
    }
}

/// Replace the entry at `i` with its two halves, smaller half on top.
///
/// This is upstream's tail, verbatim, and it is shared because the two
/// flavours run it identically:
///
/// ```js
/// LOS[i + 1] = l + 1;
/// HIS[i + 1] = HIS[i];
/// HIS[i++] = l;
///
/// if (HIS[i] - LOS[i] > HIS[i - 1] - LOS[i - 1]) { /* swap i and i - 1 */ }
/// ```
///
/// The swap is what bounds the stack: after it, index `i` — the entry the loop
/// picks up next — always holds the smaller half.
fn push_halves(los: &mut [isize; STACK], his: &mut [isize; STACK], i: &mut isize, l: isize) {
    let top = *i as usize;

    los[top + 1] = l + 1;
    his[top + 1] = his[top];
    his[top] = l;

    *i += 1;

    let next = *i as usize;

    if his[next] - los[next] > his[next - 1] - los[next - 1] {
        los.swap(next, next - 1);
        his.swap(next, next - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::utils::typed_arrays::PointerWidth;

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
        inplace_quick_sort(&mut data, 0, DATA.len());

        assert_eq!(data, [-3, 1, 1, 2, 3, 5, 6, 7, 8, 9, 18]);
    }

    #[test]
    fn sorts_only_the_requested_slice() {
        let mut data = DATA;
        inplace_quick_sort(&mut data, 0, 3);
        assert_eq!(data, [1, 2, 7, 5, 8, 9, 1, -3, 3, 18, 6]);

        let mut data = DATA;
        inplace_quick_sort(&mut data, 3, 7);
        assert_eq!(data, [2, 7, 1, 1, 5, 8, 9, -3, 3, 18, 6]);

        let mut data = DATA;
        inplace_quick_sort(&mut data, 5, 11);
        assert_eq!(data, [2, 7, 1, 5, 8, -3, 1, 3, 6, 9, 18]);
    }

    #[test]
    fn degenerate_windows_do_nothing() {
        let mut data = [1];
        inplace_quick_sort(&mut data, 0, 1);
        assert_eq!(data, [1]);

        let mut data = DATA;
        inplace_quick_sort(&mut data, 4, 4);
        assert_eq!(data, DATA);

        inplace_quick_sort::<i32>(&mut [], 0, 0);
    }

    /// The permutation, not merely the order. `test/sort.js` asserts this
    /// exact array, and it differs from `insertion`'s at members 1 and 2
    /// because quicksort is not stable.
    #[test]
    fn produces_upstreams_exact_index_permutation() {
        let mut indices = indices_of(DATA.len());
        inplace_quick_sort_indices(&DATA, &mut indices, 0, DATA.len());

        assert_eq!(as_vec(&indices), vec![7, 6, 2, 0, 8, 3, 10, 1, 4, 5, 9]);
    }

    /// …and this is the pair that proves the point: same data, same window,
    /// two different permutations, both sorted.
    #[test]
    fn disagrees_with_insertion_sort_on_equal_keys() {
        let mut quick = indices_of(DATA.len());
        inplace_quick_sort_indices(&DATA, &mut quick, 0, DATA.len());

        let mut insertion = indices_of(DATA.len());
        crate::sort::insertion::inplace_insertion_sort_indices(
            &DATA,
            &mut insertion,
            0,
            DATA.len(),
        );

        assert_ne!(as_vec(&quick), as_vec(&insertion));

        // Both are nonetheless in non-decreasing order by value.
        for permutation in [&quick, &insertion] {
            let values: Vec<i32> = as_vec(permutation)
                .into_iter()
                .map(|member| DATA[member as usize])
                .collect();

            assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
        }
    }

    #[test]
    fn sorts_only_the_requested_slice_of_indices() {
        for (lo, hi, expected) in [
            (0usize, 3usize, vec![2u32, 0, 1, 3, 4, 5, 6, 7, 8, 9, 10]),
            (3, 7, vec![0, 1, 2, 6, 3, 4, 5, 7, 8, 9, 10]),
            (5, 11, vec![0, 1, 2, 3, 4, 7, 6, 8, 10, 5, 9]),
        ] {
            let mut indices = indices_of(DATA.len());
            inplace_quick_sort_indices(&DATA, &mut indices, lo, hi);

            assert_eq!(as_vec(&indices), expected, "window {lo}..{hi}");
        }
    }

    /// The stack bound is an argument, so it gets a check: an already-sorted
    /// input is quicksort's classic worst case for naive implementations, and
    /// this partition scheme survives it only because of the swap in
    /// [`push_halves`].
    #[test]
    fn already_sorted_input_does_not_overflow_the_stack() {
        let mut data: Vec<i32> = (0..4096).collect();
        inplace_quick_sort(&mut data, 0, 4096);

        assert!(data.windows(2).all(|pair| pair[0] < pair[1]));

        let mut reversed: Vec<i32> = (0..4096).rev().collect();
        inplace_quick_sort(&mut reversed, 0, 4096);

        assert!(reversed.windows(2).all(|pair| pair[0] < pair[1]));

        // All-equal, the other degenerate partition.
        let mut flat = vec![7i32; 4096];
        inplace_quick_sort(&mut flat, 0, 4096);

        assert_eq!(flat, vec![7i32; 4096]);
    }

    /// Out-of-range indices, the regime `test/sort.js` never enters. Upstream
    /// reads `undefined` and every comparison against it is false; the port
    /// does the same rather than panicking.
    #[test]
    fn indices_past_the_end_of_the_array_do_not_panic() {
        let array = [10i32, 20, 30];
        let mut indices = PointerVec::U8(vec![9, 2, 0, 8]);

        inplace_quick_sort_indices(&array, &mut indices, 0, 4);

        // Whatever the permutation, nothing is lost or duplicated.
        let mut seen = as_vec(&indices);
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 2, 8, 9]);
    }

    #[test]
    #[should_panic(expected = "is past the end")]
    fn a_window_past_the_array_panics() {
        let mut data = [1i32, 2, 3];
        inplace_quick_sort(&mut data, 0, 4);
    }

    /// The three widths behave identically, which is worth pinning because the
    /// bridge picks one from the caller's typed array and nothing else does.
    #[test]
    fn every_pointer_width_sorts_the_same_way() {
        let array: Vec<i32> = (0..300).map(|k| (k * 7919) % 301).collect();
        let expected = {
            let mut indices = crate::utils::typed_arrays::indices(300).unwrap();
            inplace_quick_sort_indices(&array, &mut indices, 0, 300);
            as_vec(&indices)
        };

        assert_eq!(
            crate::utils::typed_arrays::indices(300).unwrap().width(),
            PointerWidth::U16
        );

        for width in [PointerWidth::U16, PointerWidth::U32] {
            let mut indices = PointerVec::zeroed(width, 300);
            for slot in 0..300 {
                indices.set(slot, slot as u32);
            }

            inplace_quick_sort_indices(&array, &mut indices, 0, 300);

            assert_eq!(as_vec(&indices), expected, "width {width:?}");
        }
    }

    /// 1,000 values, upstream's "sanity test" made deterministic.
    #[test]
    fn sanity_sorts_a_thousand_values() {
        let mut data: Vec<i64> = (0..1000).map(|k: i64| (k * 7919) % 1009).collect();
        let mut expected = data.clone();
        expected.sort_unstable();

        inplace_quick_sort(&mut data, 0, 1000);

        assert_eq!(data, expected);
    }
}

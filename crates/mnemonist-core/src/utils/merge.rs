//! Port of upstream `utils/merge.js` (mnemonist v0.40.4).
//!
//! Six algorithms over sorted array-likes: a two-array merge and its k-way
//! generalisation, the same pair for a unique-array union, and the same pair
//! for a unique-array intersection. Upstream exports three variadic wrappers
//! (`merge`, `unionUnique`, `intersectionUnique`) that dispatch on
//! `arguments.length`; that dispatch is a JS-arity question and lives at the
//! boundary (`mnemonist-napi`), so this module exposes the two-array and
//! k-way halves directly.
//!
//! # Scope note: this is infrastructure, not a unit
//!
//! Same standing as [`crate::utils::binary_search`] and
//! [`crate::utils::hash_tables`]: there is no `test/merge.js`, only three
//! `describe('merge', ...)` blocks inside `test/_utils.js`, whose
//! require-closure also needs `iterables` (JS-value, ported at the boundary)
//! and cannot run until every sibling exists. It is a member of the eventual
//! `_utils` unit.
//!
//! # `!==` here, "neither greater nor less" in `binary_search`
//!
//! [`crate::utils::binary_search`] computes equality as the *else* arm of an
//! `if (a > b) ... else if (a < b) ... else`, which for `NaN` reports a match
//! (every comparison with `NaN` is `false`, so the chain falls to `else`).
//! This file's dedup checks are a different upstream operator —
//! `array[array.length - 1] !== aHead`, a literal `!==` — and for `NaN` that
//! is `true` (`NaN !== NaN` in JavaScript, same as `NaN != NaN` in Rust).
//! `T: PartialOrd` carries `PartialEq` as a supertrait, so `==`/`!=` here are
//! used directly rather than through the "neither less nor greater" trick,
//! and they reproduce both operators' upstream semantics exactly, including
//! where the two disagree on `NaN`.
//!
//! # BUG-UTILS-1 — the k-way algorithms drop entries when filtering removes any
//!
//! `kWayMergeArrays` and `kWayUnionUniqueArrays` build a `filtered` array that
//! skips empty inputs, then reassign `arrays = filtered` — but the loop that
//! seeds the priority heap keeps using `l`, which was captured from
//! `arrays.length` **before** the reassignment:
//!
//! ```js
//! for (i = 0, l = arrays.length; i < l; i++) { /* ... fills `filtered` ... */ }
//! // ...
//! arrays = filtered;
//! // ...
//! for (i = 0; i < l; i++)   // `l` is the ORIGINAL length, not `filtered.length`
//!   heap.push(i);
//! ```
//!
//! Whenever an input was empty (so `filtered.length < l`), the heap receives
//! indices past the end of `arrays` (now `filtered`). The first `heap.pop()`
//! that touches one of them reads `arrays[p]` (`undefined`) and then indexes
//! it, throwing `TypeError: Cannot read properties of undefined (reading
//! 'undefined')`. Verified against Node 24.18.1 with `pm-recon/mnemonist`
//! (the same v0.40.4 checkout), e.g.
//! `merge.merge([], [1, 2, 3], [4, 5, 6], [4, 7])` and
//! `merge.unionUnique([1, 2], [], [3, 4], [5, 6])` both throw; the *two-array*
//! path and `intersectionUnique`'s k-way fold (which returns `[]` on the
//! first empty array, before any heap exists) are both immune. Recorded as
//! BUG-UTILS-1.
//!
//! Not one case in `test/_utils.js`'s own `'should properly merge k arrays.'`
//! /`'should properly perform the union of k unique arrays.'` blocks includes
//! an empty array alongside two-or-more non-empty ones, so gate 4 cannot
//! reach this. [`merge_k`] and [`union_unique_k`] reproduce it as
//! [`KWayError::StaleLengthMismatch`] rather than a panic — `mnemonist-core`
//! has no exceptions, so the divergence is a `Result`, matching the
//! `TABLE_IS_FULL` convention in [`crate::utils::hash_tables`].
//!
//! # DIV-UTILS-2 CLOSED — the k-way scan now drives a real `FibonacciHeap`
//!
//! Upstream's `kWayMergeArrays`/`kWayUnionUniqueArrays` construct
//! `new FibonacciHeap(function (a, b) { a = arrays[a][pointers[a]]; b =
//! arrays[b][pointers[b]]; ... })` — a heap over **array indices**, keyed by
//! each array's *current* head value through a shared, mutable `pointers`
//! array the comparator closure reads fresh on every call. `fibonacci-heap`
//! is now ported (`crate::structures::fibonacci_heap`), so `k_way_scan`
//! below is the same algorithm, not a substitute for it: `KWayKeyComparator`
//! is that exact closure, and the loop is upstream's `while (heap.size) { p =
//! heap.pop(); ...; if (pointers[p] < arrays[p].length) heap.push(p); }`
//! verbatim.
//!
//! This closes DIV-UTILS-2 (`docs/modules/_utils.md`): a minimum head picked by a
//! **linear scan** that kept the earliest array on a tie, where upstream's
//! heap updates `min` with `<=` — favouring the most recently *pushed* node,
//! which after `consolidate`'s degree-bucket restructuring is not simply "the
//! last array". Confirmed via the differential fuzzer with the widened
//! grammar (ties and `NaN` both reinstated in the k-way generator — see
//! `crates/difffuzz/src/modules/_utils.rs`): zero divergences, where the
//! linear-scan cut disagreed with upstream inside its first few hundred
//! generated cases on exactly this shape (`merge([3], [2, -5], [2])`, see
//! `docs/modules/log/_utils.md` and the pre-fix campaign note in
//! `fuzz/log.txt`).

use std::cell::RefCell;

use super::binary_search::{lower_bound, upper_bound};
use crate::structures::fibonacci_heap::FibonacciHeap;
use crate::utils::comparators::{Comparator, Thrown};

/// Merge two sorted slices into one, preserving every duplicate.
///
/// `mergeArrays(a, b)`. The non-overlapping fast path
/// (`aEnd <= bStart` -> concatenate) is upstream's own optimisation and is
/// kept because it changes nothing observable, only work done.
pub fn merge_two<T: Clone + PartialOrd>(a: &[T], b: &[T]) -> Vec<T> {
    if a.is_empty() {
        return b.to_vec();
    }
    if b.is_empty() {
        return a.to_vec();
    }

    let (a, b) = if a[0] > b[0] { (b, a) } else { (a, b) };

    let b_start = &b[0];
    let a_end = &a[a.len() - 1];

    if a_end <= b_start {
        let mut out = Vec::with_capacity(a.len() + b.len());
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        return out;
    }

    let mut out = Vec::with_capacity(a.len() + b.len());
    let mut a_pointer = 0;

    while a_pointer < a.len() && a[a_pointer] <= *b_start {
        out.push(a[a_pointer].clone());
        a_pointer += 1;
    }

    let mut b_pointer = 0;

    while a_pointer < a.len() && b_pointer < b.len() {
        if a[a_pointer] <= b[b_pointer] {
            out.push(a[a_pointer].clone());
            a_pointer += 1;
        } else {
            out.push(b[b_pointer].clone());
            b_pointer += 1;
        }
    }

    out.extend_from_slice(&a[a_pointer..]);
    out.extend_from_slice(&b[b_pointer..]);

    out
}

/// Union of two sorted, duplicate-free slices, itself sorted and
/// duplicate-free.
///
/// `unionUniqueArrays(a, b)`. Note the fast-path test is strict `<`, not
/// `<=` as in [`merge_two`] — upstream's own inconsistency between the two
/// functions, reproduced rather than harmonised.
pub fn union_unique_two<T: Clone + PartialOrd>(a: &[T], b: &[T]) -> Vec<T> {
    if a.is_empty() {
        return b.to_vec();
    }
    if b.is_empty() {
        return a.to_vec();
    }

    let (a, b) = if a[0] > b[0] { (b, a) } else { (a, b) };

    let b_start = &b[0];
    let a_end = &a[a.len() - 1];

    if a_end < b_start {
        let mut out = Vec::with_capacity(a.len() + b.len());
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        return out;
    }

    let mut out: Vec<T> = Vec::new();
    let mut a_pointer = 0;

    // Unconditional push -- upstream's own prefix loop has NO dedup check,
    // unlike the overlap and filling loops below. It relies on the precondition
    // that `a` is already internally unique; when a caller violates that (an
    // "awkward value" no test here or upstream reaches), consecutive
    // duplicates in this prefix survive into the output. Calling `push_unique`
    // here would be *more correct* than upstream and therefore a defect under
    // this port's bug-for-bug mandate. Differential fuzzing distinguishes the
    // two, on
    // `unionUnique([-5, -5, 0], [-0.5])`; reading the code does not.
    while a_pointer < a.len() && a[a_pointer] < *b_start {
        out.push(a[a_pointer].clone());
        a_pointer += 1;
    }

    let mut b_pointer = 0;

    while a_pointer < a.len() && b_pointer < b.len() {
        if a[a_pointer] <= b[b_pointer] {
            push_unique(&mut out, a[a_pointer].clone());
            a_pointer += 1;
        } else {
            push_unique(&mut out, b[b_pointer].clone());
            b_pointer += 1;
        }
    }

    while a_pointer < a.len() {
        push_unique(&mut out, a[a_pointer].clone());
        a_pointer += 1;
    }

    while b_pointer < b.len() {
        push_unique(&mut out, b[b_pointer].clone());
        b_pointer += 1;
    }

    out
}

/// `if (array.length === 0 || array[array.length - 1] !== value) array.push(value);`
fn push_unique<T: Clone + PartialEq>(out: &mut Vec<T>, value: T) {
    if out.last() != Some(&value) {
        out.push(value);
    }
}

/// Intersection of two sorted, duplicate-free slices.
///
/// `exports.intersectionUniqueArrays` upstream — directly exported there,
/// unlike [`merge_two`]/[`union_unique_two`]'s private counterparts, kept
/// private here too since nothing outside this module calls it by that name.
pub fn intersection_unique_two<T: Clone + PartialOrd>(a: &[T], b: &[T]) -> Vec<T> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }

    let (a, b) = if a[0] > b[0] { (b, a) } else { (a, b) };

    let b_start = &b[0];
    let a_end = &a[a.len() - 1];

    if a_end < b_start {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut a_pointer = lower_bound(a, b_start, None, None);
    let mut b_pointer = 0;
    let b_limit = upper_bound(b, a_end, None, None);

    while a_pointer < a.len() && b_pointer < b_limit {
        let a_head = &a[a_pointer];
        let b_head = &b[b_pointer];

        if *a_head < *b_head {
            a_pointer = lower_bound(a, b_head, Some(a_pointer + 1), None);
        } else if *a_head > *b_head {
            b_pointer = lower_bound(b, a_head, Some(b_pointer + 1), None);
        } else {
            out.push(a_head.clone());
            a_pointer += 1;
            b_pointer += 1;
        }
    }

    out
}

/// The message upstream's k-way merge/union throws, verbatim (BUG-UTILS-1).
pub const STALE_LENGTH_TYPE_ERROR: &str =
    "Cannot read properties of undefined (reading 'undefined')";

/// The one failure mode of the k-way merge/union algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KWayError {
    /// BUG-UTILS-1: at least one input was empty (and therefore filtered out)
    /// while three or more remained live, which upstream cannot survive.
    StaleLengthMismatch,
}

/// Merge `k` sorted array-likes into one.
///
/// `kWayMergeArrays(arrays)`. `arrays` is the *unfiltered* input list, exactly
/// as upstream receives its `arguments` object -- filtering empties out is
/// this function's own first step, and BUG-UTILS-1 is a property of that step, not
/// something a caller can dodge by pre-filtering differently than upstream
/// does.
///
/// # Errors
///
/// [`KWayError::StaleLengthMismatch`] -- see the module docs, BUG-UTILS-1.
pub fn merge_k<T: Clone + PartialOrd>(arrays: &[&[T]]) -> Result<Vec<T>, KWayError> {
    let original_len = arrays.len();
    let filtered: Vec<&[T]> = arrays.iter().copied().filter(|a| !a.is_empty()).collect();

    match filtered.len() {
        0 => return Ok(Vec::new()),
        1 => return Ok(filtered[0].to_vec()),
        2 => return Ok(merge_two(filtered[0], filtered[1])),
        _ => {}
    }

    if original_len != filtered.len() {
        return Err(KWayError::StaleLengthMismatch);
    }

    Ok(k_way_scan(&filtered, |out, value| out.push(value)))
}

/// Union of `k` sorted, duplicate-free array-likes.
///
/// `kWayUnionUniqueArrays(arrays)`, same contract and same BUG-UTILS-1 as
/// [`merge_k`].
///
/// # Errors
///
/// [`KWayError::StaleLengthMismatch`] -- see the module docs, BUG-UTILS-1.
pub fn union_unique_k<T: Clone + PartialOrd>(arrays: &[&[T]]) -> Result<Vec<T>, KWayError> {
    let original_len = arrays.len();
    let filtered: Vec<&[T]> = arrays.iter().copied().filter(|a| !a.is_empty()).collect();

    match filtered.len() {
        0 => return Ok(Vec::new()),
        1 => return Ok(filtered[0].to_vec()),
        2 => return Ok(union_unique_two(filtered[0], filtered[1])),
        _ => {}
    }

    if original_len != filtered.len() {
        return Err(KWayError::StaleLengthMismatch);
    }

    Ok(k_way_scan(&filtered, push_unique))
}

/// The comparator upstream's k-way algorithms build inline:
///
/// ```js
/// new FibonacciHeap(function (a, b) {
///   a = arrays[a][pointers[a]];
///   b = arrays[b][pointers[b]];
///   if (a < b) return -1;
///   if (a > b) return 1;
///   return 0;
/// });
/// ```
///
/// The heap stores **array indices** (`usize`), not values; `pointers` is
/// read fresh on every call, exactly as the JS closure over a shared,
/// mutable array is, so a comparison always sees each array's *current*
/// head. `Thrown` because native comparisons here never fail — `T:
/// PartialOrd`'s `<`/`>` on two Rust values cannot throw the way `ToPrimitive`
/// on two JS values can (see `crate::utils::comparators`'s own docs on why
/// the bridge needs `Operand` where this needs nothing).
struct KWayKeyComparator<'a, T> {
    arrays: &'a [&'a [T]],
    pointers: &'a RefCell<Vec<usize>>,
}

impl<T: PartialOrd> Comparator<usize, Thrown> for KWayKeyComparator<'_, T> {
    fn compare(&self, a: &usize, b: &usize) -> Result<f64, Thrown> {
        let pointers = self.pointers.borrow();
        let a_head = &self.arrays[*a][pointers[*a]];
        let b_head = &self.arrays[*b][pointers[*b]];

        if a_head < b_head {
            return Ok(-1.0);
        }

        if a_head > b_head {
            return Ok(1.0);
        }

        Ok(0.0)
    }
}

/// `kWayMergeArrays`/`kWayUnionUniqueArrays`'s shared body, once the
/// stale-length check (BUG-UTILS-1) has passed: seed a [`FibonacciHeap`] with one
/// entry per array, then repeatedly pop the index whose current head is
/// smallest, hand its value to `sink`, and re-push that index if its array
/// has more left — upstream's own `while (heap.size) { p = heap.pop(); v =
/// arrays[p][pointers[p]++]; ...; if (pointers[p] < arrays[p].length)
/// heap.push(p); }`, verbatim.
fn k_way_scan<T, F>(arrays: &[&[T]], mut sink: F) -> Vec<T>
where
    T: Clone + PartialOrd,
    F: FnMut(&mut Vec<T>, T),
{
    let total: usize = arrays.iter().map(|a| a.len()).sum();
    let pointers = RefCell::new(vec![0usize; arrays.len()]);
    let comparator = KWayKeyComparator {
        arrays,
        pointers: &pointers,
    };
    let heap: FibonacciHeap<usize, _, Thrown> = FibonacciHeap::new(comparator);

    for index in 0..arrays.len() {
        heap.push(index)
            .expect("KWayKeyComparator over Rust PartialOrd never fails");
    }

    let mut out = Vec::with_capacity(total);

    while heap.size() > 0 {
        let index = heap
            .pop()
            .expect("KWayKeyComparator over Rust PartialOrd never fails")
            .expect("heap.size() > 0 guarantees pop() yields Some");

        let value = arrays[index][pointers.borrow()[index]].clone();

        pointers.borrow_mut()[index] += 1;
        sink(&mut out, value);

        if pointers.borrow()[index] < arrays[index].len() {
            heap.push(index)
                .expect("KWayKeyComparator over Rust PartialOrd never fails");
        }
    }

    out
}

/// Intersection of `k` sorted, duplicate-free array-likes.
///
/// `exports.kWayIntersectionUniqueArrays` -- directly exported upstream, and
/// the one k-way algorithm that never touches a heap: it folds
/// [`intersection_unique_two`]'s binary-search walk left to right over
/// `arrays[1..]`, seeded from `arrays[0]`, bailing out the moment the running
/// intersection is empty. No stale-length variable exists to go wrong, and
/// empirically (Node 24.18.1) this is the one of the three k-way functions
/// that does not throw when an input is empty -- it returns `[]` before ever
/// reaching the fold, which is *why* BUG-UTILS-1 does not apply here.
///
/// # Stated divergence: `NaN` at the very first scanned bound
///
/// Upstream seeds `maxStart`/`minEnd` from the JS sentinels `-Infinity`/
/// `Infinity`, so `first > maxStart` (and symmetrically for `minEnd`) is
/// false whenever `first` is `NaN`, leaving the sentinel in place until a
/// later, non-`NaN` array supplies a real bound. This port seeds from
/// `Option<T>`, so the *first* array scanned always sets the accumulator,
/// `NaN` included -- there is no generic `T`-shaped `-Infinity` to seed from
/// without a sentinel trait (`crate::utils::comparators`-style), and nothing
/// in `test/_utils.js` exercises `NaN` here at all. Untested upstream and
/// unfuzzed here for the same reason; recorded rather than silently
/// papered over.
pub fn intersection_unique_k<T: Clone + PartialOrd>(arrays: &[&[T]]) -> Vec<T> {
    if arrays.is_empty() {
        return Vec::new();
    }

    let mut max_start: Option<T> = None;
    let mut min_end: Option<T> = None;

    for array in arrays {
        // Upstream checks `al === 0` (and returns `[]`) before ever reading
        // `arrays[i][0]` -- for *every* array, including the first. Reading
        // `arrays[0][0]` up front, before this loop runs, would panic on an
        // empty first array instead of reporting the empty intersection.
        if array.is_empty() {
            return Vec::new();
        }

        let first = &array[0];
        let last = &array[array.len() - 1];

        max_start = Some(match max_start {
            Some(current) if current >= *first => current,
            _ => first.clone(),
        });
        min_end = Some(match min_end {
            Some(current) if current <= *last => current,
            _ => last.clone(),
        });
    }

    // `arrays` was checked non-empty above, so the loop ran at least once and
    // both accumulators are `Some`.
    let max_start = max_start.expect("at least one array was scanned");
    let min_end = min_end.expect("at least one array was scanned");

    if max_start > min_end {
        return Vec::new();
    }
    if max_start == min_end {
        return vec![max_start];
    }

    let mut current: Vec<T> = arrays[0].to_vec();
    let mut start = max_start;

    for array in &arrays[1..] {
        let a = current;
        let b = *array;
        let mut next = Vec::new();

        let mut a_pointer = 0;
        let mut b_pointer = lower_bound(b, &start, None, None);
        let a_limit = a.len();
        let b_limit = b.len();

        while a_pointer < a_limit && b_pointer < b_limit {
            let a_head = &a[a_pointer];
            let b_head = &b[b_pointer];

            if *a_head < *b_head {
                a_pointer = lower_bound(&a, b_head, Some(a_pointer + 1), None);
            } else if *a_head > *b_head {
                b_pointer = lower_bound(b, a_head, Some(b_pointer + 1), None);
            } else {
                next.push(a_head.clone());
                a_pointer += 1;
                b_pointer += 1;
            }
        }

        if next.is_empty() {
            return next;
        }

        start = next[0].clone();
        current = next;
    }

    current
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------- merge_two

    /// Upstream's own `#.merge` two-array cases, transcribed from
    /// `test/_utils.js` -- which cannot run yet, because its require-closure
    /// needs `iterables`.
    #[test]
    fn merge_two_matches_the_upstream_suites_own_cases() {
        let cases: &[(&[i32], &[i32], &[i32])] = &[
            (&[1, 2, 3], &[], &[1, 2, 3]),
            (&[], &[1, 2, 3], &[1, 2, 3]),
            (&[], &[], &[]),
            (&[1, 2, 3], &[4, 5, 6], &[1, 2, 3, 4, 5, 6]),
            (&[4, 5, 6], &[1, 2, 3], &[1, 2, 3, 4, 5, 6]),
            (&[1, 2, 2, 3], &[2, 3, 3, 4], &[1, 2, 2, 2, 3, 3, 3, 4]),
        ];

        for (a, b, expected) in cases {
            assert_eq!(merge_two(a, b), *expected, "merge_two({a:?}, {b:?})");
        }
    }

    /// Upstream's own `#.merge` k-array cases.
    #[test]
    fn merge_k_matches_the_upstream_suites_own_cases() {
        let empties: [&[i32]; 4] = [&[], &[], &[], &[]];
        assert_eq!(merge_k(&empties), Ok(Vec::new()));

        let a = [1, 2, 3];
        let b = [4, 5, 6];
        let c = [1, 2, 3];
        let d = [4, 7];
        let arrays: [&[i32]; 4] = [&a, &b, &c, &d];

        assert_eq!(merge_k(&arrays), Ok(vec![1, 1, 2, 2, 3, 3, 4, 4, 5, 6, 7]));
    }

    // ------------------------------------------------------- union_unique

    #[test]
    fn union_unique_two_matches_the_upstream_suites_own_cases() {
        let cases: &[(&[i32], &[i32], &[i32])] = &[
            (&[1, 2, 3], &[], &[1, 2, 3]),
            (&[], &[1, 2, 3], &[1, 2, 3]),
            (&[], &[], &[]),
            (&[1, 2, 3], &[4, 5, 6], &[1, 2, 3, 4, 5, 6]),
            (&[4, 5, 6], &[1, 2, 3], &[1, 2, 3, 4, 5, 6]),
            (&[1, 2], &[2, 4], &[1, 2, 4]),
            (&[1, 2, 3, 4, 5], &[2, 3, 4, 5, 6], &[1, 2, 3, 4, 5, 6]),
        ];

        for (a, b, expected) in cases {
            assert_eq!(
                union_unique_two(a, b),
                *expected,
                "union_unique_two({a:?}, {b:?})"
            );
        }
    }

    /// Found by differential fuzzing (seed 42, this unit's first campaign),
    /// not by reading: upstream's prefix loop in `unionUniqueArrays` has NO
    /// dedup check, unlike its overlap and filling loops, so an internally
    /// non-unique first argument leaks a duplicate straight into the output.
    /// Calling the shared `push_unique` helper in the prefix loop too would be
    /// more correct than upstream, and therefore a port defect under this
    /// port's bug-for-bug mandate, not an improvement. Verified against Node
    /// 24.18.1.
    #[test]
    fn the_prefix_loop_does_not_deduplicate_an_already_non_unique_input() {
        assert_eq!(
            union_unique_two(&[-5, -5, 0], &[0]), // -0.5 is not an i32; 0 exercises the same prefix
            vec![-5, -5, 0]
        );

        // The float case actually found by the fuzzer.
        assert_eq!(
            union_unique_two(&[-5.0, -5.0, 0.0], &[-0.5]),
            vec![-5.0, -5.0, -0.5, 0.0]
        );
    }

    #[test]
    fn union_unique_k_matches_the_upstream_suites_own_cases() {
        let empties: [&[i32]; 4] = [&[], &[], &[], &[]];
        assert_eq!(union_unique_k(&empties), Ok(Vec::new()));

        let a = [1, 2, 3];
        let b = [4, 5, 6];
        let c = [1, 2, 3];
        let d = [4, 7];
        let arrays: [&[i32]; 4] = [&a, &b, &c, &d];

        assert_eq!(union_unique_k(&arrays), Ok(vec![1, 2, 3, 4, 5, 6, 7]));
    }

    // -------------------------------------------------- intersection_unique

    #[test]
    fn intersection_unique_two_matches_the_upstream_suites_own_cases() {
        let cases: &[(&[i32], &[i32], &[i32])] = &[
            (&[1, 2, 3], &[], &[]),
            (&[], &[1, 2, 3], &[]),
            (&[], &[], &[]),
            (&[1, 2, 3], &[4, 5, 6], &[]),
            (&[4, 5, 6], &[1, 2, 3], &[]),
            (&[1, 2], &[2, 4], &[2]),
            (&[1, 2, 3, 4, 5], &[2, 3, 4, 5, 6], &[2, 3, 4, 5]),
            (&[1, 2, 3, 4], &[2, 4, 6], &[2, 4]),
        ];

        for (a, b, expected) in cases {
            assert_eq!(
                intersection_unique_two(a, b),
                *expected,
                "intersection_unique_two({a:?}, {b:?})"
            );
        }
    }

    #[test]
    fn intersection_unique_k_matches_the_upstream_suites_own_cases() {
        let empties: [&[i32]; 4] = [&[], &[], &[], &[]];
        assert_eq!(intersection_unique_k(&empties), Vec::<i32>::new());

        let a = [1, 2, 3];
        let b = [4, 5, 6];
        let c = [1, 2, 3];
        let d = [4, 7];
        let arrays: [&[i32]; 4] = [&a, &b, &c, &d];
        assert_eq!(intersection_unique_k(&arrays), Vec::<i32>::new());

        let e = [1, 2];
        let f = [3, 4];
        let g = [5, 6];
        let h = [7, 8];
        let arrays2: [&[i32]; 4] = [&e, &f, &g, &h];
        assert_eq!(intersection_unique_k(&arrays2), Vec::<i32>::new());

        let i1 = [1, 2, 3];
        let i2 = [3, 4, 5];
        let i3 = [1, 3, 4];
        let i4 = [3, 567];
        let i5 = [-14, 3];
        let arrays3: [&[i32]; 5] = [&i1, &i2, &i3, &i4, &i5];
        assert_eq!(intersection_unique_k(&arrays3), vec![3]);

        let j1 = [1, 2, 3, 4];
        let j2 = [3, 4, 5];
        let j3 = [1, 3, 4];
        let j4 = [3, 4, 567];
        let j5 = [-14, 3, 4];
        let arrays4: [&[i32]; 5] = [&j1, &j2, &j3, &j4, &j5];
        assert_eq!(intersection_unique_k(&arrays4), vec![3, 4]);
    }

    // -------------------------------------------------------------- DIV-UTILS-2

    /// The exact case that found DIV-UTILS-2 (`docs/modules/_utils.md`): three arrays, one of them (`[2, -5]`) genuinely
    /// unsorted -- upstream never validates sortedness, and this is where a
    /// tie-break disagreement actually shows up in the output, not merely in
    /// theory. Traced against upstream's real algorithm by hand (`push(0)`,
    /// `push(1)`, `push(2)`; the third push ties `arrays[1][0] == arrays[2][0]
    /// == 2` and `FibonacciHeap.push`'s `<=` tie-break makes the JUST-pushed
    /// index 2 win, so index 2's lone element pops first, then index 1 pops
    /// its own tied `2`, exposing its unsorted second element `-5` next, and
    /// only then index 0's `3`): upstream's real output is `[2, 2, -5, 3]`,
    /// which is what this test pins now that DIV-UTILS-2 is closed. A
    /// linear-scan substitute for the heap yields `[2, -5, 2, 3]` instead --
    /// see this test's sibling
    /// `ties_across_arrays_do_not_affect_the_merged_multiset` for why that
    /// only matters once a tie meets an unsorted array.
    #[test]
    fn merge_k_matches_upstreams_real_heap_on_the_case_that_found_div_utils_2() {
        let a: [i32; 1] = [3];
        let b: [i32; 2] = [2, -5];
        let c: [i32; 1] = [2];
        let arrays: [&[i32]; 3] = [&a, &b, &c];

        assert_eq!(merge_k(&arrays), Ok(vec![2, 2, -5, 3]));
    }

    // ---------------------------------------------------------------- BUG-UTILS-1

    /// BUG-UTILS-1, isolated at its sharpest: three non-empty arrays plus one empty
    /// one. `filtered.len() == 3` (still the heap path) but the original
    /// argument count was 4, so upstream's stale `l` seeds the heap with one
    /// index too many and crashes. `test/_utils.js`'s own k-array cases never
    /// mix an empty array with two-or-more non-empty ones, so gate 4 cannot
    /// reach this -- verified empirically against Node 24.18.1 instead (see
    /// the module docs).
    #[test]
    fn merge_k_reproduces_b_180_when_filtering_drops_the_length() {
        let empty: [i32; 0] = [];
        let a = [1, 2, 3];
        let b = [4, 5, 6];
        let c = [4, 7];
        let arrays: [&[i32]; 4] = [&empty, &a, &b, &c];

        assert_eq!(merge_k(&arrays), Err(KWayError::StaleLengthMismatch));
    }

    #[test]
    fn union_unique_k_reproduces_b_180_when_filtering_drops_the_length() {
        let a = [1, 2];
        let empty: [i32; 0] = [];
        let b = [3, 4];
        let c = [5, 6];
        let arrays: [&[i32]; 4] = [&a, &empty, &b, &c];

        assert_eq!(union_unique_k(&arrays), Err(KWayError::StaleLengthMismatch));
    }

    /// The one place BUG-UTILS-1 does NOT reach: filtering down to exactly two or
    /// fewer live arrays takes the early-return branches, which run *before*
    /// `arrays = filtered` and the stale `l` are ever consulted.
    #[test]
    fn filtering_down_to_two_or_fewer_never_reaches_the_bug() {
        let empty: [i32; 0] = [];
        let a = [1, 2, 3];

        let one_and_empties: [&[i32]; 3] = [&empty, &a, &empty];
        assert_eq!(merge_k(&one_and_empties), Ok(vec![1, 2, 3]));

        let b = [4, 5];
        let two_and_an_empty: [&[i32]; 3] = [&empty, &a, &b];
        assert_eq!(merge_k(&two_and_an_empty), Ok(vec![1, 2, 3, 4, 5]));
    }

    /// `intersection_unique_k` is immune: it returns `[]` on the first empty
    /// array, before the fold (and before any stale-length variable) exists.
    #[test]
    fn intersection_unique_k_is_immune_to_bug_utils_1() {
        let empty: [i32; 0] = [];
        let a = [1, 2, 3];
        let b = [4, 5, 6];
        let c = [4, 7];
        let arrays: [&[i32]; 4] = [&empty, &a, &b, &c];

        assert_eq!(intersection_unique_k(&arrays), Vec::<i32>::new());
    }

    // ---------------------------------------------------------------- gaps
    //
    // Coverage upstream's `test/_utils.js` does not have.

    /// `NaN`'s two operators disagree, and this file uses both: the merges'
    /// `<=`/`<` swaps (`greater`) treat `NaN` as losing every comparison
    /// (matching `binary_search`), while the unions' dedup check uses `!==`,
    /// under which `NaN !== NaN` is `true`. So a run of `NaN`s merges as an
    /// unsorted-looking run (every comparison false, so nothing ever
    /// "wins"), while the same run unions without ever deduplicating.
    #[test]
    fn nan_is_never_deduplicated_by_the_union_dedup_check() {
        let a = [f64::NAN, f64::NAN];
        let b = [f64::NAN];

        let result = union_unique_two(&a, &b);
        assert_eq!(result.len(), 3);
        assert!(result.iter().all(|v| v.is_nan()));
    }

    /// A tie among IDENTICAL values (every element here is `3`) is
    /// unobservable in the merge's flat output regardless of which
    /// implementation breaks it: whichever array's `3` the heap pops first,
    /// the emitted value is still `3`, so the tie-break choice cannot change
    /// what a caller sees. This is NOT the same claim as "ties never matter"
    /// -- see the module docs' DIV-UTILS-2 section: once tied *unsorted* arrays
    /// interleave with distinct later values, which array wins a tie
    /// genuinely changes the output, which is exactly what made the
    /// pre-heap linear-scan cut of this file (DIV-UTILS-2, now closed) diverge
    /// from upstream on `merge([3], [2, -5], [2])`.
    #[test]
    fn ties_across_arrays_do_not_affect_the_merged_multiset() {
        let a = [3, 3, 3];
        let b = [3, 3];
        let c = [3];
        let arrays: [&[i32]; 3] = [&a, &b, &c];

        assert_eq!(merge_k(&arrays), Ok(vec![3, 3, 3, 3, 3, 3]));
    }

    /// A single non-empty argument (`arguments.length` need not be 2 or more
    /// for upstream to reach `kWayMergeArrays`): `filtered.len() == 1` takes
    /// the early return, which is a plain copy.
    #[test]
    fn a_single_array_is_copied_not_referenced() {
        let a = [1, 2, 3];
        let arrays: [&[i32]; 1] = [&a];

        assert_eq!(merge_k(&arrays), Ok(vec![1, 2, 3]));
        assert_eq!(union_unique_k(&arrays), Ok(vec![1, 2, 3]));
    }

    /// Zero arrays: unreachable through the real variadic dispatcher (which
    /// gates on `isArrayLike(arguments[0])` before ever calling in), but
    /// total here rather than panicking.
    #[test]
    fn zero_arrays_is_empty_not_a_panic() {
        let arrays: [&[i32]; 0] = [];

        assert_eq!(merge_k(&arrays), Ok(Vec::new()));
        assert_eq!(union_unique_k(&arrays), Ok(Vec::new()));
        assert_eq!(intersection_unique_k(&arrays), Vec::<i32>::new());
    }
}

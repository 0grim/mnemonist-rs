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
//! # B-180 — the k-way algorithms drop entries when filtering removes any
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
//! NOTES.md B-180.
//!
//! Not one case in `test/_utils.js`'s own `'should properly merge k arrays.'`
//! /`'should properly perform the union of k unique arrays.'` blocks includes
//! an empty array alongside two-or-more non-empty ones, so gate 4 cannot
//! reach this. [`merge_k`] and [`union_unique_k`] reproduce it as
//! [`KWayError::StaleLengthMismatch`] rather than a panic — `mnemonist-core`
//! has no exceptions, so the divergence is a `Result`, matching the
//! `TABLE_IS_FULL` convention in [`crate::utils::hash_tables`].
//!
//! # The heap is a linear scan, not a `FibonacciHeap`
//!
//! Upstream's k-way algorithms drive a `FibonacciHeap` keyed on the arrays'
//! current head values. `fibonacci-heap.js` is not ported (T2 in
//! `planning/ROADMAP.md`), so this file picks the minimum head by a linear
//! scan over the live arrays instead. The two are observably identical for
//! every case reachable here: a tie between two arrays' heads is broken
//! arbitrarily by *both* implementations, and the only place a tie is
//! observable is the union's "differs from the last pushed value" dedup
//! check, which compares against the immediately preceding *value*, not
//! against which array supplied it. Stated as a divergence rather than
//! elided, because it is a real algorithmic substitution, not "the same
//! algorithm in different words" — just one with no path to a different
//! flattened result.

use super::binary_search::{lower_bound, upper_bound};

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

    while a_pointer < a.len() && a[a_pointer] < *b_start {
        push_unique(&mut out, a[a_pointer].clone());
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

/// The message upstream's k-way merge/union throws, verbatim (B-180).
pub const STALE_LENGTH_TYPE_ERROR: &str =
    "Cannot read properties of undefined (reading 'undefined')";

/// The one failure mode of the k-way merge/union algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KWayError {
    /// B-180: at least one input was empty (and therefore filtered out)
    /// while three or more remained live, which upstream cannot survive.
    StaleLengthMismatch,
}

/// Merge `k` sorted array-likes into one.
///
/// `kWayMergeArrays(arrays)`. `arrays` is the *unfiltered* input list, exactly
/// as upstream receives its `arguments` object -- filtering empties out is
/// this function's own first step, and B-180 is a property of that step, not
/// something a caller can dodge by pre-filtering differently than upstream
/// does.
///
/// # Errors
///
/// [`KWayError::StaleLengthMismatch`] -- see the module docs, B-180.
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
/// `kWayUnionUniqueArrays(arrays)`, same contract and same B-180 as
/// [`merge_k`].
///
/// # Errors
///
/// [`KWayError::StaleLengthMismatch`] -- see the module docs, B-180.
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

/// Repeatedly pick the smallest live head across `arrays` and hand it to
/// `sink`, in lockstep, until every array is exhausted.
///
/// The linear scan standing in for upstream's `FibonacciHeap` -- see the
/// module docs for why the substitution is unobservable here.
fn k_way_scan<T, F>(arrays: &[&[T]], mut sink: F) -> Vec<T>
where
    T: Clone + PartialOrd,
    F: FnMut(&mut Vec<T>, T),
{
    let total: usize = arrays.iter().map(|a| a.len()).sum();
    let mut pointers = vec![0usize; arrays.len()];
    let mut out = Vec::with_capacity(total);

    loop {
        let mut best: Option<usize> = None;

        for (index, array) in arrays.iter().enumerate() {
            if pointers[index] >= array.len() {
                continue;
            }

            best = match best {
                None => Some(index),
                Some(current) if array[pointers[index]] < arrays[current][pointers[current]] => {
                    Some(index)
                }
                Some(current) => Some(current),
            };
        }

        match best {
            None => return out,
            Some(index) => {
                let value = arrays[index][pointers[index]].clone();
                pointers[index] += 1;
                sink(&mut out, value);
            }
        }
    }
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
/// reaching the fold, which is *why* B-180 does not apply here.
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

    // ---------------------------------------------------------------- B-180

    /// B-180, isolated at its sharpest: three non-empty arrays plus one empty
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

    /// The one place B-180 does NOT reach: filtering down to exactly two or
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
    fn intersection_unique_k_is_immune_to_b_180() {
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

    /// A tie broken by the linear scan is unobservable in the merge's flat
    /// output: swapping which of two equal-valued arrays "wins" the pick
    /// cannot change the resulting multiset, only which array a given
    /// position's value happened to come from -- and nothing here can tell.
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

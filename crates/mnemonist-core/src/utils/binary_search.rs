//! Port of upstream `utils/binary-search.js` (mnemonist v0.40.4).
//!
//! Seven functions: an exact search and its comparator variant, and lower/upper
//! bounds in plain, comparator and indirect-through-an-index-array flavours.
//!
//! # Scope note: this is infrastructure, not a unit
//!
//! DESIGN.md §1.1 defines a unit as the require-closure of one upstream *test
//! file*. There is no `test/binary-search.js`. The only file that exercises
//! these functions is `test/_utils.js`, whose closure also needs `merge` and
//! `iterables`, neither of which is ported yet — so not one of its assertions
//! can run today. Like `utils/bitwise`, this file therefore gets gates 1, 2, 7
//! and 8 and never appears in `tests/scope.txt` on its own. It is a member of
//! the eventual `_utils` unit, and a dependency of `merge` and `vp-tree`.
//!
//! # Out-of-range reads are part of the contract
//!
//! Every function indexes its haystack with a midpoint derived from caller
//! supplied bounds, and upstream never checks that those bounds are inside the
//! array. In JavaScript an over-long `hi` reads `undefined`, and `undefined`
//! loses **both** comparisons: `undefined > x` and `undefined < x` are each
//! `false`, and so are `x <= undefined` and `x >= undefined`. That is not an
//! error path, it is reachable behaviour with observable consequences:
//!
//! ```text
//! search([1, 2, 3], 9, 0, 100)  ->  49   // "found" at a hole
//! ```
//!
//! So every read below goes through [`Missing`]-aware helpers that return
//! `false` for a comparison against an absent element, which reproduces the
//! JavaScript exactly rather than panicking on a bounds check the original does
//! not have.
//!
//! # Deliberate divergences
//!
//! * **Comparators return [`Ordering`], not a number.** Upstream branches on
//!   `comparison > 0` / `< 0` / else, so the three arms map onto `Ordering`
//!   exactly — with one unrepresentable case: a JavaScript comparator that
//!   returns `NaN` fails both tests and takes the *equal* arm. `Ordering` has
//!   no `NaN`, so a NaN-returning comparator cannot be expressed here. Nothing
//!   in mnemonist ships such a comparator.
//! * **`search` returns `isize` with `-1` for "absent"** rather than
//!   `Option<usize>`, because callers upstream test `!== -1` and a port of
//!   those callers reads more obviously against the same sentinel.
//! * **`(lo + hi) >>> 1` is computed as `(lo + hi) / 2`.** The `>>> 1` also
//!   truncates the sum to 32 bits, which can only matter for an array of at
//!   least 2^31 elements. Unreachable in JavaScript, where such an array cannot
//!   exist, and not reproduced.
//!
//! # An inconsistency worth naming
//!
//! [`search_with_comparator`] calls `comparator(element, value)` while
//! [`lower_bound_with_comparator`] and [`upper_bound_with_comparator`] call
//! `comparator(value, element)`. The argument order is reversed between the two
//! halves of the same file.
//!
//! Measured, not assumed: for any **antisymmetric** comparator the reversal is
//! invisible, because the swap negates the result and each family's branch
//! conditions are negated to match. Both families then agree that "element
//! orders at or after value" means "go left". Every comparator mnemonist ships
//! is antisymmetric, which is why upstream's suite cannot see this. It becomes
//! observable the moment a comparator is not — see the test
//! `the_two_comparator_families_take_their_arguments_in_opposite_orders`, and
//! `docs/modules/utils-binary-search.md`.

use std::cmp::Ordering;

/// Result of `array[i]` where `i` may be past the end: `None` is `undefined`.
type Missing<'a, T> = Option<&'a T>;

/// `element > value`, where an absent element loses.
fn greater<T: PartialOrd>(element: Missing<'_, T>, value: &T) -> bool {
    matches!(element, Some(element) if element > value)
}

/// `element < value`, where an absent element loses.
fn less<T: PartialOrd>(element: Missing<'_, T>, value: &T) -> bool {
    matches!(element, Some(element) if element < value)
}

/// `value <= element`, where an absent element loses.
fn value_le<T: PartialOrd>(value: &T, element: Missing<'_, T>) -> bool {
    matches!(element, Some(element) if value <= element)
}

/// `value >= element`, where an absent element loses.
fn value_ge<T: PartialOrd>(value: &T, element: Missing<'_, T>) -> bool {
    matches!(element, Some(element) if value >= element)
}

/// `array[index]`, with a negative index reading `undefined` as JavaScript does.
fn at<T>(array: &[T], index: isize) -> Missing<'_, T> {
    usize::try_from(index)
        .ok()
        .and_then(|index| array.get(index))
}

/// Index of `value` in a sorted `array`, or `-1`.
///
/// `lo` defaults to `0` and `hi` to `array.len()`; `hi` is *exclusive*, and the
/// first thing upstream does is decrement it.
///
/// Equality here is "neither greater nor less", which is what upstream's
/// `if / else if / else` chain computes. For a total order that is ordinary
/// equality; for `f64`, `NaN` compares false both ways, so a `NaN` at the
/// midpoint is reported as a match. That is upstream's behaviour, verified
/// against Node.
pub fn search<T: PartialOrd>(
    array: &[T],
    value: &T,
    lo: Option<usize>,
    hi: Option<usize>,
) -> isize {
    let mut lo = lo.unwrap_or(0) as isize;
    // `hi--`: the caller's bound is exclusive, the loop's is inclusive.
    let mut hi = hi.unwrap_or(array.len()) as isize - 1;

    while lo <= hi {
        let mid = (lo + hi) / 2;
        let current = at(array, mid);

        if greater(current, value) {
            hi = mid - 1;
        } else if less(current, value) {
            lo = mid + 1;
        } else {
            return mid;
        }
    }

    -1
}

/// [`search`] with a caller-supplied ordering, over the whole array.
///
/// Note the argument order: `comparator(element, value)`. It is the opposite of
/// the bound functions below — see the module docs.
pub fn search_with_comparator<T, F>(comparator: F, array: &[T], value: &T) -> isize
where
    F: Fn(&T, &T) -> Ordering,
{
    let mut lo: isize = 0;
    let mut hi: isize = array.len() as isize - 1;

    while lo <= hi {
        let mid = (lo + hi) / 2;

        // `array[mid]` is in range for every `mid` this loop produces, because
        // `hi` starts at `len - 1` and only ever shrinks. Upstream has no
        // `lo`/`hi` parameters here, so the `undefined` path is unreachable.
        match at(array, mid) {
            None => return mid,
            Some(element) => match comparator(element, value) {
                Ordering::Greater => hi = mid - 1,
                Ordering::Less => lo = mid + 1,
                Ordering::Equal => return mid,
            },
        }
    }

    -1
}

/// First index at which `value` could be inserted keeping `array` sorted.
///
/// Equivalently: the number of elements strictly less than `value` in
/// `lo..hi`, offset by `lo`.
pub fn lower_bound<T: PartialOrd>(
    array: &[T],
    value: &T,
    lo: Option<usize>,
    hi: Option<usize>,
) -> usize {
    let mut lo = lo.unwrap_or(0);
    let mut hi = hi.unwrap_or(array.len());

    while lo < hi {
        let mid = (lo + hi) / 2;

        if value_le(value, at(array, mid as isize)) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }

    lo
}

/// [`lower_bound`] with a caller-supplied ordering, over the whole array.
///
/// Note the argument order: `comparator(value, element)`, the reverse of
/// [`search_with_comparator`].
pub fn lower_bound_with_comparator<T, F>(comparator: F, array: &[T], value: &T) -> usize
where
    F: Fn(&T, &T) -> Ordering,
{
    let mut lo = 0;
    let mut hi = array.len();

    while lo < hi {
        let mid = (lo + hi) / 2;
        let takes_upper_half = match at(array, mid as isize) {
            None => false,
            Some(element) => comparator(value, element) != Ordering::Greater,
        };

        if takes_upper_half {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }

    lo
}

/// [`lower_bound`] over `array` viewed through a sorting permutation.
///
/// `indices` is an argsort of `array`: `array[indices[0]] <= array[indices[1]]`
/// and so on. The returned index is a position in `indices`, not in `array`.
///
/// # Upstream quirk, preserved
///
/// `hi` defaults to **`array.len()`, not `indices.len()`** — the one place in
/// the file where the default bound is taken from the wrong array. When
/// `indices` is shorter, upstream reads `indices[mid]` as `undefined`, then
/// `array[undefined]` as `undefined`, and `value <= undefined` is `false`, so
/// the search moves right. Reproduced exactly; see the module docs.
pub fn lower_bound_indices<T: PartialOrd>(
    array: &[T],
    indices: &[usize],
    value: &T,
    lo: Option<usize>,
    hi: Option<usize>,
) -> usize {
    let mut lo = lo.unwrap_or(0);
    let mut hi = hi.unwrap_or(array.len());

    while lo < hi {
        let mid = (lo + hi) / 2;
        let element = indices.get(mid).and_then(|&index| array.get(index));

        if value_le(value, element) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }

    lo
}

/// Last index at which `value` could be inserted keeping `array` sorted.
///
/// Equivalently: the number of elements less than *or equal to* `value` in
/// `lo..hi`, offset by `lo`.
pub fn upper_bound<T: PartialOrd>(
    array: &[T],
    value: &T,
    lo: Option<usize>,
    hi: Option<usize>,
) -> usize {
    let mut lo = lo.unwrap_or(0);
    let mut hi = hi.unwrap_or(array.len());

    while lo < hi {
        let mid = (lo + hi) / 2;

        if value_ge(value, at(array, mid as isize)) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    lo
}

/// [`upper_bound`] with a caller-supplied ordering, over the whole array.
///
/// Note the argument order: `comparator(value, element)`.
pub fn upper_bound_with_comparator<T, F>(comparator: F, array: &[T], value: &T) -> usize
where
    F: Fn(&T, &T) -> Ordering,
{
    let mut lo = 0;
    let mut hi = array.len();

    while lo < hi {
        let mid = (lo + hi) / 2;
        let takes_lower_half = match at(array, mid as isize) {
            None => false,
            Some(element) => comparator(value, element) != Ordering::Less,
        };

        if takes_lower_half {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    lo
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ascending comparator over integers, the shape every mnemonist caller uses.
    fn ascending(a: &i32, b: &i32) -> Ordering {
        a.cmp(b)
    }

    /// The upstream suite's own `#.search` case, transcribed from
    /// `test/_utils.js` — which cannot run yet, because its require-closure
    /// needs `merge` and `iterables`.
    #[test]
    fn search_matches_the_upstream_suites_own_case() {
        let array = [1, 2, 3, 4, 5];

        for (index, value) in array.iter().enumerate() {
            assert_eq!(search(&array, value, None, None), index as isize);
        }

        assert_eq!(search(&array, &56, None, None), -1);
    }

    /// Upstream's `#.searchWithComparator` case: a *descending* array searched
    /// with a comparator that inverts it.
    #[test]
    fn search_with_comparator_matches_the_upstream_suites_own_case() {
        let array = [5, 4, 3, 2, 1];
        let comparator = |a: &i32, b: &i32| (4 - a).cmp(&(4 - b));

        for (index, value) in array.iter().enumerate() {
            assert_eq!(
                search_with_comparator(comparator, &array, value),
                index as isize
            );
        }

        assert_eq!(search_with_comparator(comparator, &array, &56), -1);
    }

    /// Upstream's `#.lowerBound` and `#.upperBound` cases, on the same array,
    /// so the two are visibly the two ends of the equal run.
    #[test]
    fn bounds_match_the_upstream_suites_own_cases() {
        let array = [1, 2, 3, 3, 3, 4, 4, 5, 5];

        assert_eq!(lower_bound(&array, &56, None, None), array.len());
        assert_eq!(lower_bound(&array, &-4, None, None), 0);
        assert_eq!(lower_bound(&array, &3, None, None), 2);
        assert_eq!(lower_bound(&array, &4, None, None), 5);
        assert_eq!(lower_bound(&array, &1, None, None), 0);
        assert_eq!(lower_bound(&array, &2, None, None), 1);
        assert_eq!(lower_bound(&array, &5, None, None), 7);
        assert_eq!(
            lower_bound(&[1, 2, 3, 4, 5, 5, 5, 6, 7, 9], &8, None, None),
            9
        );

        assert_eq!(upper_bound(&array, &56, None, None), array.len());
        assert_eq!(upper_bound(&array, &-4, None, None), 0);
        assert_eq!(upper_bound(&array, &3, None, None), 5);
        assert_eq!(upper_bound(&array, &4, None, None), 7);
        assert_eq!(upper_bound(&array, &1, None, None), 1);
        assert_eq!(upper_bound(&array, &2, None, None), 2);
        assert_eq!(upper_bound(&array, &5, None, None), 9);
    }

    /// Upstream's `#.lowerBoundWithComparator` / `#.upperBoundWithComparator`
    /// cases: an array of number *names*, ordered by a comparator that resolves
    /// them. The needle is sometimes a name and sometimes a bare number, which
    /// is why upstream's comparator sniffs `typeof`; here the element type is
    /// uniform and the mapping happens inside the comparator instead.
    #[test]
    fn comparator_bounds_match_the_upstream_suites_own_cases() {
        const WORDS: [&str; 9] = [
            "one", "two", "three", "three", "three", "four", "four", "five", "five",
        ];

        fn rank(word: &&str) -> i32 {
            match *word {
                "one" => 1,
                "two" => 2,
                "three" => 3,
                "four" => 4,
                "five" => 5,
                // Upstream's comparator passes numbers straight through; the
                // two out-of-band needles it uses are 56 and -4.
                "56" => 56,
                "-4" => -4,
                other => panic!("unmapped word `{other}`"),
            }
        }

        let comparator = |a: &&str, b: &&str| rank(a).cmp(&rank(b));

        assert_eq!(
            lower_bound_with_comparator(comparator, &WORDS, &"56"),
            WORDS.len()
        );
        assert_eq!(lower_bound_with_comparator(comparator, &WORDS, &"-4"), 0);
        assert_eq!(lower_bound_with_comparator(comparator, &WORDS, &"three"), 2);
        assert_eq!(lower_bound_with_comparator(comparator, &WORDS, &"four"), 5);
        assert_eq!(lower_bound_with_comparator(comparator, &WORDS, &"one"), 0);
        assert_eq!(lower_bound_with_comparator(comparator, &WORDS, &"two"), 1);
        assert_eq!(lower_bound_with_comparator(comparator, &WORDS, &"five"), 7);

        assert_eq!(
            upper_bound_with_comparator(comparator, &WORDS, &"56"),
            WORDS.len()
        );
        assert_eq!(upper_bound_with_comparator(comparator, &WORDS, &"-4"), 0);
        assert_eq!(upper_bound_with_comparator(comparator, &WORDS, &"three"), 5);
    }

    /// Upstream's `#.lowerBoundIndices` case: the indirect bound must agree
    /// with the direct one on the materialised sorted array.
    #[test]
    fn lower_bound_indices_matches_the_upstream_suites_own_case() {
        let array = [3, 6, 2, 5, 1, 0, 15];
        let argsort = [5, 4, 2, 0, 3, 1, 6];
        let sorted = [0, 1, 2, 3, 5, 6, 15];

        for needle in [5, -14, 0, 1, 3, 36, 6758] {
            assert_eq!(
                lower_bound_indices(&array, &argsort, &needle, None, None),
                lower_bound(&sorted, &needle, None, None),
            );
        }
    }

    // ---------------------------------------------------------------- gaps
    //
    // Everything below is coverage upstream's `test/_utils.js` does not have.

    /// Exhaustive agreement with a linear scan, for every array of length 0..=8
    /// over `{0, 1, 2}` and every needle in `-1..=3`. This is the property the
    /// upstream suite checks at seven hand-picked points.
    #[test]
    fn bounds_agree_with_a_linear_scan_exhaustively() {
        fn linear_lower(array: &[i32], value: i32) -> usize {
            array.iter().take_while(|&&x| x < value).count()
        }

        fn linear_upper(array: &[i32], value: i32) -> usize {
            array.iter().take_while(|&&x| x <= value).count()
        }

        let mut array: Vec<i32> = Vec::new();

        for length in 0..=8usize {
            // Every non-decreasing sequence of `length` values over {0, 1, 2},
            // enumerated as the 3^length base-3 numbers, filtered to sorted.
            for encoding in 0..3usize.pow(length as u32) {
                array.clear();
                let mut rest = encoding;

                for _ in 0..length {
                    array.push((rest % 3) as i32);
                    rest /= 3;
                }

                if array.windows(2).any(|pair| pair[0] > pair[1]) {
                    continue;
                }

                for value in -1..=3 {
                    assert_eq!(
                        lower_bound(&array, &value, None, None),
                        linear_lower(&array, value),
                        "lower_bound({array:?}, {value})"
                    );
                    assert_eq!(
                        upper_bound(&array, &value, None, None),
                        linear_upper(&array, value),
                        "upper_bound({array:?}, {value})"
                    );
                    assert_eq!(
                        lower_bound_with_comparator(ascending, &array, &value),
                        linear_lower(&array, value),
                        "lower_bound_with_comparator({array:?}, {value})"
                    );
                    assert_eq!(
                        upper_bound_with_comparator(ascending, &array, &value),
                        linear_upper(&array, value),
                        "upper_bound_with_comparator({array:?}, {value})"
                    );

                    let found = search(&array, &value, None, None);

                    if array.contains(&value) {
                        assert_eq!(array[found as usize], value, "search({array:?}, {value})");
                    } else {
                        assert_eq!(found, -1, "search({array:?}, {value})");
                    }

                    let found = search_with_comparator(ascending, &array, &value);

                    if array.contains(&value) {
                        assert_eq!(array[found as usize], value);
                    } else {
                        assert_eq!(found, -1);
                    }
                }
            }
        }
    }

    /// Empty haystacks. Never exercised upstream, and the one input where
    /// `search`'s `hi--` produces `-1` before the loop runs at all.
    #[test]
    fn empty_arrays() {
        let empty: [i32; 0] = [];

        assert_eq!(search(&empty, &1, None, None), -1);
        assert_eq!(search_with_comparator(ascending, &empty, &1), -1);
        assert_eq!(lower_bound(&empty, &1, None, None), 0);
        assert_eq!(upper_bound(&empty, &1, None, None), 0);
        assert_eq!(lower_bound_with_comparator(ascending, &empty, &1), 0);
        assert_eq!(upper_bound_with_comparator(ascending, &empty, &1), 0);
        assert_eq!(lower_bound_indices(&empty, &[], &1, None, None), 0);
    }

    /// Explicit `lo`/`hi`, which upstream's suite never passes. The window is
    /// honoured, and a needle outside it is not found.
    #[test]
    fn explicit_bounds_window_the_search() {
        let array = [1, 2, 3, 4, 5, 6, 7, 8];

        assert_eq!(search(&array, &3, Some(4), Some(8)), -1);
        assert_eq!(search(&array, &6, Some(4), Some(8)), 5);
        assert_eq!(lower_bound(&array, &1, Some(3), Some(6)), 3);
        assert_eq!(upper_bound(&array, &9, Some(3), Some(6)), 6);
        // An empty window returns its own start, for both bounds.
        assert_eq!(lower_bound(&array, &4, Some(2), Some(2)), 2);
        assert_eq!(upper_bound(&array, &4, Some(2), Some(2)), 2);
        // `hi == 0` makes `search`'s `hi--` negative before the first compare.
        assert_eq!(search(&array, &1, Some(0), Some(0)), -1);
    }

    /// Verified against Node 24.18.1: an over-long `hi` makes `search` report a
    /// hit at a hole, because `undefined` loses both comparisons.
    ///
    /// ```js
    /// require('./utils/binary-search.js').search([1, 2, 3], 9, 0, 100)  // 49
    /// ```
    #[test]
    fn an_over_long_hi_reports_a_hit_at_a_hole() {
        assert_eq!(search(&[1, 2, 3], &9, Some(0), Some(100)), 49);
        // The bounds walk right instead, because `x <= undefined` is false.
        assert_eq!(lower_bound(&[1, 2, 3], &9, Some(0), Some(100)), 100);
        // ...and `x >= undefined` is false too, so `upperBound` walks left.
        assert_eq!(upper_bound(&[1, 2, 3], &9, Some(0), Some(100)), 3);
    }

    /// The `lowerBoundIndices` default-bound quirk, isolated: `hi` comes from
    /// `array`, so a short `indices` is walked off the end and the answer is
    /// the *array's* length rather than a position in `indices`.
    ///
    /// Verified against Node 24.18.1:
    ///
    /// ```js
    /// lowerBoundIndices([0, 1, 2, 3, 4, 5, 6, 7], [0, 1], 1)        // 8
    /// lowerBoundIndices([0, 1, 2, 3, 4, 5, 6, 7], [0, 1], 1, 0, 2)  // 1
    /// ```
    #[test]
    fn lower_bound_indices_defaults_hi_from_the_wrong_array() {
        let array = [0, 1, 2, 3, 4, 5, 6, 7];
        let indices = [0usize, 1];

        assert_eq!(lower_bound_indices(&array, &indices, &1, None, None), 8);
        // Passing the bound the caller meant gives the answer they wanted.
        assert_eq!(
            lower_bound_indices(&array, &indices, &1, Some(0), Some(indices.len())),
            1
        );
    }

    /// `NaN` at the midpoint fails both of `search`'s comparisons and so takes
    /// the "equal" arm. Verified against Node 24.18.1:
    ///
    /// ```js
    /// search([NaN, NaN, NaN], 1)  // 1
    /// ```
    #[test]
    fn nan_is_reported_as_a_match_by_search() {
        let array = [f64::NAN, f64::NAN, f64::NAN];

        assert_eq!(search(&array, &1.0, None, None), 1);
    }

    /// Duplicates: `search` may return any index in the run, and the bounds
    /// return its two ends. Upstream never checks which index `search` picks,
    /// so this pins the actual midpoint arithmetic rather than a range.
    #[test]
    fn duplicates_pin_the_midpoint_arithmetic() {
        let array = [7; 9];

        assert_eq!(search(&array, &7, None, None), 4);
        assert_eq!(lower_bound(&array, &7, None, None), 0);
        assert_eq!(upper_bound(&array, &7, None, None), 9);
    }

    /// An **antisymmetric** comparator cannot see the argument-order
    /// difference: both families agree, which is why upstream's suite never
    /// caught it. Values from Node 24.18.1.
    #[test]
    fn an_antisymmetric_comparator_hides_the_argument_order() {
        let array = [5, 4, 3, 2, 1];
        let descending = |a: &i32, b: &i32| b.cmp(a);

        assert_eq!(search_with_comparator(descending, &array, &3), 2);
        assert_eq!(lower_bound_with_comparator(descending, &array, &3), 2);
        assert_eq!(upper_bound_with_comparator(descending, &array, &3), 3);
    }

    /// The argument-order difference, made executable with a comparator that is
    /// deliberately **not** antisymmetric: it answers "less" whenever its
    /// *first* argument is `0`, whatever the second is. `searchWithComparator`
    /// passes the element first, so `0` is never found; the bound functions
    /// pass the value first, so a needle of `0` collapses to index `0`.
    ///
    /// Verified against Node 24.18.1 with
    /// `function (a, b) { if (a === 0) return -1; return a < b ? -1 : a > b ? 1 : 0; }`
    /// over `[0, 1, 2, 3, 4, 5, 6, 7]`.
    #[test]
    fn the_two_comparator_families_take_their_arguments_in_opposite_orders() {
        let array = [0, 1, 2, 3, 4, 5, 6, 7];
        let lopsided = |a: &i32, b: &i32| {
            if *a == 0 {
                Ordering::Less
            } else {
                a.cmp(b)
            }
        };

        // value = 0: the two families disagree about the same element.
        assert_eq!(search_with_comparator(lopsided, &array, &0), -1);
        assert_eq!(lower_bound_with_comparator(lopsided, &array, &0), 0);
        assert_eq!(upper_bound_with_comparator(lopsided, &array, &0), 0);

        // value = 3 and 7: the special case never fires, so all three agree
        // with the plain versions.
        assert_eq!(search_with_comparator(lopsided, &array, &3), 3);
        assert_eq!(lower_bound_with_comparator(lopsided, &array, &3), 3);
        assert_eq!(upper_bound_with_comparator(lopsided, &array, &3), 4);
        assert_eq!(search_with_comparator(lopsided, &array, &7), 7);
        assert_eq!(lower_bound_with_comparator(lopsided, &array, &7), 7);
        assert_eq!(upper_bound_with_comparator(lopsided, &array, &7), 8);
    }

    /// A non-sorted haystack. Upstream never checks this and the result is not
    /// meaningful, but it is deterministic, and pinning it is what makes an
    /// accidental change to the midpoint or the branch order visible.
    /// Values from Node 24.18.1.
    #[test]
    fn unsorted_input_is_deterministic_garbage() {
        let array = [5, 1, 4, 2, 3];

        assert_eq!(search(&array, &5, None, None), -1);
        assert_eq!(lower_bound(&array, &3, None, None), 2);
        assert_eq!(upper_bound(&array, &3, None, None), 2);
    }
}

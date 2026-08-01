//! Port of upstream `static-interval-tree.js` (387 LOC, mnemonist v0.40.4).
//!
//! A static (build-once) interval tree: an augmented, balanced BST over
//! intervals sorted by start, built bottom-up from a sorted index list. Every
//! node is annotated with the interval holding the maximum end value in its
//! subtree, which is what lets both queries prune whole subtrees rather than
//! visiting every interval.
//!
//! # An empty tree crashes upstream (verified against Node 24.18.1)
//!
//! ```js
//! new StaticIntervalTree([])
//! // TypeError: Cannot read properties of undefined (reading '1')
//! ```
//!
//! `buildBST` is called unconditionally, even for `length === 0`:
//!
//! ```js
//! buildBST(intervals, endGetter, indices, tree, augmentations, 0, 0, length - 1);
//! //                                                              i  low  high
//! ```
//!
//! With `length === 0`, `high` is `-1`. Inside, `mid = (0 + (-1 - 0) / 2) | 0`
//! is `0` (`-0.5 | 0` truncates to `0`), so `current = sortedIndices[0]` reads
//! one past the end of a **zero-length** typed array, which is `undefined`.
//! `tree[i] = current + 1` becomes a harmless dropped `NaN` store (`tree`
//! itself is zero-length here too), but the very next line,
//! `intervals[current][1]`, indexes `intervals` with the property name
//! `"undefined"` — which is absent on a plain array — giving `undefined[1]`,
//! and indexing `undefined` throws. There is no guard anywhere upstream that
//! would catch a zero-length `intervals` before this point.
//!
//! This port reproduces the *failure*, not the crash mechanism: constructing
//! a tree from zero intervals is [`Error::EmptyIntervals`] rather than a
//! panic. A Rust panic unwinding across the FFI boundary is worse than the
//! JS exception it would stand in for (napi does not `catch_unwind` a sync
//! call — see `crates/mnemonist-napi/src/bit_vector.rs`), so an `Err` upstream
//! would reach as a thrown `TypeError` is the faithful *outcome*, even though
//! the *mechanism* (index-into-`undefined`) has no honest Rust expression.
//! Recorded as B-100 in `docs/modules/static-interval-tree.md`.
//!
//! # Getters are resolved once, not re-invoked per query
//!
//! Upstream calls `startGetter`/`endGetter` (or the default `interval[0]`/
//! `interval[1]`) both while building the tree and again, on every visited
//! node, inside each query method. Because the getters are pure functions of
//! an immutable stored interval (the test suite's, and every sane caller's),
//! re-invoking them at query time can only ever reproduce the same
//! `(start, end)` pair building already computed. `mnemonist-core` therefore
//! takes the **resolved bounds** once, at construction — [`bounds`] below —
//! and the getters themselves (a JS-value concern: an arbitrary callback, or
//! a default index/property read) never appear in this crate, matching
//! DESIGN.md §3.5. The bridge is where a getter, if any, actually runs.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::structures::static_interval_tree::StaticIntervalTree;
//!
//! let bounds = vec![(20.0, 36.0), (3.0, 41.0), (0.0, 1.0), (29.0, 99.0), (10.0, 15.0)];
//! let intervals = bounds.clone();
//! let tree = StaticIntervalTree::new(intervals, bounds).unwrap();
//!
//! assert_eq!(tree.size(), 5);
//! assert_eq!(tree.height(), 3);
//! assert_eq!(
//!     tree.intervals_containing_point(0.0).unwrap(),
//!     vec![(0.0, 1.0)]
//! );
//! ```

use core::cmp::Ordering;
use core::fmt;

use crate::structures::backing::Backing;
use crate::structures::fixed_stack::FixedStack;
use crate::utils::typed_arrays::{get_pointer_array, PointerVec};

/// Failures this port raises where upstream either throws from deep inside a
/// helper or (see the module docs) could not honestly be reproduced as a
/// panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Constructing from zero intervals. Upstream throws a raw `TypeError`
    /// from inside `buildBST`; see the module docs.
    EmptyIntervals,
    /// `length + 1` exceeds what a 32-bit pointer array can index —
    /// upstream's `getPointerArray` throw.
    LengthTooLarge,
    /// The bounded DFS scratch stack (upstream's reused `this.stack`, a
    /// `FixedStack` sized to the tree's height) ran out of room. Not reached
    /// by any input the original suite constructs; kept as a proper `Err`
    /// rather than a panic because upstream's own `FixedStack.push` would
    /// throw here too, for the same reason.
    StackOverflow,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyIntervals => formatter.write_str(
                "mnemonist-rs/static-interval-tree: cannot build a tree from zero intervals \
                 (upstream throws a TypeError reading a property of `undefined` here; see \
                 the module docs).",
            ),
            Self::LengthTooLarge => {
                formatter.write_str(crate::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE)
            }
            Self::StackOverflow => formatter.write_str(
                "mnemonist-rs/static-interval-tree: the traversal stack overflowed its \
                 capacity (upstream's FixedStack would throw here too).",
            ),
        }
    }
}

impl std::error::Error for Error {}

/// `Math.ceil(Math.log2(length + 1))`.
fn height_of(length: usize) -> usize {
    ((length + 1) as f64).log2().ceil() as usize
}

/// `Math.pow(2, height) - 1`.
fn tree_size(height: usize) -> usize {
    (1usize << height) - 1
}

/// A static interval tree over intervals of type `T`, each associated with a
/// resolved `(start, end)` bound.
///
/// `T` is the value a query hands back — upstream returns the *original*
/// interval object, not a synthesised pair, and this does too:
/// `intervals_containing_point` returns clones of the stored `T`s.
#[derive(Debug, Clone)]
pub struct StaticIntervalTree<T> {
    intervals: Vec<T>,
    /// `(start, end)` per **original** index, aligned with `intervals`.
    bounds: Vec<(f64, f64)>,
    /// The augmented BST: `0` for an empty node, else `original_index + 1`.
    tree: PointerVec,
    /// Per **original** index: the index of the interval with the maximum
    /// end value in that node's subtree.
    augmentations: PointerVec,
    height: usize,
}

impl<T: Clone> StaticIntervalTree<T> {
    /// Build a tree from `intervals`, each paired with its resolved
    /// `(start, end)` bound at the same position.
    ///
    /// # Panics
    ///
    /// If `intervals.len() != bounds.len()` — a caller/bridge invariant, not
    /// an upstream-reachable state.
    ///
    /// # Errors
    ///
    /// [`Error::EmptyIntervals`] for zero intervals (see the module docs) and
    /// [`Error::LengthTooLarge`] when `length + 1` exceeds what a pointer
    /// array can index.
    pub fn new(intervals: Vec<T>, bounds: Vec<(f64, f64)>) -> Result<Self, Error> {
        assert_eq!(
            intervals.len(),
            bounds.len(),
            "intervals and their resolved bounds must be the same length"
        );

        let length = intervals.len();

        if length == 0 {
            return Err(Error::EmptyIntervals);
        }

        let width = get_pointer_array((length + 1) as f64).map_err(|_| Error::LengthTooLarge)?;

        // Upstream builds `indices = [0, 1, ..., length-1]` (a fresh
        // `IndicesArray`, `indices[0]` left at its zero-fill and every other
        // slot assigned) and then sorts it in place by `bounds[i].0`. Sorting
        // a plain `Vec<usize>` directly is the same permutation without the
        // intermediate identity array, which `buildBST` never observes.
        let mut order: Vec<usize> = (0..length).collect();

        order.sort_by(|&a, &b| {
            bounds[a]
                .0
                .partial_cmp(&bounds[b].0)
                .unwrap_or(Ordering::Equal)
        });

        let height = height_of(length);
        let mut tree = PointerVec::zeroed(width, tree_size(height));
        let mut augmentations = PointerVec::zeroed(width, length);

        build_bst(
            &bounds,
            &order,
            &mut tree,
            &mut augmentations,
            0,
            0,
            length as i64 - 1,
        );

        Ok(Self {
            intervals,
            bounds,
            tree,
            augmentations,
            height,
        })
    }

    /// `this.size` — the number of intervals indexed.
    pub fn size(&self) -> usize {
        self.intervals.len()
    }

    /// `this.height`.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The augmented BST array, upstream's public `this.tree`.
    pub fn tree(&self) -> &PointerVec {
        &self.tree
    }

    /// The per-interval max-end pointers, upstream's public
    /// `this.augmentations`.
    pub fn augmentations(&self) -> &PointerVec {
        &self.augmentations
    }

    /// `#.intervalsContainingPoint(point)`.
    ///
    /// # Errors
    ///
    /// [`Error::StackOverflow`] if the bounded DFS scratch stack runs out of
    /// room; see the [`Error`] docs. Not reachable by any tree this module's
    /// own test suite builds.
    pub fn intervals_containing_point(&self, point: f64) -> Result<Vec<T>, Error> {
        let mut matches = Vec::new();
        let l = self.tree.len();
        let mut stack = self.dfs_stack()?;

        stack.push(0).map_err(|_| Error::StackOverflow)?;

        while stack.size() > 0 {
            let bst_index = stack.pop().expect("size() > 0 guarantees a value") as usize;
            let interval_index = self.tree.get(bst_index) as usize - 1;
            let max_interval = self.augmentations.get(interval_index) as usize;

            let max = self.bounds[max_interval].1;

            if point > max {
                continue;
            }

            let left = bst_index * 2 + 1;

            if left < l && self.tree.get(left) != 0 {
                stack.push(left as u32).map_err(|_| Error::StackOverflow)?;
            }

            let (start, end) = self.bounds[interval_index];

            if point >= start && point <= end {
                matches.push(self.intervals[interval_index].clone());
            }

            if point < start {
                continue;
            }

            let right = bst_index * 2 + 2;

            if right < l && self.tree.get(right) != 0 {
                stack.push(right as u32).map_err(|_| Error::StackOverflow)?;
            }
        }

        Ok(matches)
    }

    /// `#.intervalsOverlappingInterval([start, end])`.
    ///
    /// # Errors
    ///
    /// As [`intervals_containing_point`](Self::intervals_containing_point).
    pub fn intervals_overlapping_interval(
        &self,
        query_start: f64,
        query_end: f64,
    ) -> Result<Vec<T>, Error> {
        let mut matches = Vec::new();
        let l = self.tree.len();
        let mut stack = self.dfs_stack()?;

        stack.push(0).map_err(|_| Error::StackOverflow)?;

        while stack.size() > 0 {
            let bst_index = stack.pop().expect("size() > 0 guarantees a value") as usize;
            let interval_index = self.tree.get(bst_index) as usize - 1;
            let max_interval = self.augmentations.get(interval_index) as usize;

            let max = self.bounds[max_interval].1;

            if query_start > max {
                continue;
            }

            let left = bst_index * 2 + 1;

            if left < l && self.tree.get(left) != 0 {
                stack.push(left as u32).map_err(|_| Error::StackOverflow)?;
            }

            let (start, end) = self.bounds[interval_index];

            if query_end >= start && query_start <= end {
                matches.push(self.intervals[interval_index].clone());
            }

            if query_end < start {
                continue;
            }

            let right = bst_index * 2 + 2;

            if right < l && self.tree.get(right) != 0 {
                stack.push(right as u32).map_err(|_| Error::StackOverflow)?;
            }
        }

        Ok(matches)
    }

    /// A fresh DFS scratch stack, sized to the tree's height exactly as
    /// upstream's `this.stack = new FixedStack(IndicesArray, this.height)`.
    ///
    /// Allocated per call rather than reused: upstream's `stack.clear()` at
    /// the top of every query already resets it to empty, so the only
    /// observable difference a reused buffer could carry over is undropped
    /// debris that no method reads. `Backing::Filled(0)` matches the *typed*
    /// `IndicesArray` upstream allocates (never a plain `Array`).
    fn dfs_stack(&self) -> Result<FixedStack<u32>, Error> {
        FixedStack::new(Backing::Filled(0u32), self.height).map_err(|_| Error::StackOverflow)
    }
}

/// Recursive bottom-up BST build, a direct translation of upstream's
/// `buildBST`. `low`/`high` are `i64` because the top-level call for a
/// single-interval tree passes `high = 0`, and — reachably only for the
/// `length == 0` case this module refuses before ever calling this function —
/// `high` can be `-1`.
///
/// Returns the augmentation value (max end) of the subtree rooted at `i`.
#[allow(clippy::too_many_arguments)]
fn build_bst(
    bounds: &[(f64, f64)],
    sorted_indices: &[usize],
    tree: &mut PointerVec,
    augmentations: &mut PointerVec,
    i: usize,
    low: i64,
    high: i64,
) -> f64 {
    // `(low + (high - low) / 2) | 0` -- JS's `|0` truncates the *whole sum*
    // toward zero, not the division in isolation.
    let mid_f = low as f64 + (high as f64 - low as f64) / 2.0;
    let mid = js_trunc(mid_f);

    let mid_minus_one = mid - 1;
    let mid_plus_one = mid + 1;

    // `sortedIndices[mid]`. In range for every call this module's constructor
    // ever makes (see the module docs for the one case that is not, which is
    // refused before construction reaches here).
    let current = sorted_indices[mid as usize];

    tree.set(i, current as u32 + 1);

    let end = bounds[current].1;

    let left = i * 2 + 1;
    let right = i * 2 + 2;

    let mut left_end = f64::NEG_INFINITY;
    let mut right_end = f64::NEG_INFINITY;

    if low <= mid_minus_one {
        left_end = build_bst(
            bounds,
            sorted_indices,
            tree,
            augmentations,
            left,
            low,
            mid_minus_one,
        );
    }

    if mid_plus_one <= high {
        right_end = build_bst(
            bounds,
            sorted_indices,
            tree,
            augmentations,
            right,
            mid_plus_one,
            high,
        );
    }

    let augmentation = end.max(left_end).max(right_end);

    let mut augmentation_pointer = current;

    if augmentation == left_end {
        // `augmentations[tree[left] - 1]`. `tree[left]` is only ever read
        // here when `augmentation == leftEnd`, which (barring an interval
        // whose end is literally `-Infinity`, never produced by any bridge
        // this module has) only happens when the left recursion actually
        // ran and wrote a real `tree[left]`.
        augmentation_pointer =
            augmentations.get(tree.get(left).saturating_sub(1) as usize) as usize;
    } else if augmentation == right_end {
        augmentation_pointer =
            augmentations.get(tree.get(right).saturating_sub(1) as usize) as usize;
    }

    augmentations.set(current, augmentation_pointer as u32);

    augmentation
}

/// `x | 0`: truncate toward zero, as a 32-bit two's-complement integer.
/// Every input this module produces is comfortably in range.
fn js_trunc(value: f64) -> i64 {
    value.trunc() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::type_complexity)]
    fn pairs(values: &[(f64, f64)]) -> (Vec<(f64, f64)>, Vec<(f64, f64)>) {
        (values.to_vec(), values.to_vec())
    }

    /// 1:1 port of every upstream `it` block in `test/static-interval-tree.js`.
    #[test]
    fn reproduces_the_upstream_suite() {
        let basic = [
            (20.0, 36.0),
            (3.0, 41.0),
            (0.0, 1.0),
            (29.0, 99.0),
            (10.0, 15.0),
        ];

        let (intervals, bounds) = pairs(&basic);
        let tree = StaticIntervalTree::new(intervals, bounds).unwrap();

        assert_eq!(tree.size(), 5);
        assert_eq!(tree.height(), 3);

        assert_eq!(tree.intervals_containing_point(134.0).unwrap(), vec![]);
        assert_eq!(
            tree.intervals_containing_point(13.0).unwrap(),
            vec![(10.0, 15.0), (3.0, 41.0)]
        );
        assert_eq!(
            tree.intervals_containing_point(0.0).unwrap(),
            vec![(0.0, 1.0)]
        );
        assert_eq!(
            tree.intervals_containing_point(4.0).unwrap(),
            vec![(3.0, 41.0)]
        );
        assert_eq!(
            tree.intervals_containing_point(25.0).unwrap(),
            vec![(20.0, 36.0), (3.0, 41.0)]
        );

        assert_eq!(
            tree.intervals_overlapping_interval(-34.0, 4.0).unwrap(),
            vec![(0.0, 1.0), (3.0, 41.0)]
        );
        assert_eq!(
            tree.intervals_overlapping_interval(-100.0, 100.0).unwrap(),
            vec![
                (10.0, 15.0),
                (20.0, 36.0),
                (29.0, 99.0),
                (0.0, 1.0),
                (3.0, 41.0),
            ]
        );
    }

    /// `StaticIntervalTree.from(map)`, where a two-entry `Map`'s default
    /// iterator yields `[key, value]` pairs -- exactly the `[start, end]`
    /// shape. Modelled here as the bridge would resolve it: two intervals,
    /// bounds taken from the pair itself.
    #[test]
    fn a_two_entry_source_gives_a_height_of_two() {
        let (intervals, bounds) = pairs(&[(20.0, 36.0), (29.0, 99.0)]);
        let tree = StaticIntervalTree::new(intervals, bounds).unwrap();

        assert_eq!(tree.size(), 2);
        assert_eq!(tree.height(), 2);
    }

    /// The "getters" test, translated: the resolved bounds are the same
    /// regardless of whether they came from array indexing or a getter
    /// callback, and the query results are the original values, unchanged.
    #[test]
    fn getters_resolve_to_the_same_bounds_object_values_survive() {
        #[derive(Debug, Clone, PartialEq)]
        struct Described {
            start: f64,
            end: f64,
        }

        let described = [
            Described {
                start: 20.0,
                end: 36.0,
            },
            Described {
                start: 3.0,
                end: 41.0,
            },
            Described {
                start: 0.0,
                end: 1.0,
            },
            Described {
                start: 29.0,
                end: 99.0,
            },
            Described {
                start: 10.0,
                end: 15.0,
            },
        ];

        let bounds: Vec<(f64, f64)> = described.iter().map(|d| (d.start, d.end)).collect();
        let intervals = described.to_vec();

        let tree = StaticIntervalTree::new(intervals, bounds).unwrap();

        assert_eq!(
            tree.intervals_containing_point(0.0).unwrap(),
            vec![Described {
                start: 0.0,
                end: 1.0
            }]
        );
        assert_eq!(
            tree.intervals_overlapping_interval(-34.0, 4.0).unwrap(),
            vec![
                Described {
                    start: 0.0,
                    end: 1.0
                },
                Described {
                    start: 3.0,
                    end: 41.0
                },
            ]
        );
    }

    /// **Verified against Node 24.18.1**: `new StaticIntervalTree([])` throws
    /// a `TypeError`. See the module docs for the exact mechanism.
    #[test]
    fn zero_intervals_is_refused_rather_than_silently_accepted() {
        let result = StaticIntervalTree::<(f64, f64)>::new(vec![], vec![]);

        assert_eq!(result.unwrap_err(), Error::EmptyIntervals);
    }

    /// A single interval: `low == high == 0`, no recursion either side, and
    /// `tree`/`augmentations` both length 1. Verified against Node: `tree ===
    /// [1]`, `augmentations === [0]`.
    #[test]
    fn a_single_interval_tree_has_one_node() {
        let (intervals, bounds) = pairs(&[(1.0, 2.0)]);
        let tree = StaticIntervalTree::new(intervals, bounds).unwrap();

        assert_eq!(tree.height(), 1);
        assert_eq!(tree.tree().len(), 1);
        assert_eq!(tree.tree().get(0), 1);
        assert_eq!(tree.augmentations().get(0), 0);
        assert_eq!(
            tree.intervals_containing_point(1.0).unwrap(),
            vec![(1.0, 2.0)]
        );
        assert_eq!(tree.intervals_containing_point(3.0).unwrap(), vec![]);
    }

    /// Ties in the sort key (equal starts) are broken by original order, a
    /// stable sort -- matching `%TypedArray%.prototype.sort`, which V8 has
    /// implemented stably since the same TC39 change that made
    /// `Array.prototype.sort` stable. Verified directly against Node 24.18.1.
    #[test]
    fn ties_in_start_are_broken_by_original_insertion_order() {
        let (intervals, bounds) = pairs(&[(5.0, 10.0), (5.0, 20.0), (5.0, 30.0)]);
        let tree = StaticIntervalTree::new(intervals, bounds).unwrap();

        // All three start at 5 and all contain point 5; the point query walks
        // the BST rather than insertion order, so this only pins that no
        // panic/underflow occurs and that every tied interval is still found.
        let mut found = tree.intervals_containing_point(5.0).unwrap();
        found.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        assert_eq!(found, vec![(5.0, 10.0), (5.0, 20.0), (5.0, 30.0)]);
    }

    /// A point exactly on an interval's boundary is inclusive on both ends --
    /// upstream considers every interval closed (`point >= start && point <=
    /// end`), stated explicitly in the upstream file's own header comment.
    #[test]
    fn intervals_are_closed_on_both_ends() {
        let (intervals, bounds) = pairs(&[(10.0, 20.0)]);
        let tree = StaticIntervalTree::new(intervals, bounds).unwrap();

        assert_eq!(
            tree.intervals_containing_point(10.0).unwrap(),
            vec![(10.0, 20.0)]
        );
        assert_eq!(
            tree.intervals_containing_point(20.0).unwrap(),
            vec![(10.0, 20.0)]
        );
        assert_eq!(tree.intervals_containing_point(9.0).unwrap(), vec![]);
        assert_eq!(tree.intervals_containing_point(21.0).unwrap(), vec![]);
    }

    /// A query interval that does not overlap anything at all.
    #[test]
    fn a_non_overlapping_query_interval_finds_nothing() {
        let (intervals, bounds) = pairs(&[(10.0, 20.0), (30.0, 40.0)]);
        let tree = StaticIntervalTree::new(intervals, bounds).unwrap();

        assert_eq!(
            tree.intervals_overlapping_interval(21.0, 29.0).unwrap(),
            vec![]
        );
    }

    /// A larger, non-power-of-two-minus-one interval count, to exercise a
    /// height beyond what the upstream suite's five intervals reach.
    #[test]
    fn a_larger_tree_answers_every_point_correctly() {
        let source: Vec<(f64, f64)> = (0..50)
            .map(|i| (i as f64 * 2.0, i as f64 * 2.0 + 1.5))
            .collect();
        let (intervals, bounds) = pairs(&source);
        let tree = StaticIntervalTree::new(intervals, bounds).unwrap();

        assert_eq!(tree.size(), 50);

        for i in 0..50 {
            let point = i as f64 * 2.0 + 0.5;
            let hits = tree.intervals_containing_point(point).unwrap();
            assert_eq!(hits, vec![(i as f64 * 2.0, i as f64 * 2.0 + 1.5)]);
        }

        // Between two intervals: nothing contains it.
        assert_eq!(tree.intervals_containing_point(1.6).unwrap(), vec![]);
    }

    /// The guard this module's constructor applies before allocating
    /// anything: `length + 1` past what a pointer array can index is refused,
    /// exactly where upstream's `getPointerArray(length + 1)` throws.
    #[test]
    fn a_length_too_large_to_index_is_refused() {
        assert_eq!(
            get_pointer_array(4_294_967_297.0).unwrap_err(),
            crate::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE
        );
    }
}

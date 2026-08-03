//! Port of upstream `kd-tree.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! A k-dimensional tree: a flat binary tree over fixed-size numeric points,
//! splitting on one axis per depth (round-robin over `dimensions`) at the
//! median of whatever points remain in the current window. Built once via
//! [`KdTree::from_rows`]/[`KdTree::from_axes`] — there is no `add`, exactly as
//! `vp-tree.js`'s tree is immutable after construction — and queried by
//! [`KdTree::nearest_neighbor`], [`KdTree::k_nearest_neighbors`] and
//! [`KdTree::linear_k_nearest_neighbors`].
//!
//! # Reuses `fixed-reverse-heap` and `comparators::TupleComparator` verbatim
//!
//! `kNearestNeighbors`/`linearKNearestNeighbors` both build `new
//! FixedReverseHeap(Array, createTupleComparator(n), k)` — the exact module
//! [`crate::structures::fixed_reverse_heap::FixedReverseHeap`] already ports,
//! parameterised over [`crate::utils::comparators::TupleComparator`], which
//! [`crate::structures::heap`]'s own docs call out as reached by this module
//! by name. Nothing new is invented for either: nearest-neighbor search here
//! is entirely "walk the tree, feed every candidate to a bounded heap
//! upstream already ports."
//!
//! `kNearestNeighbors`'s heap items are `[dist, visited++, pivot]`, compared
//! lexicographically — so **the running `visited` counter is not a diagnostic
//! aside, it is the tie-break upstream's own algorithm depends on** whenever
//! two candidates land at the exact same distance. It is threaded through the
//! recursion here for exactly that reason, not dropped as unobserved state.
//!
//! # `inplaceQuickSortIndices` is reused rather than re-derived
//!
//! `buildTree` sorts `ids[lo..hi)` by `axes[d]` the same way
//! `vp-tree.js`'s builder sorts by distance — see that module's docs for why
//! the working permutation is a [`PointerVec`] here even though upstream's
//! own `ids` is a plain array: only the scratch representation differs, and
//! [`inplace_quick_sort_indices`](crate::sort::quick::inplace_quick_sort_indices)
//! is already exhaustively tested against `sort/quick.js`.
//!
//! # What this deliberately does not model
//!
//! * **An empty tree's query.** `KDTree.from([], dimensions)` builds cleanly
//!   (see [`build_tree`]'s early return, mirroring `vp_tree`'s), but querying
//!   it would read `this.pivots[0]` as `undefined` and cascade from there. No
//!   upstream test builds an empty tree and queries it. [`KdTree::nearest_neighbor`]
//!   returns [`None`] instead.
//! * **`k <= 0`.** Upstream throws `mnemonist/kd-tree.kNearestNeighbors: k
//!   should be a positive number.` — reproduced via [`Err`] with the same
//!   message — but only for `k == 0`; `usize` cannot carry the negative or
//!   fractional values JS's untyped `k` could, and nothing in the upstream
//!   suite exercises either.
//! * **`dimensions == 0`.** Upstream's `(d + 1) % dimensions` is `NaN` and
//!   its `axes[NaN]` is `undefined`, neither of which is an error in
//!   JavaScript: it builds a degenerate one-node tree for a single row and
//!   throws `TypeError: Cannot read properties of undefined (reading '0')`
//!   from two rows up. Rust has no `NaN` index and `% 0` panics, so both are
//!   branched around: the throw is reproduced by message, and the axis step
//!   is left at `0` on a path where upstream's own `NaN` is never read back.
//!   Verified against Node 24.18.1. No upstream test constructs a
//!   zero-dimensional tree; this used to panic, and through the bridge that
//!   aborted the host process.

use crate::sort::quick::inplace_quick_sort_indices;
use crate::structures::fixed_reverse_heap::FixedReverseHeap;
use crate::structures::heap::{Store, VecStore};
use crate::utils::comparators::{create_tuple_comparator, Comparator, Thrown, TupleComparator};
use crate::utils::typed_arrays::{get_pointer_array, PointerVec};

/// [`TupleComparator`] compares `[f64; N]` directly; [`FixedReverseHeap`]'s
/// backing [`VecStore`] holds `Option<[f64; N]>` slots (a JS array's
/// possible holes -- see `heap.rs`'s own docs). This lifts the comparator
/// over that `Option` the same way `vp_tree.rs`'s `NeighborComparator` does:
/// a hole is never actually produced here (nothing pushed to this heap ever
/// shrinks it from a comparator, unlike a re-entrant JS one could), so the
/// `_` arm is unreached in practice rather than a modelled JS behaviour.
///
/// `N` is the tuple width -- 3 for `k_nearest_neighbors`'s `[dist, visited,
/// pivot]`, 2 for `linear_k_nearest_neighbors`'s `[dist, i]`. Fixed-size
/// rather than `Vec<f64>`: every tuple this heap ever holds is created once,
/// per node visited, and `Store::get`/`set` clone the slot on every sift
/// step, so a `Vec<f64>` here meant a fresh heap allocation on nearly every
/// comparison. `[f64; N]` is `Copy`, so the same clone is a stack copy. See
/// `utils/comparators.rs`'s array `Comparator` impl for why this changes
/// nothing about the comparison itself.
#[derive(Debug, Clone, Copy)]
struct TupleOfOption<const N: usize>(TupleComparator);

impl<const N: usize> Comparator<Option<[f64; N]>, Thrown> for TupleOfOption<N> {
    fn compare(&self, a: &Option<[f64; N]>, b: &Option<[f64; N]>) -> Result<f64, Thrown> {
        match (a, b) {
            (Some(a), Some(b)) => Comparator::<[f64; N], Thrown>::compare(&self.0, a, b),
            _ => Ok(0.0),
        }
    }
}

/// `mnemonist/kd-tree.kNearestNeighbors: k should be a positive number.`,
/// verbatim -- upstream's message for both `kNearestNeighbors` and
/// `linearKNearestNeighbors`, which share the guard.
pub const NON_POSITIVE_K: &str =
    "mnemonist/kd-tree.kNearestNeighbors: k should be a positive number.";

/// Upstream's `KDTree`.
///
/// `L` is the label type carried alongside each point (a plain JS value
/// upstream — a string, a number, whatever `data[i][0]` was).
#[derive(Debug, Clone)]
pub struct KdTree<L> {
    dimensions: usize,
    axes: Vec<Vec<f64>>,
    labels: Vec<L>,
    pivots: PointerVec,
    lefts: PointerVec,
    rights: PointerVec,
    size: usize,
}

/// `squaredDistanceAxes(dimensions, axes, pivot, b)`.
fn squared_distance(dimensions: usize, axes: &[Vec<f64>], pivot: usize, query: &[f64]) -> f64 {
    let mut dist = 0.0;

    for d in 0..dimensions {
        let step = axes[d][pivot] - query[d];
        dist += step * step;
    }

    dist
}

/// `buildTree(dimensions, axes, ids, labels)`.
///
/// `ids` is consumed (sorted in place per-window, per axis, exactly as
/// upstream's `inplaceQuickSortIndices(axes[d], ids, lo, hi)` mutates its
/// shared array across the whole build).
fn build_tree(
    dimensions: usize,
    axes: &[Vec<f64>],
    mut ids: PointerVec,
    n: usize,
) -> Result<(PointerVec, PointerVec, PointerVec), Thrown> {
    if n == 0 {
        let width = get_pointer_array(1.0).expect("one always fits the narrowest width");

        return Ok((
            PointerVec::zeroed(width, 0),
            PointerVec::zeroed(width, 0),
            PointerVec::zeroed(width, 0),
        ));
    }

    // `dimensions == 0` leaves upstream with an empty `axes`, and its loop
    // body runs `inplaceQuickSortIndices(axes[d], ...)` on `undefined` and
    // then `d = (d + 1) % dimensions`, which is `NaN`. Neither is an error in
    // JavaScript. With one item the window is a single element, the sort
    // never dereferences its array, no child window is pushed, and `NaN` is
    // never used again -- so upstream returns a degenerate one-node tree.
    // With two or more, the sort does reach into `undefined` and upstream
    // throws. Verified against Node 24.18.1: `KDTree.from(rows, 0)` builds
    // for `rows.length <= 1` and throws `TypeError: Cannot read properties
    // of undefined (reading '0')` from two rows up.
    //
    // Rust has neither a `NaN` index nor a `% 0`, so both are branched around
    // rather than reproduced. Before this guard existed, `axes[d]` panicked
    // and, through the bridge, aborted the host Node process on an input
    // upstream accepts.
    if dimensions == 0 && n >= 2 {
        return Err(Thrown("Cannot read properties of undefined (reading '0')"));
    }

    // `+1` because node index `0` doubles as the "no child" sentinel in
    // `lefts`/`rights`, matching upstream's own comment verbatim.
    let width = get_pointer_array((n + 1) as f64)
        .expect("mnemonist-rs/KDTree: more items than a u32 pointer array can address");

    let mut pivots = PointerVec::zeroed(width, n);
    let mut lefts = PointerVec::zeroed(width, n);
    let mut rights = PointerVec::zeroed(width, n);

    // (axis, lo, hi, parent, is_right_child_of_parent)
    let mut stack: Vec<(usize, usize, usize, Option<usize>, bool)> = vec![(0, 0, n, None, false)];
    let mut i = 0usize;

    while let Some((d, lo, hi, parent, is_right)) = stack.pop() {
        // Skipped only when there are no axes at all, which the guard above
        // has already narrowed to the single-item case -- where a sort over
        // one element is a no-op in either language.
        if dimensions > 0 {
            inplace_quick_sort_indices(&axes[d], &mut ids, lo, hi);
        }

        let window_len = hi - lo;
        let median = lo + (window_len >> 1);
        let pivot = ids.get(median) as usize;

        pivots.set(i, pivot as u32);

        if let Some(parent_index) = parent {
            if is_right {
                rights.set(parent_index, (i + 1) as u32);
            } else {
                lefts.set(parent_index, (i + 1) as u32);
            }
        }

        // `% 0` panics in Rust where JS yields `NaN`. Reached only on the
        // single-item zero-dimension path, where no child window is pushed
        // and upstream's `NaN` is never read back either, so the value is
        // unobservable rather than merely unlikely.
        let next_d = if dimensions == 0 {
            0
        } else {
            (d + 1) % dimensions
        };

        // Right
        if median != lo && median != hi - 1 {
            stack.push((next_d, median + 1, hi, Some(i), true));
        }

        // Left
        if median != lo {
            stack.push((next_d, lo, median, Some(i), false));
        }

        i += 1;
    }

    Ok((pivots, lefts, rights))
}

impl<L: Clone> KdTree<L> {
    /// `KDTree.from(iterable, dimensions)` -- `rows` is upstream's `[label,
    /// [x, y, ...]]` shape already materialised (the bridge's job is turning
    /// the JS iterable into this).
    pub fn from_rows(rows: Vec<(L, Vec<f64>)>, dimensions: usize) -> Result<Self, Thrown> {
        let n = rows.len();
        let mut axes = vec![vec![0.0f64; n]; dimensions];
        let mut labels = Vec::with_capacity(n);

        for (i, (label, point)) in rows.into_iter().enumerate() {
            // `axis[i] = row[1][d]` past the end of the point stores
            // `undefined` into a `Float64Array`, which is `NaN`. Not a throw
            // upstream, and previously an index-out-of-bounds panic here.
            for (d, axis) in axes.iter_mut().enumerate() {
                axis[i] = point.get(d).copied().unwrap_or(f64::NAN);
            }
            labels.push(label);
        }

        let ids =
            crate::utils::typed_arrays::indices(n as f64).expect("n items always addressable");
        let (pivots, lefts, rights) = build_tree(dimensions, &axes, ids, n)?;

        Ok(Self {
            dimensions,
            axes,
            labels,
            pivots,
            lefts,
            rights,
            size: n,
        })
    }

    /// `KDTree.fromAxes(axes, labels)`. Upstream defaults `labels` to
    /// `typed.indices(axes[0].length)` when omitted; that default is a JS
    /// convenience the bridge applies (numeric labels), not something core
    /// decides, so `labels` is required here.
    pub fn from_axes(axes: Vec<Vec<f64>>, labels: Vec<L>) -> Result<Self, Thrown> {
        let dimensions = axes.len();
        let n = labels.len();

        let ids =
            crate::utils::typed_arrays::indices(n as f64).expect("n items always addressable");
        let (pivots, lefts, rights) = build_tree(dimensions, &axes, ids, n)?;

        Ok(Self {
            dimensions,
            axes,
            labels,
            pivots,
            lefts,
            rights,
            size: n,
        })
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    pub fn axes(&self) -> &[Vec<f64>] {
        &self.axes
    }

    pub fn labels(&self) -> &[L] {
        &self.labels
    }

    pub fn pivots(&self) -> &PointerVec {
        &self.pivots
    }

    pub fn lefts(&self) -> &PointerVec {
        &self.lefts
    }

    pub fn rights(&self) -> &PointerVec {
        &self.rights
    }

    /// `#.nearestNeighbor`. [`None`] on an empty tree; see the module docs.
    pub fn nearest_neighbor(&self, query: &[f64]) -> Option<&L> {
        if self.size == 0 {
            return None;
        }

        let mut best_distance = f64::INFINITY;
        let mut best: Option<usize> = None;

        self.recurse_nearest(0, 0, query, &mut best_distance, &mut best);

        best.map(|pivot| &self.labels[pivot])
    }

    #[allow(clippy::too_many_arguments)]
    fn recurse_nearest(
        &self,
        d: usize,
        node: usize,
        query: &[f64],
        best_distance: &mut f64,
        best: &mut Option<usize>,
    ) {
        let left = self.lefts.get(node) as usize;
        let right = self.rights.get(node) as usize;
        let pivot = self.pivots.get(node) as usize;

        let dist = squared_distance(self.dimensions, &self.axes, pivot, query);

        if dist < *best_distance {
            *best = Some(pivot);
            *best_distance = dist;

            if dist == 0.0 {
                return;
            }
        }

        let dx = self.axes[d][pivot] - query[d];
        // See `build_tree`: `% 0` panics where JS yields `NaN`, and a
        // zero-dimension tree has at most one node, so no descent uses it.
        let next_d = if self.dimensions == 0 {
            0
        } else {
            (d + 1) % self.dimensions
        };

        if dx > 0.0 {
            if left != 0 {
                self.recurse_nearest(next_d, left - 1, query, best_distance, best);
            }
        } else if right != 0 {
            self.recurse_nearest(next_d, right - 1, query, best_distance, best);
        }

        if dx * dx < *best_distance {
            if dx > 0.0 {
                if right != 0 {
                    self.recurse_nearest(next_d, right - 1, query, best_distance, best);
                }
            } else if left != 0 {
                self.recurse_nearest(next_d, left - 1, query, best_distance, best);
            }
        }
    }

    /// `#.kNearestNeighbors`.
    /// Returns `Option<L>` per slot, not `L`. Upstream's `k === 1` branch is
    /// `return [this.nearestNeighbor(query)]` — an array of length one
    /// *whatever* `nearestNeighbor` gave, `undefined` included, and it can
    /// give `undefined` whenever a coordinate is `NaN`, since every
    /// comparison against `NaN` is false and no candidate is ever accepted.
    /// Collapsing that to an empty vector changed the array's **length**, not
    /// just its contents. Found by differential fuzzing once the grammar was
    /// widened to generate points shorter than `dimensions`.
    pub fn k_nearest_neighbors(
        &self,
        k: usize,
        query: &[f64],
    ) -> Result<Vec<Option<L>>, &'static str> {
        if k == 0 {
            return Err(NON_POSITIVE_K);
        }

        let k = k.min(self.size);

        if k == 0 {
            return Ok(Vec::new());
        }

        if k == 1 {
            return Ok(vec![self.nearest_neighbor(query).cloned()]);
        }

        let comparator = TupleOfOption::<3>(create_tuple_comparator(3));
        let heap = FixedReverseHeap::new(VecStore::<[f64; 3]>::new(), comparator, k);
        let mut visited = 0usize;

        self.recurse_knn(0, 0, query, &heap, &mut visited, k);

        let best = heap.consume().expect("VecStore never fails");
        let count = best.length().expect("VecStore never fails");
        let mut out = Vec::with_capacity(count);

        for idx in 0..count {
            if let Some(tuple) = best.get(idx).expect("VecStore never fails") {
                let pivot = tuple[2] as usize;
                out.push(Some(self.labels[pivot].clone()));
            }
        }

        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn recurse_knn(
        &self,
        d: usize,
        node: usize,
        query: &[f64],
        heap: &FixedReverseHeap<VecStore<[f64; 3]>, TupleOfOption<3>>,
        visited: &mut usize,
        k: usize,
    ) {
        let left = self.lefts.get(node) as usize;
        let right = self.rights.get(node) as usize;
        let pivot = self.pivots.get(node) as usize;

        let dist = squared_distance(self.dimensions, &self.axes, pivot, query);

        heap.push(Some([dist, *visited as f64, pivot as f64]))
            .expect("VecStore never fails");
        *visited += 1;

        let point = query[d];
        let split = self.axes[d][pivot];
        let dx = point - split;
        // See `build_tree`: `% 0` panics where JS yields `NaN`, and a
        // zero-dimension tree has at most one node, so no descent uses it.
        let next_d = if self.dimensions == 0 {
            0
        } else {
            (d + 1) % self.dimensions
        };

        if point < split {
            if left != 0 {
                self.recurse_knn(next_d, left - 1, query, heap, visited, k);
            }
        } else if right != 0 {
            self.recurse_knn(next_d, right - 1, query, heap, visited, k);
        }

        let peek_distance = heap
            .peek()
            .expect("VecStore never fails")
            .expect("at least one push has happened above")[0];

        if dx * dx < peek_distance || heap.size() < k {
            if point < split {
                if right != 0 {
                    self.recurse_knn(next_d, right - 1, query, heap, visited, k);
                }
            } else if left != 0 {
                self.recurse_knn(next_d, left - 1, query, heap, visited, k);
            }
        }
    }

    /// `#.linearKNearestNeighbors`.
    pub fn linear_k_nearest_neighbors(
        &self,
        k: usize,
        query: &[f64],
    ) -> Result<Vec<L>, &'static str> {
        if k == 0 {
            return Err(NON_POSITIVE_K);
        }

        let k = k.min(self.size);

        if k == 0 {
            return Ok(Vec::new());
        }

        let comparator = TupleOfOption::<2>(create_tuple_comparator(2));
        let heap = FixedReverseHeap::new(VecStore::<[f64; 2]>::new(), comparator, k);

        for i in 0..self.size {
            let pivot = self.pivots.get(i) as usize;
            let dist = squared_distance(self.dimensions, &self.axes, pivot, query);

            heap.push(Some([dist, i as f64]))
                .expect("VecStore never fails");
        }

        let best = heap.consume().expect("VecStore never fails");
        let count = best.length().expect("VecStore never fails");
        let mut out = Vec::with_capacity(count);

        for idx in 0..count {
            if let Some(tuple) = best.get(idx).expect("VecStore never fails") {
                let node_index = tuple[1] as usize;
                let pivot = self.pivots.get(node_index) as usize;

                out.push(self.labels[pivot].clone());
            }
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Vec<(&'static str, Vec<f64>)> {
        vec![
            ("zero", vec![2.0, 3.0]),
            ("one", vec![5.0, 4.0]),
            ("two", vec![9.0, 6.0]),
            ("three", vec![4.0, 7.0]),
            ("four", vec![8.0, 1.0]),
            ("five", vec![7.0, 2.0]),
        ]
    }

    fn as_u32(pv: &PointerVec) -> Vec<u32> {
        (0..pv.len()).map(|i| pv.get(i)).collect()
    }

    fn squared_distance_labelled(a: &[f64], b: &[f64]) -> f64 {
        a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
    }

    /// `knn(k, q)` from the upstream test file: a brute-force top-k by
    /// squared distance, ties broken however `Array.prototype.sort` (a
    /// tuple comparator over `[dist, item]`) settles them -- used here only
    /// to check *membership*, exactly as upstream's own `Set` comparison
    /// does, sidestepping tie order entirely.
    fn brute_force_knn(k: usize, query: &[f64]) -> std::collections::BTreeSet<&'static str> {
        let mut candidates: Vec<(f64, &'static str)> = data()
            .into_iter()
            .map(|(label, point)| (squared_distance_labelled(query, &point), label))
            .collect();

        candidates.sort_by(|a, b| a.partial_cmp(b).unwrap());
        candidates
            .into_iter()
            .take(k)
            .map(|(_, label)| label)
            .collect()
    }

    #[test]
    fn builds_the_tree_upstream_pins() {
        let tree = KdTree::from_rows(data(), 2).unwrap();

        for (label, point) in data() {
            assert_eq!(tree.nearest_neighbor(&point), Some(&label));
        }

        assert_eq!(tree.nearest_neighbor(&[8.0, 5.0]), Some(&"two"));

        assert_eq!(as_u32(tree.pivots()), vec![5, 1, 0, 3, 2, 4]);
        assert_eq!(as_u32(tree.lefts()), vec![2, 3, 0, 0, 6, 0]);
        assert_eq!(as_u32(tree.rights()), vec![5, 4, 0, 0, 0, 0]);
    }

    #[test]
    fn builds_from_axes_directly_and_agrees_with_from_rows() {
        let rows = data();
        let labels: Vec<&str> = rows.iter().map(|(l, _)| *l).collect();
        let axes = vec![
            rows.iter().map(|(_, p)| p[0]).collect::<Vec<f64>>(),
            rows.iter().map(|(_, p)| p[1]).collect::<Vec<f64>>(),
        ];

        let tree = KdTree::from_axes(axes, labels).unwrap();

        for (label, point) in data() {
            assert_eq!(tree.nearest_neighbor(&point), Some(&label));
        }
        assert_eq!(tree.nearest_neighbor(&[8.0, 5.0]), Some(&"two"));
        assert_eq!(as_u32(tree.pivots()), vec![5, 1, 0, 3, 2, 4]);
        assert_eq!(as_u32(tree.lefts()), vec![2, 3, 0, 0, 6, 0]);
        assert_eq!(as_u32(tree.rights()), vec![5, 4, 0, 0, 0, 0]);
    }

    #[test]
    fn builds_from_axes_without_labels_using_positional_indices() {
        let rows = data();
        let axes = vec![
            rows.iter().map(|(_, p)| p[0]).collect::<Vec<f64>>(),
            rows.iter().map(|(_, p)| p[1]).collect::<Vec<f64>>(),
        ];
        let labels: Vec<usize> = (0..rows.len()).collect();

        let tree = KdTree::from_axes(axes, labels).unwrap();

        for (i, (_, point)) in data().into_iter().enumerate() {
            assert_eq!(tree.nearest_neighbor(&point), Some(&i));
        }
        assert_eq!(tree.nearest_neighbor(&[8.0, 5.0]), Some(&2usize));
    }

    #[test]
    fn k_nearest_neighbors_matches_brute_force_membership() {
        let tree = KdTree::from_rows(data(), 2).unwrap();

        for (_, point) in data() {
            assert_eq!(
                tree.nearest_neighbor(&point).copied(),
                tree.k_nearest_neighbors(1, &point).unwrap()[0]
            );

            let by_tree: std::collections::BTreeSet<&str> = tree
                .k_nearest_neighbors(2, &point)
                .unwrap()
                .into_iter()
                .flatten()
                .collect();
            assert_eq!(by_tree, brute_force_knn(2, &point));

            let by_tree: std::collections::BTreeSet<&str> = tree
                .k_nearest_neighbors(3, &point)
                .unwrap()
                .into_iter()
                .flatten()
                .collect();
            assert_eq!(by_tree, brute_force_knn(3, &point));
        }
    }

    #[test]
    fn linear_k_nearest_neighbors_matches_upstreams_pinned_case() {
        let tree = KdTree::from_rows(data(), 2).unwrap();

        for (_, point) in data() {
            assert_eq!(
                tree.nearest_neighbor(&point),
                tree.linear_k_nearest_neighbors(1, &point).unwrap().first()
            );
        }

        assert_eq!(
            tree.linear_k_nearest_neighbors(3, &[8.0, 3.0]).unwrap(),
            vec!["five", "four", "one"]
        );
    }

    #[test]
    fn k_is_clamped_to_size_rather_than_padding_with_nothing() {
        let tree = KdTree::from_rows(data(), 2).unwrap();

        assert_eq!(tree.k_nearest_neighbors(100, &[0.0, 0.0]).unwrap().len(), 6);
        assert_eq!(
            tree.linear_k_nearest_neighbors(100, &[0.0, 0.0])
                .unwrap()
                .len(),
            6
        );
    }

    #[test]
    fn zero_k_is_rejected_with_upstreams_message() {
        let tree = KdTree::from_rows(data(), 2).unwrap();

        assert_eq!(
            tree.k_nearest_neighbors(0, &[0.0, 0.0]),
            Err(NON_POSITIVE_K)
        );
        assert_eq!(
            tree.linear_k_nearest_neighbors(0, &[0.0, 0.0]),
            Err(NON_POSITIVE_K)
        );
    }

    #[test]
    fn an_empty_tree_builds_cleanly_and_answers_no_queries() {
        let tree: KdTree<&str> = KdTree::from_rows(Vec::new(), 2).unwrap();

        assert_eq!(tree.size(), 0);
        assert_eq!(tree.nearest_neighbor(&[0.0, 0.0]), None);
    }

    /// Queries whose nearest neighbor lies across the splitting plane from
    /// the query point -- the case a naive "always trust the split" KD-tree
    /// gets wrong, and precisely why the "going the other way" branch exists
    /// in both `nearestNeighbor` and `kNearestNeighbors`.
    #[test]
    fn finds_neighbors_across_the_splitting_plane() {
        // A dense diagonal line turns out NOT to be adversarial enough here:
        // when every point's x equals its y, the primary "trust the split"
        // descent already converges on the true answer coordinate by
        // coordinate, so it is not a case that needs the "go the other way
        // too" branch at all -- confirmed empirically: this construction
        // alone did not go red under gate 6's falsification of that branch
        // (see `docs/modules/kd-tree.md`). A dense 2D *grid* (not a line) is
        // what actually forces it: many points share a coordinate on
        // whichever axis the tree splits on, so a query can land close to a
        // plane with the true nearest neighbor on its far side while the
        // primary descent's own path does not happen to pass close enough
        // first. This is also what `crates/difffuzz/src/modules/kd_tree.rs`'s
        // `grammar_self_check` uses, and what actually caught the sabotage
        // during gate 6 (this test did not, at first -- see the module doc's
        // Bugs/Fuzz section for the investigation).
        let side = 10i64;
        let rows: Vec<(usize, Vec<f64>)> = (0..side * side)
            .map(|i| (i as usize, vec![(i % side) as f64, (i / side) as f64]))
            .collect();
        let tree = KdTree::from_rows(rows.clone(), 2).unwrap();

        let mut state = 0x9E3779B97F4A7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for _ in 0..200 {
            let qx = (next() % (side as u64 * 2)) as f64 - side as f64 / 2.0;
            let qy = (next() % (side as u64 * 2)) as f64 - side as f64 / 2.0;
            let query = [qx, qy];

            // `nearest_neighbor` specifically -- its recursion is the one
            // with the "go the other way too" branch this test exists to
            // exercise; `k_nearest_neighbors` walks an entirely different
            // recursive function with its own copy of that branch, and does
            // not exercise this one at all.
            let found = tree.nearest_neighbor(&query).copied();
            let found_distance =
                found.map(|label| squared_distance_labelled(&query, &rows[label].1));

            let brute_best_distance = rows
                .iter()
                .map(|(_, p)| squared_distance_labelled(&query, p))
                .fold(f64::INFINITY, f64::min);

            assert_eq!(
                found_distance,
                Some(brute_best_distance),
                "tree's nearest_neighbor must be exactly as close as brute force's for {query:?}"
            );
        }
    }

    // ------------------------------------------------------------------
    // Malformed rows and zero dimensions. Every expectation below was taken
    // from real upstream in Node 24.18.1, not from reading kd-tree.js: each
    // of these inputs used to panic here, and through the bridge a panic
    // aborts the host process rather than throwing.
    // ------------------------------------------------------------------

    #[test]
    fn a_point_shorter_than_dimensions_pads_with_nan() {
        // Upstream: `axis[i] = row[1][d]` reads past the end, stores
        // `undefined` into a `Float64Array`, and gets `NaN`. It does not
        // throw: `KDTree.from([['a', [1]]], 2)` builds.
        let tree = KdTree::from_rows(vec![("a", vec![1.0])], 2).unwrap();

        assert_eq!(tree.axes[0][0], 1.0);
        assert!(
            tree.axes[1][0].is_nan(),
            "the missing component must be NaN"
        );
    }

    #[test]
    fn a_zero_dimension_tree_of_one_item_builds() {
        // Upstream's `axes[NaN]` is `undefined` and its one-element window
        // never dereferences it, so a single row builds a degenerate tree.
        let tree = KdTree::from_rows(vec![("a", vec![1.0, 2.0])], 0).unwrap();

        assert_eq!(tree.size, 1);
        assert_eq!(tree.dimensions, 0);
        assert!(tree.axes.is_empty());
    }

    #[test]
    fn a_zero_dimension_tree_of_two_items_raises_upstreams_type_error() {
        // From two rows up, upstream's sort really does reach into
        // `undefined` and throws.
        let error = KdTree::from_rows(vec![("a", vec![1.0]), ("b", vec![2.0])], 0).unwrap_err();

        assert_eq!(error.0, "Cannot read properties of undefined (reading '0')");
    }

    #[test]
    fn k_of_one_returns_a_one_element_array_even_when_it_holds_nothing() {
        // Upstream: `if (k === 1) return [this.nearestNeighbor(query)]`. With
        // every point shorter than `dimensions`, axis 1 is all NaN, every
        // comparison against it is false, no candidate is ever accepted, and
        // `nearestNeighbor` gives `undefined` -- which upstream still wraps in
        // an array of length one. Verified against Node 24.18.1:
        // `KDTree.from([[0,[0]],[1,[0]]], 2).kNearestNeighbors(1, [0,0])` is
        // `[undefined]`, length 1.
        let tree = KdTree::from_rows(vec![(0i64, vec![0.0]), (1, vec![0.0])], 2).unwrap();

        assert_eq!(tree.nearest_neighbor(&[0.0, 0.0]), None);

        let hits = tree.k_nearest_neighbors(1, &[0.0, 0.0]).unwrap();

        assert_eq!(
            hits.len(),
            1,
            "the array's length is upstream's, not its contents'"
        );
        assert_eq!(hits[0], None);
    }
}

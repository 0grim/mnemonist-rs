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
//! * **`dimensions == 0`.** `(d + 1) % dimensions` divides by zero on both
//!   sides (JS produces `NaN`; Rust panics); no test constructs a zero-
//!   dimensional tree.

use crate::sort::quick::inplace_quick_sort_indices;
use crate::structures::fixed_reverse_heap::FixedReverseHeap;
use crate::structures::heap::{Store, VecStore};
use crate::utils::comparators::{create_tuple_comparator, Comparator, Thrown, TupleComparator};
use crate::utils::typed_arrays::{get_pointer_array, PointerVec};

/// [`TupleComparator`] compares `Vec<f64>` directly; [`FixedReverseHeap`]'s
/// backing [`VecStore`] holds `Option<Vec<f64>>` slots (a JS array's
/// possible holes -- see `heap.rs`'s own docs). This lifts the comparator
/// over that `Option` the same way `vp_tree.rs`'s `NeighborComparator` does:
/// a hole is never actually produced here (nothing pushed to this heap ever
/// shrinks it from a comparator, unlike a re-entrant JS one could), so the
/// `_` arm is unreached in practice rather than a modelled JS behaviour.
#[derive(Debug, Clone, Copy)]
struct TupleOfOption(TupleComparator);

impl Comparator<Option<Vec<f64>>, Thrown> for TupleOfOption {
    fn compare(&self, a: &Option<Vec<f64>>, b: &Option<Vec<f64>>) -> Result<f64, Thrown> {
        match (a, b) {
            (Some(a), Some(b)) => Comparator::<Vec<f64>, Thrown>::compare(&self.0, a, b),
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
) -> (PointerVec, PointerVec, PointerVec) {
    if n == 0 {
        let width = get_pointer_array(1.0).expect("one always fits the narrowest width");

        return (
            PointerVec::zeroed(width, 0),
            PointerVec::zeroed(width, 0),
            PointerVec::zeroed(width, 0),
        );
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
        inplace_quick_sort_indices(&axes[d], &mut ids, lo, hi);

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

        let next_d = (d + 1) % dimensions;

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

    (pivots, lefts, rights)
}

impl<L: Clone> KdTree<L> {
    /// `KDTree.from(iterable, dimensions)` -- `rows` is upstream's `[label,
    /// [x, y, ...]]` shape already materialised (the bridge's job is turning
    /// the JS iterable into this).
    pub fn from_rows(rows: Vec<(L, Vec<f64>)>, dimensions: usize) -> Self {
        let n = rows.len();
        let mut axes = vec![vec![0.0f64; n]; dimensions];
        let mut labels = Vec::with_capacity(n);

        for (i, (label, point)) in rows.into_iter().enumerate() {
            for d in 0..dimensions {
                axes[d][i] = point[d];
            }
            labels.push(label);
        }

        let ids =
            crate::utils::typed_arrays::indices(n as f64).expect("n items always addressable");
        let (pivots, lefts, rights) = build_tree(dimensions, &axes, ids, n);

        Self {
            dimensions,
            axes,
            labels,
            pivots,
            lefts,
            rights,
            size: n,
        }
    }

    /// `KDTree.fromAxes(axes, labels)`. Upstream defaults `labels` to
    /// `typed.indices(axes[0].length)` when omitted; that default is a JS
    /// convenience the bridge applies (numeric labels), not something core
    /// decides, so `labels` is required here.
    pub fn from_axes(axes: Vec<Vec<f64>>, labels: Vec<L>) -> Self {
        let dimensions = axes.len();
        let n = labels.len();

        let ids =
            crate::utils::typed_arrays::indices(n as f64).expect("n items always addressable");
        let (pivots, lefts, rights) = build_tree(dimensions, &axes, ids, n);

        Self {
            dimensions,
            axes,
            labels,
            pivots,
            lefts,
            rights,
            size: n,
        }
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
        let next_d = (d + 1) % self.dimensions;

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
    pub fn k_nearest_neighbors(&self, k: usize, query: &[f64]) -> Result<Vec<L>, &'static str> {
        if k == 0 {
            return Err(NON_POSITIVE_K);
        }

        let k = k.min(self.size);

        if k == 0 {
            return Ok(Vec::new());
        }

        if k == 1 {
            return Ok(self.nearest_neighbor(query).into_iter().cloned().collect());
        }

        let comparator = TupleOfOption(create_tuple_comparator(3));
        let heap = FixedReverseHeap::new(VecStore::<Vec<f64>>::new(), comparator, k);
        let mut visited = 0usize;

        self.recurse_knn(0, 0, query, &heap, &mut visited, k);

        let best = heap.consume().expect("VecStore never fails");
        let count = best.length().expect("VecStore never fails");
        let mut out = Vec::with_capacity(count);

        for idx in 0..count {
            if let Some(tuple) = best.get(idx).expect("VecStore never fails") {
                let pivot = tuple[2] as usize;
                out.push(self.labels[pivot].clone());
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
        heap: &FixedReverseHeap<VecStore<Vec<f64>>, TupleOfOption>,
        visited: &mut usize,
        k: usize,
    ) {
        let left = self.lefts.get(node) as usize;
        let right = self.rights.get(node) as usize;
        let pivot = self.pivots.get(node) as usize;

        let dist = squared_distance(self.dimensions, &self.axes, pivot, query);

        heap.push(Some(vec![dist, *visited as f64, pivot as f64]))
            .expect("VecStore never fails");
        *visited += 1;

        let point = query[d];
        let split = self.axes[d][pivot];
        let dx = point - split;
        let next_d = (d + 1) % self.dimensions;

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

        let comparator = TupleOfOption(create_tuple_comparator(2));
        let heap = FixedReverseHeap::new(VecStore::<Vec<f64>>::new(), comparator, k);

        for i in 0..self.size {
            let pivot = self.pivots.get(i) as usize;
            let dist = squared_distance(self.dimensions, &self.axes, pivot, query);

            heap.push(Some(vec![dist, i as f64]))
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
        let tree = KdTree::from_rows(data(), 2);

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

        let tree = KdTree::from_axes(axes, labels);

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

        let tree = KdTree::from_axes(axes, labels);

        for (i, (_, point)) in data().into_iter().enumerate() {
            assert_eq!(tree.nearest_neighbor(&point), Some(&i));
        }
        assert_eq!(tree.nearest_neighbor(&[8.0, 5.0]), Some(&2usize));
    }

    #[test]
    fn k_nearest_neighbors_matches_brute_force_membership() {
        let tree = KdTree::from_rows(data(), 2);

        for (_, point) in data() {
            assert_eq!(
                tree.nearest_neighbor(&point),
                tree.k_nearest_neighbors(1, &point).unwrap().first()
            );

            let by_tree: std::collections::BTreeSet<&str> = tree
                .k_nearest_neighbors(2, &point)
                .unwrap()
                .into_iter()
                .collect();
            assert_eq!(by_tree, brute_force_knn(2, &point));

            let by_tree: std::collections::BTreeSet<&str> = tree
                .k_nearest_neighbors(3, &point)
                .unwrap()
                .into_iter()
                .collect();
            assert_eq!(by_tree, brute_force_knn(3, &point));
        }
    }

    #[test]
    fn linear_k_nearest_neighbors_matches_upstreams_pinned_case() {
        let tree = KdTree::from_rows(data(), 2);

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
        let tree = KdTree::from_rows(data(), 2);

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
        let tree = KdTree::from_rows(data(), 2);

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
        let tree: KdTree<&str> = KdTree::from_rows(Vec::new(), 2);

        assert_eq!(tree.size(), 0);
        assert_eq!(tree.nearest_neighbor(&[0.0, 0.0]), None);
    }

    /// Queries whose nearest neighbor lies across the splitting plane from
    /// the query point -- the case a naive "always trust the split" KD-tree
    /// gets wrong, and precisely why the "going the other way" branch exists
    /// in both `nearestNeighbor` and `kNearestNeighbors`.
    #[test]
    fn finds_neighbors_across_the_splitting_plane() {
        // A dense diagonal line: the tree's first split is near the middle,
        // and a query placed just barely on one side of it, but far along
        // the OTHER axis, has its true nearest neighbor on the other side of
        // that first split.
        let rows: Vec<(usize, Vec<f64>)> = (0..64).map(|i| (i, vec![i as f64, i as f64])).collect();
        let tree = KdTree::from_rows(rows, 2);

        // Splits are on x first; query sits just left of x=32 but its true
        // nearest neighbor (32, 32) sits just across that plane.
        let query = [31.6, 31.6];
        let expected_nn = 32usize;

        assert_eq!(tree.nearest_neighbor(&query), Some(&expected_nn));

        let by_tree: std::collections::BTreeSet<usize> = tree
            .k_nearest_neighbors(3, &query)
            .unwrap()
            .into_iter()
            .collect();
        let by_brute: std::collections::BTreeSet<usize> = {
            let mut candidates: Vec<(f64, usize)> = (0..64)
                .map(|i| {
                    let p = [i as f64, i as f64];
                    (squared_distance_labelled(&query, &p), i)
                })
                .collect();
            candidates.sort_by(|a, b| a.partial_cmp(b).unwrap());
            candidates.into_iter().take(3).map(|(_, i)| i).collect()
        };

        assert_eq!(by_tree, by_brute);
    }
}

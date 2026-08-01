//! Port of upstream `vp-tree.js` (mnemonist v0.40.4, commit `1f2c7520`).
//!
//! A vantage-point tree: a flat binary tree over a caller-supplied metric,
//! built once at construction (there is no `add`) and queried by
//! `nearestNeighbors`/`neighbors`. Every node picks a *vantage point* and a
//! radius `mu` splitting its remaining items into "closer than `mu`" (left)
//! and "farther than `mu`" (right); a query prunes a subtree entirely when
//! the triangle inequality proves it cannot contain anything better than
//! what has already been found.
//!
//! # Flat-array shape, reused rather than reinvented
//!
//! Upstream stores the tree as four parallel arrays — `nodes` (item index),
//! `lefts`/`rights` (child node index + 1, `0` meaning "no child"), `mus`
//! (split radius) — sized from `getPointerArray(N)`, the same width-selection
//! [`crate::utils::typed_arrays`] already implements for every other typed
//! module. [`VpTree`] keeps that representation exactly: `nodes`/`lefts`/
//! `rights` are [`PointerVec`], so the bridge can expose the same
//! `Uint8Array`/`Uint16Array`/`Uint32Array` upstream's own constructor test
//! pins byte-for-byte.
//!
//! # The heap is the closest relative already in this codebase
//!
//! `nearestNeighbors` keeps a bounded max-heap-by-distance (`new
//! Heap(comparator)`, trimmed by hand rather than `fixed-reverse-heap`'s
//! automatic capacity), and this port reuses
//! [`crate::structures::heap::Heap`] verbatim rather than re-deriving sift/tie
//! behaviour: the original test pins an *exact order* among tied distances
//! (`nearestNeighbors(5, 'look')` puts `lock` before `book`, both at distance
//! 1), so only the identical algorithm — not a re-implementation that merely
//! agrees on membership — reproduces it.
//!
//! # Distance is passed per call, never stored
//!
//! Same reasoning as `bk_tree.rs`'s: the JS callback belongs at the boundary,
//! so [`VpTree::try_new`]/[`VpTree::try_nearest_neighbors`]/
//! [`VpTree::try_neighbors`] all take a `FnMut(&I, &I) -> Result<f64, E>`
//! rather than storing `this.distance` on the struct. A failing call during
//! construction propagates before any [`VpTree`] is returned — upstream's own
//! `createBinaryTree` throws synchronously out of the constructor, and
//! nothing partially built ever escapes either side.
//!
//! # What this deliberately does not model
//!
//! * **An empty tree's query.** `new VPTree(distance, [])` builds cleanly
//!   (every array ends up length zero — see [`build`]'s early return), but
//!   upstream's `nearestNeighbors`/`neighbors` on it would read
//!   `this.nodes[0]` as `undefined`, then `this.items[undefined]` as
//!   `undefined`, and hand that `undefined` vantage point to the caller's
//!   `distance` function — which throws for every metric in the upstream
//!   suite (`levenshtein(undefined, …)`, `a[0]-b[0]` on `undefined`). No test
//!   anywhere constructs an empty tree and queries it. This port returns no
//!   neighbors instead of reproducing an unrelated crash in a caller's own
//!   metric; see the module doc's Deliberate divergences.
//! * **`k == 0`.** Upstream's arithmetic (`if (neighbors.size >= k) tau =
//!   neighbors.peek().distance`) reaches `peek()` on a heap immediately
//!   emptied back to zero, i.e. `undefined.distance` — another
//!   caller-independent crash nothing tests. Returns no neighbors instead.
//! * **A reentrant distance function's effect on shared state.** Upstream's
//!   `this.heap`/`this.D` are single instance fields reused across calls; a
//!   distance function that calls back into the same tree's query methods
//!   observes them interleaved with the outer call's. This port's queries
//!   build their scratch heap locally per call rather than sharing one on
//!   `self`, so a reentrant call here is simply independent — which is *more*
//!   correct than upstream's shared-state interleaving, not merely different.
//!   No test inspects `D`/the heap directly, so this is invisible to the
//!   upstream suite; recorded as a divergence anyway, per CLAUDE.md's rule
//!   that "more correct" still needs to be written down.

use crate::sort::quick::inplace_quick_sort_indices;
use crate::structures::heap::{Heap, VecStore};
use crate::utils::comparators::{Comparator, Thrown};
use crate::utils::typed_arrays::{get_pointer_array, indices as pointer_indices, PointerVec};

/// One `nearestNeighbors`/`neighbors` hit: upstream's `{distance, item}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Neighbor<I> {
    pub distance: f64,
    pub item: I,
}

/// `nearestNeighbors`'s internal heap comparator, verbatim:
///
/// ```js
/// function comparator(a, b) {
///   if (a.distance < b.distance) return 1;
///   if (a.distance > b.distance) return -1;
///   return 0;
/// }
/// ```
///
/// A max-heap by distance: the root is the *worst* surviving neighbor, so
/// `push` followed by `if (size > k) pop()` keeps the `k` smallest.
#[derive(Debug, Clone, Copy, Default)]
struct NeighborComparator;

impl<I> Comparator<Option<Neighbor<I>>, Thrown> for NeighborComparator {
    fn compare(&self, a: &Option<Neighbor<I>>, b: &Option<Neighbor<I>>) -> Result<f64, Thrown> {
        match (a, b) {
            (Some(a), Some(b)) => {
                if a.distance < b.distance {
                    Ok(1.0)
                } else if a.distance > b.distance {
                    Ok(-1.0)
                } else {
                    Ok(0.0)
                }
            }
            // Never reached: this comparator never touches the heap it is
            // comparing (unlike a JS comparator that could), so nothing ever
            // hands it a hole.
            _ => Ok(0.0),
        }
    }
}

/// Upstream's `VPTree`.
///
/// `I` is the item type; a distance metric is supplied per call. See the
/// module docs for why nothing here stores one.
#[derive(Debug, Clone)]
pub struct VpTree<I> {
    items: Vec<I>,
    nodes: PointerVec,
    lefts: PointerVec,
    rights: PointerVec,
    mus: Vec<f64>,
    size: usize,
}

/// `createBinaryTree(distance, items, indices)`.
///
/// `indices` upstream is a *plain* JS array (`iterables.toArrayWithIndices`
/// never typed-arrays it), so the working permutation here is also not a
/// [`PointerVec`] for fidelity's own sake — it is one purely so that
/// [`inplace_quick_sort_indices`] (already exhaustively tested against
/// `sort/quick.js`) can be reused rather than re-derived. The algorithm and
/// every comparison are identical either way; only the scratch
/// representation differs, and it is never observed.
fn build<I, F, E>(
    items: &[I],
    distance: &mut F,
) -> Result<(PointerVec, PointerVec, PointerVec, Vec<f64>), E>
where
    F: FnMut(&I, &I) -> Result<f64, E>,
{
    let n = items.len();

    if n == 0 {
        let width = get_pointer_array(0.0).expect("zero always fits the narrowest width");

        return Ok((
            PointerVec::zeroed(width, 0),
            PointerVec::zeroed(width, 0),
            PointerVec::zeroed(width, 0),
            Vec::new(),
        ));
    }

    let width = get_pointer_array(n as f64)
        .expect("mnemonist-rs/VPTree: more items than a u32 pointer array can address");
    let mut indices = pointer_indices(n as f64).expect("n items always addressable at `width`");

    let mut nodes = PointerVec::zeroed(width, n);
    let mut lefts = PointerVec::zeroed(width, n);
    let mut rights = PointerVec::zeroed(width, n);
    let mut mus = vec![0.0f64; n];
    let mut distances = vec![0.0f64; n];

    let mut next_free = 0usize;
    // (node_index, lo, hi) -- half-open `[lo, hi)` over `indices`.
    let mut stack: Vec<(usize, usize, usize)> = vec![(0, 0, n)];

    while let Some((node_index, lo, hi)) = stack.pop() {
        let vantage_point = indices.get(hi - 1) as usize;
        let hi = hi - 1;
        let window_len = hi - lo;

        nodes.set(node_index, vantage_point as u32);

        if window_len == 0 {
            continue;
        }

        if window_len == 1 {
            let other = indices.get(lo) as usize;
            let mu = distance(&items[vantage_point], &items[other])?;

            mus[node_index] = mu;

            next_free += 1;
            rights.set(node_index, next_free as u32);
            nodes.set(next_free, other as u32);

            continue;
        }

        for i in lo..hi {
            let idx = indices.get(i) as usize;
            distances[idx] = distance(&items[vantage_point], &items[idx])?;
        }

        inplace_quick_sort_indices(&distances, &mut indices, lo, hi);

        let median_index = lo as f64 + (window_len as f64) / 2.0 - 1.0;

        let mu = if median_index == median_index.trunc() {
            let mid = median_index as usize;
            let a = distances[indices.get(mid) as usize];
            let b = distances[indices.get(mid + 1) as usize];

            (a + b) / 2.0
        } else {
            let mid = median_index.ceil() as usize;

            distances[indices.get(mid) as usize]
        };

        mus[node_index] = mu;

        let mid = lower_bound(&distances, &indices, mu, lo, hi);

        // Right
        if hi - mid > 0 {
            next_free += 1;
            rights.set(node_index, next_free as u32);
            stack.push((next_free, mid, hi));
        }

        // Left
        if mid - lo > 0 {
            next_free += 1;
            lefts.set(node_index, next_free as u32);
            stack.push((next_free, lo, mid));
        }
    }

    Ok((nodes, lefts, rights, mus))
}

/// `lowerBoundIndices(distances, indices, mu, lo, hi)`, restricted to the
/// case `vp-tree.js` always calls it in: `lo`/`hi` explicit and in range, so
/// none of `utils/binary-search.js`'s out-of-range/defaulted-`hi` quirks
/// (see [`crate::utils::binary_search::lower_bound_indices`]'s own docs)
/// apply here.
fn lower_bound(distances: &[f64], indices: &PointerVec, value: f64, lo: usize, hi: usize) -> usize {
    let mut lo = lo;
    let mut hi = hi;

    while lo < hi {
        let mid = (lo + hi) / 2;
        let candidate = distances[indices.get(mid) as usize];

        if value <= candidate {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }

    lo
}

impl<I: Clone> VpTree<I> {
    /// `new VPTree(distance, items)`, for a distance function that cannot
    /// fail.
    pub fn new<F>(items: Vec<I>, mut distance: F) -> Self
    where
        F: FnMut(&I, &I) -> f64,
    {
        match Self::try_new(items, |a, b| {
            Ok::<f64, std::convert::Infallible>(distance(a, b))
        }) {
            Ok(tree) => tree,
            Err(never) => match never {},
        }
    }

    /// `new VPTree(distance, items)`, for a distance function that can fail
    /// (the JS bridge's case: the callback can throw).
    ///
    /// A failing call propagates before any tree is returned — there is no
    /// partially built [`VpTree`] to leak, unlike `BkTree::try_add`, because
    /// construction here either completes or never produces a value at all.
    pub fn try_new<F, E>(items: Vec<I>, mut distance: F) -> Result<Self, E>
    where
        F: FnMut(&I, &I) -> Result<f64, E>,
    {
        let size = items.len();
        let (nodes, lefts, rights, mus) = build(&items, &mut distance)?;

        Ok(Self {
            items,
            nodes,
            lefts,
            rights,
            mus,
            size,
        })
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn items(&self) -> &[I] {
        &self.items
    }

    pub fn nodes(&self) -> &PointerVec {
        &self.nodes
    }

    pub fn lefts(&self) -> &PointerVec {
        &self.lefts
    }

    pub fn rights(&self) -> &PointerVec {
        &self.rights
    }

    pub fn mus(&self) -> &[f64] {
        &self.mus
    }

    /// `#.nearestNeighbors`, for a distance function that cannot fail.
    pub fn nearest_neighbors<F>(&self, k: usize, query: &I, mut distance: F) -> Vec<Neighbor<I>>
    where
        F: FnMut(&I, &I) -> f64,
    {
        match self.try_nearest_neighbors(k, query, |a, b| {
            Ok::<f64, std::convert::Infallible>(distance(a, b))
        }) {
            Ok(neighbors) => neighbors,
            Err(never) => match never {},
        }
    }

    /// `#.nearestNeighbors`, for a distance function that can fail.
    ///
    /// Upstream never clamps `k` to `this.size` here (unlike `kd-tree.js`'s
    /// `kNearestNeighbors`) — a `k` past the tree's size simply never trims,
    /// so every item comes back, sorted ascending by distance.
    pub fn try_nearest_neighbors<F, E>(
        &self,
        k: usize,
        query: &I,
        mut distance: F,
    ) -> Result<Vec<Neighbor<I>>, E>
    where
        F: FnMut(&I, &I) -> Result<f64, E>,
    {
        if self.size == 0 || k == 0 {
            return Ok(Vec::new());
        }

        let heap: Heap<VecStore<Neighbor<I>>, NeighborComparator> =
            Heap::new(VecStore::new(), NeighborComparator);

        let mut stack = vec![0usize];
        let mut tau = f64::INFINITY;

        while let Some(node_index) = stack.pop() {
            let item_index = self.nodes.get(node_index) as usize;
            let vantage_point = &self.items[item_index];

            let d = distance(vantage_point, query)?;

            if d < tau {
                heap.push(Some(Neighbor {
                    distance: d,
                    item: vantage_point.clone(),
                }))
                .expect("VecStore never fails");

                if heap.size() > k {
                    heap.pop().expect("VecStore never fails");
                }

                if heap.size() >= k {
                    tau = heap
                        .peek()
                        .expect("VecStore never fails")
                        .expect("heap is non-empty: size >= k >= 1 here")
                        .distance;
                }
            }

            let left_index = self.lefts.get(node_index) as usize;
            let right_index = self.rights.get(node_index) as usize;

            if left_index == 0 && right_index == 0 {
                continue;
            }

            let mu = self.mus[node_index];

            if d < mu {
                if left_index != 0 && d < mu + tau {
                    stack.push(left_index);
                }
                if right_index != 0 && d >= mu - tau {
                    stack.push(right_index);
                }
            } else {
                if right_index != 0 && d >= mu - tau {
                    stack.push(right_index);
                }
                if left_index != 0 && d < mu + tau {
                    stack.push(left_index);
                }
            }
        }

        let size = heap.size();
        let mut array: Vec<Option<Neighbor<I>>> = vec![None; size];
        let mut i = size;

        while i > 0 {
            i -= 1;
            array[i] = heap.pop().expect("VecStore never fails");
        }

        Ok(array.into_iter().flatten().collect())
    }

    /// `#.neighbors`, for a distance function that cannot fail.
    pub fn neighbors<F>(&self, radius: f64, query: &I, mut distance: F) -> Vec<Neighbor<I>>
    where
        F: FnMut(&I, &I) -> f64,
    {
        match self.try_neighbors(radius, query, |a, b| {
            Ok::<f64, std::convert::Infallible>(distance(a, b))
        }) {
            Ok(neighbors) => neighbors,
            Err(never) => match never {},
        }
    }

    /// `#.neighbors`, for a distance function that can fail.
    pub fn try_neighbors<F, E>(
        &self,
        radius: f64,
        query: &I,
        mut distance: F,
    ) -> Result<Vec<Neighbor<I>>, E>
    where
        F: FnMut(&I, &I) -> Result<f64, E>,
    {
        if self.size == 0 {
            return Ok(Vec::new());
        }

        let mut found = Vec::new();
        let mut stack = vec![0usize];

        while let Some(node_index) = stack.pop() {
            let item_index = self.nodes.get(node_index) as usize;
            let vantage_point = &self.items[item_index];

            let d = distance(vantage_point, query)?;

            if d <= radius {
                found.push(Neighbor {
                    distance: d,
                    item: vantage_point.clone(),
                });
            }

            let left_index = self.lefts.get(node_index) as usize;
            let right_index = self.rights.get(node_index) as usize;

            if left_index == 0 && right_index == 0 {
                continue;
            }

            let mu = self.mus[node_index];

            if d < mu {
                if left_index != 0 && d < mu + radius {
                    stack.push(left_index);
                }
                if right_index != 0 && d >= mu - radius {
                    stack.push(right_index);
                }
            } else {
                if right_index != 0 && d >= mu - radius {
                    stack.push(right_index);
                }
                if left_index != 0 && d < mu + radius {
                    stack.push(left_index);
                }
            }
        }

        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levenshtein(a: &str, b: &str) -> f64 {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        let mut row: Vec<i64> = (0..=b.len() as i64).collect();

        for (i, &ca) in a.iter().enumerate() {
            let mut previous = row[0];
            row[0] = i as i64 + 1;

            for (j, &cb) in b.iter().enumerate() {
                let temp = row[j + 1];
                row[j + 1] = if ca == cb {
                    previous
                } else {
                    1 + previous.min(row[j]).min(row[j + 1])
                };
                previous = temp;
            }
        }

        row[b.len()] as f64
    }

    fn dist(a: &&str, b: &&str) -> f64 {
        levenshtein(a, b)
    }

    const WORDS: [&str; 15] = [
        "book",
        "back",
        "bock",
        "lock",
        "mack",
        "shock",
        "ephemeral",
        "magistral",
        "shawarma",
        "falafel",
        "onze",
        "douze",
        "treize",
        "quatorze",
        "quinze",
    ];

    const WORST_CASE: [&str; 8] = ["abc", "abc", "abc", "bde", "bde", "cd", "cd", "abc"];

    fn identity(a: &&str, b: &&str) -> f64 {
        if a != b {
            1.0
        } else {
            0.0
        }
    }

    fn as_u32(pv: &PointerVec) -> Vec<u32> {
        (0..pv.len()).map(|i| pv.get(i)).collect()
    }

    /// The upstream constructor test, transcribed exactly: `tree.nodes`,
    /// `.lefts`, `.rights` and `.mus` are pinned byte-for-byte.
    #[test]
    fn builds_the_tree_upstream_pins() {
        let tree = VpTree::new(WORDS.to_vec(), dist);

        assert_eq!(tree.size(), 15);
        assert_eq!(
            as_u32(tree.nodes()),
            vec![14, 6, 12, 13, 11, 10, 3, 8, 7, 9, 5, 2, 0, 4, 1]
        );
        assert_eq!(
            as_u32(tree.lefts()),
            vec![2, 7, 0, 0, 0, 0, 11, 9, 0, 0, 0, 0, 14, 0, 0]
        );
        assert_eq!(
            as_u32(tree.rights()),
            vec![1, 6, 3, 4, 5, 0, 10, 8, 0, 0, 12, 0, 13, 0, 0]
        );
        assert_eq!(
            tree.mus(),
            &[6.0, 8.0, 4.0, 5.0, 2.0, 0.0, 2.0, 7.0, 0.0, 0.0, 3.0, 0.0, 2.5, 0.0, 0.0]
        );
    }

    #[test]
    fn builds_the_worst_case_tree_upstream_pins() {
        let tree = VpTree::new(WORST_CASE.to_vec(), identity);

        assert_eq!(tree.size(), 8);
        assert_eq!(as_u32(tree.nodes()), vec![7, 6, 2, 1, 0, 3, 5, 4]);
        assert_eq!(as_u32(tree.lefts()), vec![2, 6, 0, 0, 0, 0, 0, 0]);
        assert_eq!(as_u32(tree.rights()), vec![1, 5, 3, 4, 0, 7, 0, 0]);
        assert_eq!(tree.mus(), &[1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn finds_the_k_nearest_neighbors_in_the_upstream_order() {
        let tree = VpTree::new(WORDS.to_vec(), dist);

        let neighbors = tree.nearest_neighbors(2, &"look", dist);
        assert_eq!(
            neighbors,
            vec![
                Neighbor {
                    distance: 1.0,
                    item: "book"
                },
                Neighbor {
                    distance: 1.0,
                    item: "lock"
                },
            ]
        );

        let neighbors = tree.nearest_neighbors(5, &"look", dist);
        assert_eq!(
            neighbors,
            vec![
                Neighbor {
                    distance: 1.0,
                    item: "lock"
                },
                Neighbor {
                    distance: 1.0,
                    item: "book"
                },
                Neighbor {
                    distance: 2.0,
                    item: "bock"
                },
                Neighbor {
                    distance: 3.0,
                    item: "mack"
                },
                Neighbor {
                    distance: 3.0,
                    item: "back"
                },
            ]
        );
    }

    fn as_set<'a>(
        neighbors: &[Neighbor<&'a str>],
    ) -> std::collections::BTreeSet<(String, &'a str)> {
        neighbors
            .iter()
            .map(|n| (format!("{:.6}", n.distance), n.item))
            .collect()
    }

    #[test]
    fn finds_every_neighbor_within_radius() {
        let tree = VpTree::new(WORDS.to_vec(), dist);

        let got = tree.neighbors(2.0, &"look", dist);
        let expected = vec![
            Neighbor {
                distance: 1.0,
                item: "lock",
            },
            Neighbor {
                distance: 1.0,
                item: "book",
            },
            Neighbor {
                distance: 2.0,
                item: "bock",
            },
        ];
        assert_eq!(as_set(&got), as_set(&expected));

        let got = tree.neighbors(3.0, &"look", dist);
        let expected = vec![
            Neighbor {
                distance: 1.0,
                item: "lock",
            },
            Neighbor {
                distance: 3.0,
                item: "shock",
            },
            Neighbor {
                distance: 1.0,
                item: "book",
            },
            Neighbor {
                distance: 3.0,
                item: "mack",
            },
            Neighbor {
                distance: 3.0,
                item: "back",
            },
            Neighbor {
                distance: 2.0,
                item: "bock",
            },
        ];
        assert_eq!(as_set(&got), as_set(&expected));
    }

    fn euclid2d(a: &[i64; 2], b: &[i64; 2]) -> f64 {
        let dx = (a[0] - b[0]) as f64;
        let dy = (a[1] - b[1]) as f64;

        (dx * dx + dy * dy).sqrt()
    }

    /// Upstream issue #147: repeated zero-distance items must all come back.
    #[test]
    fn returns_every_neighbor_at_zero_distance() {
        let tree = VpTree::new(vec![[-100i64, -100], [100, 100]], euclid2d);

        let neighbors = tree.nearest_neighbors(2, &[100, 100], euclid2d);
        assert_eq!(
            neighbors,
            vec![
                Neighbor {
                    distance: 0.0,
                    item: [100, 100]
                },
                Neighbor {
                    distance: 80_000f64.sqrt(),
                    item: [-100, -100]
                },
            ]
        );

        let tree = VpTree::new(
            vec![[-100i64, -100], [100, 100], [100, 100], [100, 100]],
            euclid2d,
        );

        let neighbors = tree.nearest_neighbors(3, &[100, 100], euclid2d);
        assert_eq!(
            neighbors,
            vec![
                Neighbor {
                    distance: 0.0,
                    item: [100, 100]
                },
                Neighbor {
                    distance: 0.0,
                    item: [100, 100]
                },
                Neighbor {
                    distance: 0.0,
                    item: [100, 100]
                },
            ]
        );
    }

    #[test]
    fn an_empty_tree_builds_cleanly_and_answers_no_queries() {
        let tree: VpTree<&str> = VpTree::new(Vec::new(), dist);

        assert_eq!(tree.size(), 0);
        assert_eq!(tree.nearest_neighbors(3, &"anything", dist), vec![]);
        assert_eq!(tree.neighbors(100.0, &"anything", dist), vec![]);
    }

    #[test]
    fn a_failing_distance_during_construction_leaves_no_tree_behind() {
        let outcome: Result<VpTree<&str>, &str> =
            VpTree::try_new(vec!["a", "b", "c"], |_, _| Err("boom"));

        assert_eq!(outcome.err(), Some("boom"));
    }

    #[test]
    fn a_failing_distance_during_a_query_propagates() {
        let tree = VpTree::new(vec!["a", "b", "c", "d", "e"], dist);

        let outcome: Result<Vec<Neighbor<&str>>, &str> =
            tree.try_nearest_neighbors(2, &"a", |_, _| Err("boom"));

        assert_eq!(outcome.err(), Some("boom"));
    }

    /// D-heavy: a query with a wide-open radius must exercise *both* the
    /// "search the other subtree too" branch and the "skip it" branch across
    /// a single query set, or half the pruning logic is dead code. Counts how
    /// many of the 15 nodes are visited per query at three different radii.
    #[test]
    fn pruning_goes_both_ways_across_radii() {
        let tree = VpTree::new(WORDS.to_vec(), dist);

        // A radius of 0 prunes hardest: almost nothing but the exact query
        // (if present) should be visited relative to the tree's 15 nodes.
        let tiny = tree.neighbors(0.0, &"book", dist);
        // A huge radius forces every node to be a hit and both subtrees to
        // always be explored (no pruning at all is possible).
        let huge = tree.neighbors(1000.0, &"book", dist);

        assert_eq!(huge.len(), 15, "an unbounded radius must visit every item");
        assert!(
            tiny.len() < huge.len(),
            "a zero radius must prune strictly more than an unbounded one"
        );
    }
}

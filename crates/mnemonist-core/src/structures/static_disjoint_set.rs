//! Port of upstream `static-disjoint-set.js` (mnemonist v0.40.4, commit
//! `1f2c7520`).
//!
//! A static disjoint set (union-find). "Static" because the number of items
//! must be known up front: upstream sizes two typed arrays at construction
//! time and never grows them.
//!
//! # Fidelity notes
//!
//! This is a behavioural port, not a transliteration: it uses `&mut self`
//! where upstream mutates and returns `Result` where upstream throws. Three
//! upstream details are reproduced deliberately, because they are observable:
//!
//! 1. **The rank bug.** [`StaticDisjointSet::union`] compares the ranks of its
//!    *arguments* instead of the ranks of their *roots*, which disables the
//!    union-by-rank heuristic. See the comment in `union` for details. Results
//!    stay correct, but *which* element ends up as a set's root differs from a
//!    textbook union-find, and `find` returns that root. **Do not fix it here**
//!    — it is reported upstream separately.
//! 2. **Two different pointer widths.** `parents` is sized by
//!    `getPointerArray(size)` and `ranks` by `getPointerArray(Math.log2(size))`.
//!    Ranks are therefore *always* 8-bit in practice — widening would need
//!    `log2(size) > 256`, and `parents` already rejects any `size` past
//!    2<sup>32</sup> — and, because of the bug above,
//!    a root's rank can be bumped once per union and wrap around at 256. The
//!    wrap is emulated by [`PointerVec`], which allocates a real `Vec<u8>` for
//!    an 8-bit width, so the narrowing store truncates exactly as a JS
//!    `Uint8Array` write does.
//! 3. **First-encounter-by-index ordering** in `mapping` and `compile`: set ids
//!    are handed out in the order their members are first walked, ascending by
//!    item index.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::structures::static_disjoint_set::StaticDisjointSet;
//!
//! let mut sets = StaticDisjointSet::new(5).unwrap();
//! sets.union(0, 1);
//! sets.union(3, 4);
//!
//! assert_eq!(sets.dimension(), 3);
//! assert!(sets.connected(0, 1));
//! assert!(!sets.connected(0, 2));
//! assert_eq!(sets.compile(), vec![vec![0, 1], vec![2], vec![3, 4]]);
//! ```

use crate::utils::typed_arrays::{get_pointer_array, PointerVec, PointerWidth};

/// The result of [`StaticDisjointSet::mapping`].
///
/// Upstream returns a real typed array whose constructor is picked at call
/// time from `getPointerArray(this.dimension)`, and that choice is observable
/// to JS callers. Rust has no runtime array constructor, so the chosen width
/// travels alongside the values and the napi bridge rebuilds the matching JS
/// `TypedArray` from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    width: PointerWidth,
    values: Vec<u32>,
}

impl Mapping {
    /// Width the equivalent JS typed array would have been allocated with.
    pub fn width(&self) -> PointerWidth {
        self.width
    }

    /// Set id assigned to each item, indexed by item.
    pub fn values(&self) -> &[u32] {
        &self.values
    }

    /// Consume the mapping, yielding the set ids.
    pub fn into_values(self) -> Vec<u32> {
        self.values
    }
}

/// A static disjoint set (union-find) over the items `0..size`.
#[derive(Debug, Clone)]
pub struct StaticDisjointSet {
    size: usize,
    dimension: usize,
    parents: PointerVec,
    ranks: PointerVec,
}

impl StaticDisjointSet {
    /// Build a set of `size` singletons.
    ///
    /// # Errors
    ///
    /// Returns [`crate::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE`] when
    /// `size` exceeds what a 32-bit pointer array can index, which is where
    /// upstream throws.
    ///
    /// # Panics
    ///
    /// A `size` that passes validation but is too large to allocate aborts the
    /// process through the global allocator rather than returning `Err`. The
    /// two arrays cost exactly what upstream's two typed arrays do — `parents`
    /// at `getPointerArray(size)` bytes per slot, `ranks` always one — so the
    /// bound is the same as upstream's. Upstream throws a catchable error in
    /// that situation; stable Rust has no fallible `Vec` allocation, so callers
    /// accepting untrusted sizes should bound them beforehand.
    pub fn new(size: usize) -> Result<Self, &'static str> {
        // Two different widths, exactly as upstream: parents must index every
        // item, whereas ranks only ever hold values on the order of log2(size).
        let parents_width = get_pointer_array(size as f64)?;
        let ranks_width = get_pointer_array((size as f64).log2())?;

        let mut parents = PointerVec::zeroed(parents_width, size);

        for i in 0..size {
            parents.set(i, i as u32);
        }

        Ok(Self {
            size,
            dimension: size,
            parents,
            ranks: PointerVec::zeroed(ranks_width, size),
        })
    }

    /// Number of items the set was built with. Never changes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Number of disjoint sets currently held. Starts at `size` and drops by
    /// one per *successful* union.
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Width chosen for the internal parents array.
    pub fn parents_width(&self) -> PointerWidth {
        self.parents.width()
    }

    /// Width chosen for the internal ranks array, from `log2(size)`.
    pub fn ranks_width(&self) -> PointerWidth {
        self.ranks.width()
    }

    /// Find the root of the set containing `x`, compressing the path walked.
    ///
    /// Takes `&mut self` because path compression rewrites parents.
    ///
    /// # Panics
    ///
    /// Panics if `x >= size`. Upstream reads past the end of a typed array
    /// instead, which yields `undefined` and quietly returns garbage; panicking
    /// is the closest honest equivalent.
    pub fn find(&mut self, x: usize) -> usize {
        // Two loops, as upstream: the first walks up to the root, the second
        // re-walks the original path and repoints every node at it.
        let mut y = x;

        loop {
            let c = self.parents.get(y) as usize;

            if y == c {
                break;
            }

            y = c;
        }

        // Path compression.
        let mut cursor = x;

        loop {
            let p = self.parents.get(cursor) as usize;

            if p == y {
                break;
            }

            self.parents.set(cursor, y as u32);
            cursor = p;
        }

        y
    }

    /// Merge the sets containing `x` and `y`.
    ///
    /// Returns whether a merge actually happened; upstream returns `this` for
    /// chaining and exposes the same information only through `dimension`.
    ///
    /// # Panics
    ///
    /// Panics if either index is `>= size`, as [`StaticDisjointSet::find`] does.
    pub fn union(&mut self, x: usize, y: usize) -> bool {
        let x_root = self.find(x);
        let y_root = self.find(y);

        // x and y are already in the same set.
        if x_root == y_root {
            return false;
        }

        self.dimension -= 1;

        // BUG-FOR-BUG REPRODUCTION -- DO NOT "FIX".
        //
        // Upstream static-disjoint-set.js:87-88 reads the ranks of the
        // *arguments* (`this.ranks[x]`, `this.ranks[y]`) where union-by-rank
        // requires the ranks of the *roots* (`this.ranks[xRoot]`,
        // `this.ranks[yRoot]`), while line 98 increments `this.ranks[xRoot]`.
        // Ranks of non-root items are therefore written once at most and read
        // forever after as stale zeroes, which disables the rank heuristic and
        // degrades unions towards O(n) chains.
        //
        // Results stay *correct* -- the sets are still partitioned the same way
        // -- but which item ends up as a set's root differs from a fixed
        // implementation, and `find` returns that root, so the difference is
        // observable. Reproduced verbatim; reported upstream separately.
        let x_rank = self.ranks.get(x);
        let y_rank = self.ranks.get(y);

        if x_rank < y_rank {
            self.parents.set(x_root, y_root as u32);
        } else if x_rank > y_rank {
            self.parents.set(y_root, x_root as u32);
        } else {
            self.parents.set(y_root, x_root as u32);
            // `ranks[xRoot]++` on a typed array: wraps at the array's width.
            let bumped = self.ranks.get(x_root).wrapping_add(1);
            self.ranks.set(x_root, bumped);
        }

        true
    }

    /// Whether `x` and `y` belong to the same set.
    ///
    /// # Panics
    ///
    /// Panics if either index is `>= size`, as [`StaticDisjointSet::find`] does.
    pub fn connected(&mut self, x: usize, y: usize) -> bool {
        let x_root = self.find(x);

        x_root == self.find(y)
    }

    /// Map every item to the id of the set it belongs to.
    ///
    /// Ids are handed out in ascending item order: the set of item `0` is
    /// always id `0`. The width is picked at call time from the *current*
    /// dimension, so it can narrow as unions accumulate.
    pub fn mapping(&mut self) -> Mapping {
        // Infallible in practice: `dimension <= size` and `get_pointer_array`
        // is monotonic, so construction already proved this width exists.
        let width = get_pointer_array(self.dimension as f64)
            .expect("dimension never exceeds size, whose width was validated at construction");

        // Upstream keys an object by root; roots are item indices, so a
        // sentinel-filled vector is the same lookup without the hashing.
        let mut ids: Vec<Option<u32>> = vec![None; self.parents.len()];
        let mut values = vec![0u32; self.parents.len()];
        let mut c: u32 = 0;

        // `values` is as long as `parents`, so this walks the same indices in
        // the same order as upstream's `for (i = 0; i < parents.length; i++)`.
        for (i, slot) in values.iter_mut().enumerate() {
            let r = self.find(i);

            match ids[r] {
                None => {
                    *slot = c;
                    ids[r] = Some(c);
                    c += 1;
                }
                Some(id) => *slot = id,
            }
        }

        Mapping { width, values }
    }

    /// Compile the disjoint set into one sorted item list per set.
    ///
    /// Sets appear in the order their first member is encountered, and members
    /// within a set are ascending, both following from the index walk.
    pub fn compile(&mut self) -> Vec<Vec<usize>> {
        let mut ids: Vec<Option<usize>> = vec![None; self.parents.len()];
        let mut result: Vec<Vec<usize>> = Vec::with_capacity(self.dimension);

        for i in 0..self.parents.len() {
            let r = self.find(i);

            match ids[r] {
                None => {
                    ids[r] = Some(result.len());
                    result.push(vec![i]);
                }
                Some(id) => result[id].push(i),
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE;

    /// Verbatim port of `test/static-disjoint-set.js`:
    /// "should be possible to have a set working."
    #[test]
    fn should_be_possible_to_have_a_set_working() {
        let mut sets = StaticDisjointSet::new(10).unwrap();

        sets.union(0, 1);
        sets.union(1, 5);
        sets.union(0, 7);

        sets.union(8, 9);

        sets.union(2, 3);
        sets.union(2, 4);

        assert_eq!(sets.size(), 10);
        assert_eq!(sets.dimension(), 4);
        assert!(sets.connected(1, 7));
        assert!(!sets.connected(6, 0));

        let mapping = sets.mapping();

        assert_eq!(mapping.values(), &[0, 0, 1, 1, 1, 0, 2, 0, 3, 3]);

        let compiled = sets.compile();

        assert_eq!(
            compiled,
            vec![vec![0, 1, 5, 7], vec![2, 3, 4], vec![6], vec![8, 9]]
        );
    }

    #[test]
    fn empty_set_is_degenerate_but_legal() {
        let mut sets = StaticDisjointSet::new(0).unwrap();

        assert_eq!(sets.size(), 0);
        assert_eq!(sets.dimension(), 0);
        // log2(0) is -inf, which still selects the narrowest width upstream.
        assert_eq!(sets.parents_width(), PointerWidth::U8);
        assert_eq!(sets.ranks_width(), PointerWidth::U8);
        assert!(sets.mapping().values().is_empty());
        assert!(sets.compile().is_empty());
    }

    #[test]
    fn singleton_is_its_own_root() {
        let mut sets = StaticDisjointSet::new(1).unwrap();

        assert_eq!(sets.find(0), 0);
        assert!(sets.connected(0, 0));
        assert_eq!(sets.dimension(), 1);
        assert_eq!(sets.mapping().values(), &[0]);
        assert_eq!(sets.compile(), vec![vec![0]]);
    }

    #[test]
    fn dimension_only_drops_on_a_successful_union() {
        let mut sets = StaticDisjointSet::new(4).unwrap();

        assert!(sets.union(0, 1));
        assert_eq!(sets.dimension(), 3);

        // Already connected: no-op, dimension untouched.
        assert!(!sets.union(1, 0));
        assert_eq!(sets.dimension(), 3);

        // Self-union is never a merge.
        assert!(!sets.union(2, 2));
        assert_eq!(sets.dimension(), 3);
    }

    #[test]
    fn find_compresses_the_whole_path() {
        let mut sets = StaticDisjointSet::new(4).unwrap();

        // Chain 3 -> 2 -> 1 -> 0 by hand, bypassing union's rank logic.
        for i in 1..4 {
            sets.parents.set(i, (i - 1) as u32);
        }

        assert_eq!(sets.find(3), 0);

        // Every node on the walked path now points straight at the root.
        assert_eq!(sets.parents.get(3), 0);
        assert_eq!(sets.parents.get(2), 0);
        assert_eq!(sets.parents.get(1), 0);
    }

    /// Pins the reproduced rank bug. A textbook union-by-rank would compare
    /// `ranks[xRoot] == ranks[yRoot] == 1` here and keep `0` as the root;
    /// upstream compares `ranks[1] == 0 < ranks[3] == 1` and flips it, so the
    /// root becomes `3`. If this test starts failing, the bug was "fixed" and
    /// the port has diverged from upstream.
    #[test]
    fn reproduces_upstream_rank_bug() {
        let mut sets = StaticDisjointSet::new(8).unwrap();

        sets.union(0, 1); // ranks[0] -> 1, parents[1] = 0
        sets.union(0, 2); // ranks[0] == 1 > ranks[2] == 0, parents[2] = 0
        sets.union(3, 4); // ranks[3] -> 1, parents[4] = 3

        // Non-root ranks were never maintained.
        assert_eq!(sets.ranks.get(1), 0);
        assert_eq!(sets.ranks.get(0), 1);
        assert_eq!(sets.ranks.get(3), 1);

        sets.union(1, 3); // reads ranks[1] == 0 and ranks[3] == 1, not the roots'

        assert_eq!(sets.find(1), 3);
        assert_eq!(sets.find(0), 3);
        assert_eq!(sets.dimension(), 4);
    }

    #[test]
    fn picks_a_distinct_width_per_array() {
        let sets = StaticDisjointSet::new(300).unwrap();

        // parents must index 300 items; ranks only log2(300) ~= 8.23.
        assert_eq!(sets.parents_width(), PointerWidth::U16);
        assert_eq!(sets.ranks_width(), PointerWidth::U8);
    }

    /// The rank bug lets a single root's rank be bumped once per union, far
    /// past what `log2(size)` sized the ranks array for. In JS that write
    /// truncates; so does ours.
    #[test]
    fn root_rank_wraps_at_the_ranks_array_width() {
        let mut sets = StaticDisjointSet::new(300).unwrap();

        sets.union(0, 1);

        // `1` is never a root again, so ranks[1] stays 0 and every one of these
        // takes the equal-ranks branch, bumping ranks[0].
        for k in 2..300 {
            sets.union(1, k);
        }

        assert_eq!(sets.dimension(), 1);
        assert_eq!(sets.ranks.get(1), 0);
        // 299 increments into a Uint8Array.
        assert_eq!(sets.ranks.get(0), 299 % 256);
    }

    #[test]
    fn mapping_width_follows_the_current_dimension() {
        let mut sets = StaticDisjointSet::new(300).unwrap();

        // 300 singletons need 16 bits worth of ids.
        assert_eq!(sets.mapping().width(), PointerWidth::U16);

        for k in 1..300 {
            sets.union(0, k);
        }

        // One set left: ids fit in 8 bits now.
        let mapping = sets.mapping();
        assert_eq!(mapping.width(), PointerWidth::U8);
        assert_eq!(mapping.values(), vec![0u32; 300].as_slice());
    }

    #[test]
    fn mapping_and_compile_agree_and_are_index_ordered() {
        let mut sets = StaticDisjointSet::new(6).unwrap();

        sets.union(4, 2);
        sets.union(5, 1);

        let mapping = sets.mapping().into_values();
        let compiled = sets.compile();

        assert_eq!(mapping, vec![0, 1, 2, 3, 2, 1]);
        assert_eq!(compiled, vec![vec![0], vec![1, 5], vec![2, 4], vec![3]]);
        assert_eq!(compiled.len(), sets.dimension());

        for (set_id, items) in compiled.iter().enumerate() {
            for &item in items {
                assert_eq!(mapping[item] as usize, set_id);
            }
        }
    }

    #[test]
    fn rejects_a_size_no_pointer_array_can_index() {
        assert_eq!(
            StaticDisjointSet::new(4_294_967_297).unwrap_err(),
            POINTER_ARRAY_TOO_LARGE
        );
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn find_panics_out_of_range() {
        let mut sets = StaticDisjointSet::new(3).unwrap();
        sets.find(3);
    }
}

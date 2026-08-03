//! Port of upstream `multi-array.js` (mnemonist v0.40.4, 448 LOC).
//!
//! `MultiArray` represents an *array of arrays* — a numeric index maps to a
//! "container" (a bucket of values) — without allocating one object per
//! bucket. Every bucket is a singly linked list threaded through one flat
//! `items` array: `tails[index]` is the position of the bucket's most
//! recently pushed item, and `pointers[pointer]` is "the position of the item
//! pushed into this bucket just before this one". Walking a bucket therefore
//! means starting at its tail and following `pointers` backwards exactly
//! `lengths[index]` times — no `None`/sentinel check is needed because the
//! walk is bounded by a length counted independently, not by a terminator.
//!
//! # Two containers, not one generic one
//!
//! Upstream's `Container` may be `Array` (the default, unbounded, exact
//! values) or a numeric typed array class (`Uint8Array`/`Uint16Array`/
//! `Uint32Array`) supplied together with a fixed `capacity`. The two modes
//! differ in more than storage width:
//!
//! * **Dynamic** (`capacity` omitted): `items`/`pointers`/`tails`/`lengths`
//!   all grow by `.push`, without bound, and values round-trip exactly (a
//!   plain `Array` element is never coerced).
//! * **Fixed** (`capacity` given): `items`/`pointers` are preallocated to
//!   `capacity` slots up front, values are coerced through `ToUint32` and
//!   narrowed to the chosen width exactly as a real typed-array store is (see
//!   `crate::utils::typed_arrays::PointerVec`, already built for
//!   `crate::structures::vector::Vector`'s identical split), and `push`/`set`
//!   throw `mnemonist/multi-array: attempting to allocate further than
//!   capacity.` once `size` reaches `capacity`.
//!
//! Only these two combinations are modelled: `test/multi-array.js` never
//! constructs a fixed-capacity `Array` container (which upstream's own
//! `hasFixedCapacity` branch would in fact break on, since a preallocated
//! `new Array(capacity)` still has no bound on total pushes the way a real
//! typed array does — untested and not reproduced), and never constructs a
//! dynamic *typed-array* container (which would call `.push` on a typed
//! array and throw a `TypeError` in real upstream, since typed arrays have no
//! such method — also untested). The same scope cut `vector.rs` makes for
//! its own unmodelled `ArrayClass` combinations, for the same reason.
//!
//! # `get` is forward order, `values(index)` is reverse
//!
//! `get(index)` builds a `length`-sized array and fills it **from the back**,
//! reading tail-to-head as it goes (`array[--i] = items[pointer]`) — so the
//! item read *first* (the most recently pushed) lands in the *last* slot,
//! and the result comes out in insertion order. `values(index)`, by
//! contrast, yields items directly off the tail-to-head walk with no
//! reversal, so it comes out in **reverse** insertion order (most recent
//! first). Both are reproduced exactly; see the tests below, which pin both
//! orders against the same bucket.
//!
//! # Internal bookkeeping widths are not reproduced; they are unobservable
//!
//! Upstream additionally picks a narrow unsigned pointer width for
//! `tails`/`lengths`/(fixed-mode) `pointers`, purely as a memory
//! optimisation over what a real `Array` would cost. Nothing in
//! `test/multi-array.js` or the differential fuzzer can observe the
//! *internal* representation of an index-to-position mapping, only the
//! *values* a bucket yields — which are still width-narrowed exactly where
//! upstream narrows them (the [`Storage::Fixed`] item backing). This is the
//! same category of simplification `crate::structures::multi_map`'s
//! `dimension()` makes (a derived count instead of a tracked one): cheaper,
//! and behaviourally identical on every reachable input.
//!
//! This bookkeeping is kept as `u32` rather than upstream's own
//! per-capacity-chosen width (which can be as narrow as `u8`) or a native
//! `usize`: `u32` is simple (one type, no runtime width dispatch the way
//! [`crate::utils::typed_arrays::PointerVec`] needs for the item backing
//! itself), safely covers every domain this port or its fuzzer reaches
//! (`test/multi-array.js`'s largest case and the benchmark's 20,000-index,
//! 1,000,000-op workload both sit many orders of magnitude below
//! `u32::MAX`, and a `Vec<f64>` of `u32::MAX` items would need on the order
//! of 34 GB before the question could even arise), and is half the width of
//! `usize` on every platform this crate targets. `pointers` in particular is
//! read once per step of every bucket walk (`get`/`values_at`), so halving
//! its element width halves the bytes that walk has to bring in from memory
//! per step -- a plausible reduction in the cache-miss cost of a
//! semi-random walk over a multi-megabyte array, in the same vein as
//! `default-map.rs`'s already-confirmed cache-miss cause, but **not itself
//! confirmed here**: no profiler or cache-counter measurement was taken, so
//! this is recorded as a hypothesis, not a finding, per CLAUDE.md's rule
//! against overclaiming performance causation.

use crate::utils::typed_arrays::{PointerVec, PointerWidth, TypedValue};

/// `mnemonist/multi-array: attempting to allocate further than capacity.`
pub const CAPACITY_EXCEEDED: &str =
    "mnemonist/multi-array: attempting to allocate further than capacity.";

/// The one throw this module has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityExceeded;

impl std::fmt::Display for CapacityExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(CAPACITY_EXCEEDED)
    }
}

impl std::error::Error for CapacityExceeded {}

/// The item backing: exact values (the default `Array`) or width-narrowed,
/// preallocated ones (a fixed-capacity typed array). See the module docs.
#[derive(Debug, Clone)]
enum Storage {
    Dynamic(Vec<f64>),
    Fixed(PointerVec),
}

impl Storage {
    fn get(&self, pointer: usize) -> f64 {
        match self {
            Self::Dynamic(items) => items[pointer],
            Self::Fixed(items) => f64::from(items.get(pointer)),
        }
    }

    /// `push` in `Dynamic` mode, direct write in `Fixed` (which is already
    /// sized to `capacity` and has nothing to push onto).
    fn push_or_write(&mut self, pointer: usize, value: f64) {
        match self {
            Self::Dynamic(items) => items.push(value),
            Self::Fixed(items) => items.set(pointer, value.to_uint32()),
        }
    }
}

/// Upstream's `MultiArray`.
#[derive(Debug, Clone)]
pub struct MultiArray {
    size: usize,
    dimension: usize,
    /// Per-index tail position, grown to `dimension` on demand (`Vector.grow`
    /// and `.resize`, in both upstream modes — see the module docs on why
    /// this is `u32` rather than `usize` or upstream's own per-capacity
    /// width).
    tails: Vec<u32>,
    /// Per-index bucket length, same growth discipline as `tails`.
    lengths: Vec<u32>,
    /// Per-item "previous position in this bucket" link. Preallocated to
    /// `capacity` and zero-filled in [`Storage::Fixed`] mode (upstream never
    /// writes to it in `push`, relying on that zero-fill); grown by one
    /// `push` per new item in [`Storage::Dynamic`] mode.
    pointers: Vec<u32>,
    storage: Storage,
    capacity: Option<usize>,
}

impl MultiArray {
    /// `new MultiArray()` / `new MultiArray(Array)` — the default, unbounded
    /// container. Values round-trip exactly; see the module docs.
    pub fn new() -> Self {
        Self {
            size: 0,
            dimension: 0,
            tails: Vec::new(),
            lengths: Vec::new(),
            pointers: Vec::new(),
            storage: Storage::Dynamic(Vec::new()),
            capacity: None,
        }
    }

    /// `new MultiArray(Uint8Array | Uint16Array | Uint32Array, capacity)`.
    pub fn fixed(width: PointerWidth, capacity: usize) -> Self {
        Self {
            size: 0,
            dimension: 0,
            tails: Vec::new(),
            lengths: Vec::new(),
            pointers: vec![0u32; capacity],
            storage: Storage::Fixed(PointerVec::zeroed(width, capacity)),
            capacity: Some(capacity),
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn dimension(&self) -> usize {
        self.dimension
    }

    pub fn capacity(&self) -> Option<usize> {
        self.capacity
    }

    pub fn has_fixed_capacity(&self) -> bool {
        self.capacity.is_some()
    }

    /// The fixed-mode storage's typed-array width, for the bridge to render
    /// `get`/`containers`/`associations` as the right JS typed array.
    /// `None` in dynamic mode, where a container is a plain `Array`.
    pub fn width(&self) -> Option<PointerWidth> {
        match &self.storage {
            Storage::Dynamic(_) => None,
            Storage::Fixed(values) => Some(values.width()),
        }
    }

    fn ensure_dimension(&mut self, dimension: usize) {
        if dimension > self.tails.len() {
            self.tails.resize(dimension, 0);
            self.lengths.resize(dimension, 0);
        }
    }

    /// `#.set(index, item)`. Creates the bucket at `index` on first use,
    /// otherwise appends to it.
    ///
    /// # Errors
    ///
    /// [`CapacityExceeded`] when the structure has a fixed capacity and is
    /// already full, or `index` itself would exceed it — checked before
    /// anything else is touched, matching upstream's own guard order.
    pub fn set(&mut self, index: usize, item: f64) -> Result<(), CapacityExceeded> {
        let pointer = self.size;

        if let Some(capacity) = self.capacity {
            if index >= capacity || self.size == capacity {
                return Err(CapacityExceeded);
            }
        }

        if index >= self.dimension {
            self.dimension = index + 1;
            self.ensure_dimension(self.dimension);

            if self.capacity.is_none() {
                self.pointers.push(0);
            }

            self.lengths[index] = 1;
        } else {
            let previous_tail = self.tails[index];

            if self.capacity.is_none() {
                self.pointers.push(previous_tail);
            } else {
                self.pointers[pointer] = previous_tail;
            }

            self.lengths[index] += 1;
        }

        self.tails[index] = pointer as u32;
        self.storage.push_or_write(pointer, item);
        self.size += 1;

        Ok(())
    }

    /// `#.push(item)` — appends a brand new one-item bucket, growing
    /// `dimension` by one.
    ///
    /// # Errors
    ///
    /// [`CapacityExceeded`], same guard as [`MultiArray::set`].
    pub fn push(&mut self, item: f64) -> Result<(), CapacityExceeded> {
        let pointer = self.size;
        let index = self.dimension;

        if let Some(capacity) = self.capacity {
            if index >= capacity || self.size == capacity {
                return Err(CapacityExceeded);
            }
        }

        self.storage.push_or_write(pointer, item);

        // Upstream's `push` only grows `this.pointers` in the *dynamic*
        // branch (`this.pointers.push(0)`); fixed-capacity mode never
        // touches it, relying on the preallocated zero already sitting at
        // `pointer` — read once by `get`/`values_at` on a single-item
        // bucket but never followed further, since the walk is bounded by
        // `lengths`, not by that value.
        if self.capacity.is_none() {
            self.pointers.push(0);
        }

        self.lengths.push(1);
        self.tails.push(pointer as u32);
        self.dimension += 1;
        self.size += 1;

        Ok(())
    }

    pub fn has(&self, index: usize) -> bool {
        index < self.dimension
    }

    /// `#.multiplicity` / `#.count` — `0` for an index past `dimension`, and
    /// also `0` for one below it that was never actually written (a gap left
    /// by an out-of-order `set`; see
    /// `inserting_out_of_order_leaves_a_real_gap_at_dimension`).
    pub fn multiplicity(&self, index: usize) -> usize {
        if index >= self.dimension {
            0
        } else {
            self.lengths[index] as usize
        }
    }

    /// `#.get(index)` — the bucket in **insertion** order, or `None` past
    /// `dimension`. See the module docs for why this is the reverse of
    /// [`MultiArray::values_at`].
    pub fn get(&self, index: usize) -> Option<Vec<f64>> {
        if index >= self.dimension {
            return None;
        }

        let length = self.lengths[index];
        let mut pointer = self.tails[index];
        let mut out = vec![0.0; length as usize];
        let mut i = length;

        while i != 0 {
            i -= 1;
            out[i as usize] = self.storage.get(pointer as usize);
            pointer = self.pointers[pointer as usize];
        }

        Some(out)
    }

    /// The bucket at `index` in **reverse-insertion** order (most recently
    /// pushed first) — upstream's `values(index)`. Empty for an index past
    /// `dimension`, matching the empty iterator upstream returns there.
    pub fn values_at(&self, index: usize) -> Vec<f64> {
        if index >= self.dimension {
            return Vec::new();
        }

        let length = self.lengths[index];
        let mut pointer = self.tails[index];
        let mut out = Vec::with_capacity(length as usize);

        for _ in 0..length {
            out.push(self.storage.get(pointer as usize));
            pointer = self.pointers[pointer as usize];
        }

        out
    }

    /// Every item, in raw storage (global insertion) order — upstream's
    /// `values()` with no index.
    pub fn values(&self) -> Vec<f64> {
        (0..self.size)
            .map(|pointer| self.storage.get(pointer))
            .collect()
    }

    /// `#.containers()` — every bucket, `get`'s order, index `0..dimension`.
    pub fn containers(&self) -> Vec<Vec<f64>> {
        (0..self.dimension)
            .map(|index| self.get(index).expect("index < dimension by construction"))
            .collect()
    }

    /// `#.associations()` — `(index, container)`, `get`'s order.
    pub fn associations(&self) -> Vec<(usize, Vec<f64>)> {
        (0..self.dimension)
            .map(|index| {
                (
                    index,
                    self.get(index).expect("index < dimension by construction"),
                )
            })
            .collect()
    }

    /// `#.entries()` — `(index, value)`, reverse-insertion order within each
    /// bucket (matching [`MultiArray::values_at`]), skipping any index whose
    /// bucket is empty (a gap left by an out-of-order `set`).
    pub fn entries(&self) -> Vec<(usize, f64)> {
        let mut out = Vec::with_capacity(self.size);

        for index in 0..self.dimension {
            for value in self.values_at(index) {
                out.push((index, value));
            }
        }

        out
    }

    /// `#.keys()` — `0..dimension`.
    pub fn keys(&self) -> Vec<usize> {
        (0..self.dimension).collect()
    }
}

impl Default for MultiArray {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dynamic_from_sets(sets: &[(usize, f64)]) -> MultiArray {
        let mut array = MultiArray::new();

        for &(index, item) in sets {
            array.set(index, item).unwrap();
        }

        array
    }

    /// 1:1 transcription of `test/multi-array.js`'s "add items" case.
    #[test]
    fn reproduces_the_upstream_set_walkthrough() {
        let array =
            dynamic_from_sets(&[(0, 1.0), (0, 2.0), (0, 3.0), (1, 4.0), (1, 5.0), (2, 6.0)]);

        assert_eq!(array.size(), 6);
        assert_eq!(array.dimension(), 3);
    }

    /// "push containers", both the default and `Uint8Array` fixed variants,
    /// including the fixed variant's capacity throw.
    #[test]
    fn reproduces_the_upstream_push_walkthrough() {
        let mut array = MultiArray::new();
        array.push(1.0).unwrap();
        array.push(2.0).unwrap();
        array.push(3.0).unwrap();

        assert_eq!(array.size(), 3);
        assert_eq!(array.dimension(), 3);
        assert_eq!(array.get(0), Some(vec![1.0]));
        assert_eq!(array.get(1), Some(vec![2.0]));
        assert_eq!(array.get(2), Some(vec![3.0]));

        array.set(0, 4.0).unwrap();
        array.set(1, 5.0).unwrap();
        array.set(2, 6.0).unwrap();

        assert_eq!(array.size(), 6);
        assert_eq!(array.dimension(), 3);
        assert_eq!(array.get(0), Some(vec![1.0, 4.0]));
        assert_eq!(array.get(1), Some(vec![2.0, 5.0]));
        assert_eq!(array.get(2), Some(vec![3.0, 6.0]));

        let mut fixed = MultiArray::fixed(PointerWidth::U8, 6);
        fixed.push(1.0).unwrap();
        fixed.push(2.0).unwrap();
        fixed.push(3.0).unwrap();

        assert_eq!(fixed.size(), 3);
        assert_eq!(fixed.dimension(), 3);
        assert_eq!(fixed.get(0), Some(vec![1.0]));

        fixed.set(0, 4.0).unwrap();
        fixed.set(1, 5.0).unwrap();
        fixed.set(2, 6.0).unwrap();

        assert_eq!(fixed.size(), 6);
        assert_eq!(fixed.get(1), Some(vec![2.0, 5.0]));

        assert_eq!(fixed.push(45.0), Err(CapacityExceeded));
    }

    /// "get subarrays": `get` past `dimension` is `None`; a `has`/`get`
    /// contrast for indices below and at `dimension`.
    #[test]
    fn get_returns_none_past_dimension_and_the_bucket_otherwise() {
        let array =
            dynamic_from_sets(&[(0, 1.0), (0, 2.0), (0, 3.0), (1, 4.0), (1, 5.0), (2, 6.0)]);

        assert_eq!(array.get(4), None);
        assert_eq!(array.get(0), Some(vec![1.0, 2.0, 3.0]));
        assert_eq!(array.get(1), Some(vec![4.0, 5.0]));
        assert_eq!(array.get(2), Some(vec![6.0]));

        assert!(!array.has(4));
        assert!(array.has(0));
        assert!(array.has(1));
        assert!(array.has(2));
    }

    #[test]
    fn has_and_multiplicity_agree_with_upstream() {
        let array = dynamic_from_sets(&[(0, 4.0), (0, 5.0)]);

        assert!(array.has(0));
        assert!(!array.has(3));
        assert!(!array.has(1));

        assert_eq!(array.multiplicity(0), 2);
        assert_eq!(array.multiplicity(3), 0);
        assert_eq!(array.multiplicity(1), 0);
    }

    /// "insert in random order": `set` jumping straight to index 34 grows
    /// `dimension` to 35, leaving every untouched index in between a real,
    /// zero-length gap rather than absent.
    #[test]
    fn inserting_out_of_order_leaves_a_real_gap_at_dimension() {
        let mut array = MultiArray::new();
        array.set(34, 3.0).unwrap();
        array.set(2, 4.0).unwrap();
        array.set(2, 5.0).unwrap();

        assert_eq!(array.size(), 3);
        assert_eq!(array.dimension(), 35);
        assert_eq!(array.get(2), Some(vec![4.0, 5.0]));
        assert_eq!(array.get(34), Some(vec![3.0]));
        // A gap: below `dimension`, never set, but present (empty) rather
        // than `None`.
        assert_eq!(array.get(10), Some(vec![]));
        assert_eq!(array.multiplicity(10), 0);

        let mut fixed = MultiArray::fixed(PointerWidth::U8, 40);
        fixed.set(34, 3.0).unwrap();
        fixed.set(2, 4.0).unwrap();
        fixed.set(2, 5.0).unwrap();

        assert_eq!(fixed.size(), 3);
        assert_eq!(fixed.dimension(), 35);
        assert_eq!(fixed.get(2), Some(vec![4.0, 5.0]));
        assert_eq!(fixed.get(34), Some(vec![3.0]));
    }

    #[test]
    fn containers_and_associations_walk_dimension_in_gets_order() {
        let array =
            dynamic_from_sets(&[(0, 1.0), (0, 2.0), (0, 3.0), (1, 4.0), (1, 5.0), (2, 6.0)]);

        assert_eq!(
            array.containers(),
            vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0], vec![6.0]]
        );
        assert_eq!(
            array.associations(),
            vec![
                (0, vec![1.0, 2.0, 3.0]),
                (1, vec![4.0, 5.0]),
                (2, vec![6.0]),
            ]
        );
    }

    /// `values()`/`values_at` — global insertion order vs. per-bucket
    /// **reverse**-insertion order. The two must not be confused with
    /// `get`'s forward order; this is the sharpest place a transcription
    /// error would hide.
    #[test]
    fn values_are_global_insertion_order_or_reversed_per_bucket() {
        let array =
            dynamic_from_sets(&[(0, 1.0), (0, 2.0), (0, 3.0), (1, 4.0), (1, 5.0), (2, 6.0)]);

        assert_eq!(array.values(), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(array.values_at(0), vec![3.0, 2.0, 1.0]);
        assert_eq!(array.values_at(1), vec![5.0, 4.0]);
        assert_eq!(array.values_at(2), vec![6.0]);
        assert_eq!(array.values_at(3), Vec::<f64>::new());
    }

    #[test]
    fn entries_walk_each_bucket_tail_to_head_in_dimension_order() {
        let array =
            dynamic_from_sets(&[(0, 1.0), (0, 2.0), (0, 3.0), (1, 4.0), (1, 5.0), (2, 6.0)]);

        assert_eq!(
            array.entries(),
            vec![(0, 3.0), (0, 2.0), (0, 1.0), (1, 5.0), (1, 4.0), (2, 6.0)]
        );
    }

    #[test]
    fn keys_is_the_dimension_range() {
        let array =
            dynamic_from_sets(&[(0, 1.0), (0, 2.0), (0, 3.0), (1, 4.0), (1, 5.0), (2, 6.0)]);

        assert_eq!(array.keys(), vec![0, 1, 2]);
    }

    /// A fixed-capacity container narrows values to its width, exactly as a
    /// real typed-array store does — untested by `test/multi-array.js` (its
    /// own values never overflow a byte) but load-bearing for the bridge and
    /// the differential fuzzer.
    #[test]
    fn fixed_capacity_values_narrow_to_their_width() {
        let mut array = MultiArray::fixed(PointerWidth::U8, 2);
        array.push(300.0).unwrap();

        assert_eq!(array.get(0), Some(vec![300.0 % 256.0]));
    }

    #[test]
    fn an_empty_multi_array_has_no_containers_or_values() {
        let array = MultiArray::new();

        assert_eq!(array.size(), 0);
        assert_eq!(array.dimension(), 0);
        assert_eq!(array.get(0), None);
        assert!(array.containers().is_empty());
        assert!(array.values().is_empty());
        assert!(array.keys().is_empty());
    }
}

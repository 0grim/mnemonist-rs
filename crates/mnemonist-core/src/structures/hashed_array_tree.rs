//! Port of upstream `hashed-array-tree.js` (mnemonist v0.40.4).
//!
//! A dynamically growing array built from a vector of fixed-size *blocks*,
//! rather than from one buffer that is reallocated and copied. Growth appends a
//! block; nothing is ever moved. Because `blockSize` is required to be a power
//! of two, the index split is two bit operations:
//!
//! ```text
//! block  = index >> blockMask      // blockMask = log2(blockSize)
//! offset = index &  offsetMask     // offsetMask = blockSize - 1
//! ```
//!
//! # The bounds checks are off by one, in three different directions
//!
//! Upstream guards `set` and `get` with `if (this.length < index)`, which
//! admits `index === this.length` — one past the last live element. Three
//! separate behaviours fall out of that, all verified against Node 24.18.1:
//!
//! | call | upstream | why |
//! |---|---|---|
//! | `get(length)` | the raw block byte, normally `0` | the guard is `<`, not `<=` |
//! | `get(length + 1)` | `undefined` | the guard fires |
//! | `set(length, v)` | **writes**, and `length` does not move | the guard is `<` |
//! | `get`/`set(capacity)` when `length == capacity` | **`TypeError`** | `blocks[capacity >> blockMask]` is `undefined` |
//!
//! So a brand-new tree answers `get(0)` with `0` rather than `undefined`, even
//! though it holds nothing — which is exactly what upstream's own
//! "should return undefined on out-of-bound values" test would have caught had
//! it asked for index `0` instead of index `2`.
//!
//! # `pop` reads the wrong block
//!
//! This is the sharpest defect in the file:
//!
//! ```js
//! var lastBlock = this.blocks[this.blocks.length - 1];   // the LAST block
//! var i = (--this.length) & this.offsetMask;             // offset of the popped index
//! return lastBlock[i];
//! ```
//!
//! The offset is computed from the popped index but the *block* is always the
//! last allocated one. They agree only while everything lives in one block —
//! which is the entire coverage of upstream's test, since it uses the default
//! 1024-element blocks and pushes twice. With `blockSize: 2` and three pushes,
//! measured on Node: `pop()` yields `3`, then `0`, then `3`. The value `2` is
//! unreachable and `3` is returned twice. See [`HashedArrayTree::pop`].
//!
//! Both are reproduced here rather than fixed; see `docs/modules/hashed-array-tree.md`.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::structures::hashed_array_tree::{HashedArrayTree, Options};
//! use mnemonist_core::utils::typed_arrays::PointerWidth;
//!
//! let mut array = HashedArrayTree::new(
//!     PointerWidth::U8,
//!     Options::with_block_size(128),
//! )
//! .unwrap();
//!
//! for value in 0..250 {
//!     array.push(value);
//! }
//!
//! assert_eq!(array.length(), 250);
//! assert_eq!(array.capacity(), 256);
//! assert_eq!(array.get(34), Ok(Some(34)));
//! ```

use core::fmt;

use crate::utils::typed_arrays::{PointerVec, PointerWidth};

/// `DEFAULT_BLOCK_SIZE` upstream.
pub const DEFAULT_BLOCK_SIZE: usize = 1024;

/// Upstream throw message for a missing array constructor, verbatim.
///
/// Raised by the bridge rather than here: it keys off `arguments.length`, which
/// is a JavaScript-only notion. The string lives in the core so the two cannot
/// drift.
pub const MISSING_ARRAY_CLASS: &str =
    "mnemonist/hashed-array-tree: expecting at least a byte array constructor.";

/// Upstream throw message for a block size that is not a power of two.
pub const BLOCK_SIZE_NOT_POWER_OF_TWO: &str =
    "mnemonist/hashed-array-tree: block size should be a power of two.";

/// A failure with an upstream-identical message.
///
/// [`fmt::Display`] renders exactly what upstream throws, so the bridge can
/// hand the string to `Error::new` and a JS caller's `assert.throws(fn, /…/)`
/// matches on the same text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `new HashedArrayTree()` with no arguments.
    MissingArrayClass,
    /// A falsy or non-power-of-two `blockSize`.
    BlockSizeNotPowerOfTwo,
    /// A `blockSize` upstream accepts but this port cannot represent.
    ///
    /// Not an upstream message: upstream's `powerOfTwo` test runs on the
    /// ToInt32 of its argument, so `blockSize: 2**32` passes it and yields
    /// `blockMask === 32`, at which point `index >> 32` is `index >> 0` in
    /// JavaScript and the structure stops being a hashed array tree at all.
    /// Refused here instead of reproduced; see the divergence doc.
    BlockSizeUnsupported,
    /// `set` past `length`: `HashedArrayTree(<class>).set: index out of bounds.`
    IndexOutOfBounds {
        /// The `ArrayClass` name upstream would name in the message.
        class: &'static str,
    },
    /// Indexing a block that was never allocated.
    ///
    /// Upstream reaches `blocks[b]` with `b === blocks.length` and gets
    /// `undefined`, then indexes it — a `TypeError`. The message is V8's, and
    /// is reproduced verbatim so the two sides compare equal in the
    /// differential fuzzer. It is therefore tied to Node 24.18.1.
    UnallocatedBlock {
        /// `true` for a store, `false` for a load. V8 words the two
        /// `TypeError`s differently and both wordings are reproduced.
        writing: bool,
        /// The within-block offset the access used, which V8 quotes in the
        /// message.
        offset: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingArrayClass => formatter.write_str(MISSING_ARRAY_CLASS),
            Self::BlockSizeNotPowerOfTwo => formatter.write_str(BLOCK_SIZE_NOT_POWER_OF_TWO),
            Self::BlockSizeUnsupported => formatter.write_str(
                "mnemonist-rs/hashed-array-tree: block size must be a power of two below 2^31.",
            ),
            Self::IndexOutOfBounds { class } => {
                write!(
                    formatter,
                    "HashedArrayTree({class}).set: index out of bounds."
                )
            }
            // V8's wording, reproduced exactly.
            Self::UnallocatedBlock {
                writing: true,
                offset,
            } => write!(
                formatter,
                "Cannot set properties of undefined (setting '{offset}')"
            ),
            Self::UnallocatedBlock {
                writing: false,
                offset,
            } => write!(
                formatter,
                "Cannot read properties of undefined (reading '{offset}')"
            ),
        }
    }
}

/// The object form of upstream's second constructor argument.
///
/// Upstream accepts either a number (an initial capacity) or an options object,
/// and every field is read as `x.field || default` — so a `0` falls back to the
/// default rather than being honoured. [`Options::from_capacity`] is the number
/// form; the field defaults here are the object form's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Slots to allocate up front — upstream's `initialCapacity`. A `0` here
    /// means "use the default", because upstream reads it as `x || default`.
    pub initial_capacity: usize,
    /// Elements to consider live from the start — upstream's `initialLength`.
    /// Also implies at least that much capacity.
    pub initial_length: usize,
    /// Elements per block — upstream's `blockSize`. Must be a power of two.
    pub block_size: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            initial_capacity: 0,
            initial_length: 0,
            block_size: DEFAULT_BLOCK_SIZE,
        }
    }
}

impl Options {
    /// Upstream's `new HashedArrayTree(Class, 5)` form.
    pub fn from_capacity(initial_capacity: usize) -> Self {
        Self {
            initial_capacity,
            ..Self::default()
        }
    }

    /// Upstream's `{blockSize: n}` form.
    pub fn with_block_size(block_size: usize) -> Self {
        Self {
            block_size,
            ..Self::default()
        }
    }

    /// Upstream's `{initialLength: n}` form.
    pub fn with_initial_length(initial_length: usize) -> Self {
        Self {
            initial_length,
            ..Self::default()
        }
    }
}

/// `powerOfTwo(x)` — upstream's guard, including its ToInt32 truncation.
///
/// `(x & (x - 1)) === 0` in JavaScript converts both operands to *signed*
/// 32-bit integers first, so the test is really about `x mod 2^32`. Reproduced
/// with the same wrapping cast: `2**32` passes here exactly as it does upstream
/// (verified against Node), and [`Error::BlockSizeUnsupported`] then refuses it
/// on the next line rather than pretending the shift would work.
fn power_of_two(x: usize) -> bool {
    let truncated = x as u32 as i32;

    truncated & truncated.wrapping_sub(1) == 0
}

/// A dynamically growing array of fixed-size blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashedArrayTree {
    class: PointerWidth,
    length: usize,
    capacity: usize,
    block_size: usize,
    offset_mask: usize,
    block_mask: u32,
    blocks: Vec<PointerVec>,
}

impl HashedArrayTree {
    /// Build a tree of `class` elements with the given options.
    ///
    /// # Errors
    ///
    /// [`Error::BlockSizeNotPowerOfTwo`] where upstream throws, and
    /// [`Error::BlockSizeUnsupported`] for the block sizes upstream accepts but
    /// this port declines to misrepresent.
    pub fn new(class: PointerWidth, options: Options) -> Result<Self, Error> {
        // `if (!blockSize || !powerOfTwo(blockSize))`. The falsy half is why a
        // `blockSize` of 0 in the options object does NOT reach this check at
        // all — `options.blockSize || DEFAULT_BLOCK_SIZE` has already replaced
        // it. Zero only arrives here from a Rust caller building `Options`
        // directly, and upstream's guard rejects it, so it is rejected here.
        if options.block_size == 0 || !power_of_two(options.block_size) {
            return Err(Error::BlockSizeNotPowerOfTwo);
        }

        // Below 2^31 the shift is a real shift in both languages. At or above
        // it, JavaScript's `index >> blockMask` starts taking `blockMask mod 32`
        // and the structure degenerates; see `Error::BlockSizeUnsupported`.
        if options.block_size > 1 << 30 {
            return Err(Error::BlockSizeUnsupported);
        }

        // `Math.max(initialLength, initialCapacity)`, then `Math.ceil(/ blockSize)`.
        let requested = options.initial_length.max(options.initial_capacity);
        let initial_blocks = requested.div_ceil(options.block_size);

        Ok(Self {
            class,
            length: options.initial_length,
            capacity: initial_blocks * options.block_size,
            block_size: options.block_size,
            offset_mask: options.block_size - 1,
            // `Math.log2(blockSize)`, exact because the value is a power of two.
            block_mask: options.block_size.trailing_zeros(),
            blocks: (0..initial_blocks)
                .map(|_| PointerVec::zeroed(class, options.block_size))
                .collect(),
        })
    }

    /// Number of live elements — upstream's `length`.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Allocated slots — upstream's `capacity`, always `blocks * blockSize`.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Elements per block — upstream's `blockSize`, always a power of two.
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// `blockSize - 1`, the low-bit mask upstream stores.
    pub fn offset_mask(&self) -> usize {
        self.offset_mask
    }

    /// `Math.log2(blockSize)`, the shift upstream stores.
    pub fn block_mask(&self) -> u32 {
        self.block_mask
    }

    /// Element width, standing in for upstream's `ArrayClass`.
    pub fn class(&self) -> PointerWidth {
        self.class
    }

    /// The JS constructor name upstream would report, for error messages.
    pub fn class_name(&self) -> &'static str {
        class_name(self.class)
    }

    /// The blocks, exposed because upstream's `blocks` is a public property and
    /// the differential fuzzer compares it block for block.
    pub fn blocks(&self) -> &[PointerVec] {
        &self.blocks
    }

    /// Write `value` at `index`.
    ///
    /// # Errors
    ///
    /// [`Error::IndexOutOfBounds`] for `index > length` — note the strict
    /// comparison, which is upstream's and which lets `index == length` through
    /// to a write that does not move `length`.
    ///
    /// [`Error::UnallocatedBlock`] when that admitted `index` is also
    /// `capacity`, where upstream indexes an absent block and raises a
    /// `TypeError`.
    pub fn set(&mut self, index: usize, value: u32) -> Result<(), Error> {
        // `if (this.length < index) throw`.
        if self.length < index {
            return Err(Error::IndexOutOfBounds {
                class: class_name(self.class),
            });
        }

        let (block, offset) = self.split(index);

        match self.blocks.get_mut(block) {
            // A typed-array store: truncating, never growing.
            Some(values) => {
                values.set(offset, value);
                Ok(())
            }
            None => Err(Error::UnallocatedBlock {
                writing: true,
                offset,
            }),
        }
    }

    /// Read `index`, or `Ok(None)` where upstream returns `undefined`.
    ///
    /// `Ok(None)` is only the `length < index` branch. `index == length` is
    /// *not* out of bounds upstream, so it reads the block byte and normally
    /// answers `Some(0)` — including on an empty tree.
    ///
    /// # Errors
    ///
    /// [`Error::UnallocatedBlock`], as [`set`](HashedArrayTree::set).
    pub fn get(&self, index: usize) -> Result<Option<u32>, Error> {
        // `if (this.length < index) return;`
        if self.length < index {
            return Ok(None);
        }

        let (block, offset) = self.split(index);

        match self.blocks.get(block) {
            Some(values) => Ok(Some(values.get(offset))),
            None => Err(Error::UnallocatedBlock {
                writing: false,
                offset,
            }),
        }
    }

    /// Append blocks until the tree holds at least `capacity` slots.
    ///
    /// `None` is upstream's `typeof capacity !== 'number'` branch — a bare
    /// `grow()` — which adds exactly one block.
    pub fn grow(&mut self, capacity: Option<usize>) {
        let target = capacity.unwrap_or(self.capacity + self.block_size);

        if self.capacity >= target {
            return;
        }

        while self.capacity < target {
            self.blocks
                .push(PointerVec::zeroed(self.class, self.block_size));
            self.capacity += self.block_size;
        }
    }

    /// Set `length`, growing if it has to. Never deallocates.
    ///
    /// Shrinking leaves both the blocks and their contents in place, so the
    /// dropped elements are still readable through [`get`](HashedArrayTree::get)
    /// at `index == length` and still reachable by
    /// [`pop`](HashedArrayTree::pop)'s wrong-block read.
    pub fn resize(&mut self, length: usize) {
        if length == self.length {
            return;
        }

        if length < self.length {
            self.length = length;
            return;
        }

        self.length = length;
        self.grow(Some(length));
    }

    /// Append `value`, returning the new length.
    pub fn push(&mut self, value: u32) -> usize {
        if self.capacity == self.length {
            self.grow(None);
        }

        let index = self.length;
        let (block, offset) = self.split(index);

        // `this.blocks[block][i] = value`. Unlike `set`, this cannot miss a
        // block: the growth above guarantees `index < capacity`, and `capacity`
        // is always `blocks.len() * block_size`.
        if let Some(values) = self.blocks.get_mut(block) {
            values.set(offset, value);
        }

        self.length += 1;
        self.length
    }

    /// Remove and return the last element — **from the last allocated block**.
    ///
    /// Bug-for-bug. Upstream computes the offset from the popped index but
    /// takes the block unconditionally from the end of `blocks`, so the two
    /// agree only while the tree occupies a single block. Measured on Node with
    /// `blockSize: 2` after pushing `1, 2, 3`:
    ///
    /// ```text
    /// blocks = [[1, 2], [3, 0]]
    /// pop() -> 3   // index 2, offset 0, last block -> correct by luck
    /// pop() -> 0   // index 1, offset 1, last block -> reads the padding
    /// pop() -> 3   // index 0, offset 0, last block -> yields 3 a second time
    /// ```
    ///
    /// `length` is still decremented correctly, so only the returned value is
    /// wrong. Fixing it would be a silent behavioural divergence, so it is
    /// reproduced and pinned by a test instead. See BUG-SPARSE-MAP-1.
    pub fn pop(&mut self) -> Option<u32> {
        if self.length == 0 {
            return None;
        }

        self.length -= 1;
        let offset = self.length & self.offset_mask;

        // `this.blocks[this.blocks.length - 1]` is `undefined` for an empty
        // block list, which upstream would then index and raise on. That state
        // needs `length > 0` with `capacity == 0`, which no public call
        // sequence produces: every path that raises `length` allocates first.
        self.blocks.last().map(|values| values.get(offset))
    }

    /// `[index >> blockMask, index & offsetMask]`.
    fn split(&self, index: usize) -> (usize, usize) {
        (index >> self.block_mask, index & self.offset_mask)
    }
}

/// The JS constructor name a width stands in for.
///
/// Kept local rather than added to [`crate::utils::typed_arrays`] because it is
/// a *JavaScript* name, and only the modules that reproduce an upstream error
/// message need it.
fn class_name(class: PointerWidth) -> &'static str {
    match class {
        PointerWidth::U8 => "Uint8Array",
        PointerWidth::U16 => "Uint16Array",
        PointerWidth::U32 => "Uint32Array",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(block_size: usize) -> HashedArrayTree {
        HashedArrayTree::new(PointerWidth::U8, Options::with_block_size(block_size)).unwrap()
    }

    /// 1:1 port of every upstream `it` block, as a baseline.
    #[test]
    fn reproduces_the_upstream_suite() {
        assert_eq!(
            HashedArrayTree::new(PointerWidth::U8, Options::with_block_size(27)).unwrap_err(),
            Error::BlockSizeNotPowerOfTwo
        );

        let array = HashedArrayTree::new(PointerWidth::U8, Options::from_capacity(5)).unwrap();
        assert_eq!(array.length(), 0);
        assert_eq!(array.capacity(), 1024);

        let mut array =
            HashedArrayTree::new(PointerWidth::U8, Options::with_initial_length(3)).unwrap();
        array.set(2, 24).unwrap();
        assert_eq!(array.length(), 3);
        assert_eq!(array.get(2), Ok(Some(24)));

        let array = HashedArrayTree::new(PointerWidth::U8, Options::from_capacity(5)).unwrap();
        assert_eq!(array.get(2), Ok(None));

        let mut array = HashedArrayTree::new(PointerWidth::U8, Options::from_capacity(4)).unwrap();
        assert_eq!(
            array.set(56, 4).unwrap_err(),
            Error::IndexOutOfBounds {
                class: "Uint8Array"
            }
        );

        let mut array = tree(128);
        for value in 0..250 {
            array.push(value);
        }
        assert_eq!(array.length(), 250);
        assert_eq!(array.capacity(), 256);
        assert_eq!(array.get(34), Ok(Some(34)));

        let mut array = HashedArrayTree::new(PointerWidth::U32, Options::default()).unwrap();
        array.push(1);
        array.push(2);
        assert_eq!(array.pop(), Some(2));
        assert_eq!(array.length(), 1);
        assert_eq!(array.pop(), Some(1));
        assert_eq!(array.length(), 0);
        assert_eq!(array.pop(), None);
        assert_eq!(array.length(), 0);
        array.push(34);
        array.push(35);
        assert_eq!(array.get(1), Ok(Some(35)));
        assert_eq!(array.length(), 2);

        let mut array = tree(2);
        array.grow(Some(5));
        assert_eq!(array.capacity(), 6);
        array.grow(Some(2));
        assert_eq!(array.capacity(), 6);
        array.grow(None);
        assert_eq!(array.capacity(), 8);

        let mut array = HashedArrayTree::new(
            PointerWidth::U8,
            Options {
                initial_length: 23,
                block_size: 8,
                ..Options::default()
            },
        )
        .unwrap();
        array.resize(20);
        assert_eq!((array.capacity(), array.length()), (24, 20));
        array.resize(30);
        assert_eq!((array.capacity(), array.length()), (32, 30));
    }

    /// Gap: `pop` takes the offset from the popped index and the block from the
    /// end of `blocks`. Upstream's own test never leaves the first block, so
    /// the two never disagree there.
    ///
    /// Measured against Node 24.18.1: `3`, `0`, `3`.
    #[test]
    fn pop_reads_the_last_block_rather_than_the_popped_index_s_block() {
        let mut array = tree(2);

        array.push(1);
        array.push(2);
        array.push(3);

        assert_eq!(array.blocks()[0], PointerVec::U8(vec![1, 2]));
        assert_eq!(array.blocks()[1], PointerVec::U8(vec![3, 0]));

        assert_eq!(array.pop(), Some(3));
        // Offset 1 of the LAST block, which is padding — not the `2` at
        // index 1.
        assert_eq!(array.pop(), Some(0));
        // And `3` comes back a second time.
        assert_eq!(array.pop(), Some(3));
        assert_eq!(array.length(), 0);
        assert_eq!(array.pop(), None);
    }

    /// The same defect reached through `resize` rather than growth: shrinking
    /// does not deallocate, so `blocks.last()` stays far ahead of `length`.
    #[test]
    fn pop_after_a_shrinking_resize_reads_a_block_that_is_no_longer_live() {
        let mut array = tree(2);

        for value in [7, 8, 9, 10] {
            array.push(value);
        }
        array.resize(1);

        // Offset 0 of block 1, which holds 9 — not the 7 at index 0.
        assert_eq!(array.pop(), Some(9));
        assert_eq!(array.length(), 0);
    }

    /// Gap: the `length < index` guard is strict, so `index == length` is a
    /// legal read of a slot that holds nothing.
    #[test]
    fn get_at_length_reads_the_block_instead_of_reporting_absence() {
        let array = HashedArrayTree::new(PointerWidth::U8, Options::from_capacity(5)).unwrap();

        // Upstream's own test asserts `get(2) === undefined` here; `get(0)` on
        // the same empty tree is `0`.
        assert_eq!(array.length(), 0);
        assert_eq!(array.get(0), Ok(Some(0)));
        assert_eq!(array.get(1), Ok(None));
        assert_eq!(array.get(2), Ok(None));
    }

    /// And the write half: `set(length, v)` lands, and `length` does not move,
    /// so the value is invisible to `pop` but visible to `get(length)`.
    #[test]
    fn set_at_length_writes_a_slot_that_length_does_not_cover() {
        let mut array = HashedArrayTree::new(
            PointerWidth::U8,
            Options {
                initial_length: 3,
                block_size: 8,
                ..Options::default()
            },
        )
        .unwrap();

        array.set(3, 99).unwrap();

        assert_eq!(array.length(), 3);
        assert_eq!(array.get(3), Ok(Some(99)));
        assert_eq!(array.get(4), Ok(None));
    }

    /// Gap: when the admitted `index == length` is also `capacity`, upstream
    /// indexes a block that does not exist. Verified against Node, which raises
    /// `TypeError: Cannot set properties of undefined (setting '0')`.
    #[test]
    fn indexing_at_capacity_raises_the_typeerror_upstream_raises() {
        let mut array = HashedArrayTree::new(
            PointerWidth::U8,
            Options {
                initial_length: 4,
                block_size: 2,
                ..Options::default()
            },
        )
        .unwrap();

        assert_eq!(
            (array.length(), array.capacity(), array.blocks().len()),
            (4, 4, 2)
        );

        assert_eq!(
            array.set(4, 1).unwrap_err(),
            Error::UnallocatedBlock {
                writing: true,
                offset: 0
            }
        );
        assert_eq!(
            array.get(4).unwrap_err(),
            Error::UnallocatedBlock {
                writing: false,
                offset: 0
            }
        );

        assert_eq!(
            array.set(4, 1).unwrap_err().to_string(),
            "Cannot set properties of undefined (setting '0')"
        );
        assert_eq!(
            array.get(4).unwrap_err().to_string(),
            "Cannot read properties of undefined (reading '0')"
        );
    }

    /// Gap: the error message embeds the array class, and upstream's test only
    /// matches `/bounds/`.
    #[test]
    fn the_out_of_bounds_message_names_the_array_class() {
        for (class, name) in [
            (PointerWidth::U8, "Uint8Array"),
            (PointerWidth::U16, "Uint16Array"),
            (PointerWidth::U32, "Uint32Array"),
        ] {
            let mut array = HashedArrayTree::new(class, Options::from_capacity(4)).unwrap();

            assert_eq!(
                array.set(56, 4).unwrap_err().to_string(),
                format!("HashedArrayTree({name}).set: index out of bounds.")
            );
        }
    }

    /// Gap: stores truncate at the element width. Upstream's test never pushes
    /// a value above 250 into its `Uint8Array`.
    #[test]
    fn stores_truncate_at_the_element_width() {
        let mut narrow = tree(2);
        narrow.push(300);
        assert_eq!(narrow.get(0), Ok(Some(300 % 256)));

        let mut wide =
            HashedArrayTree::new(PointerWidth::U16, Options::with_block_size(2)).unwrap();
        wide.push(70_000);
        assert_eq!(wide.get(0), Ok(Some(70_000 % 65_536)));

        let mut widest =
            HashedArrayTree::new(PointerWidth::U32, Options::with_block_size(2)).unwrap();
        widest.push(u32::MAX);
        assert_eq!(widest.get(0), Ok(Some(u32::MAX)));
    }

    /// Gap: the derived constants. Upstream asserts `capacity` in two places
    /// and never looks at `blockMask` or `offsetMask`, which are what make the
    /// index split work.
    #[test]
    fn derives_the_index_split_constants_from_the_block_size() {
        for (block_size, mask, offset) in [
            (1usize, 0u32, 0usize),
            (2, 1, 1),
            (128, 7, 127),
            (1024, 10, 1023),
        ] {
            let array = tree(block_size);

            assert_eq!(array.block_size(), block_size);
            assert_eq!(array.block_mask(), mask, "block size {block_size}");
            assert_eq!(array.offset_mask(), offset, "block size {block_size}");
        }
    }

    /// Gap: `grow` with no argument on a tree that has never allocated.
    #[test]
    fn a_bare_grow_adds_exactly_one_block() {
        let mut array = tree(2);

        assert_eq!((array.capacity(), array.blocks().len()), (0, 0));

        array.grow(None);
        assert_eq!((array.capacity(), array.blocks().len()), (2, 1));

        array.grow(None);
        assert_eq!((array.capacity(), array.blocks().len()), (4, 2));

        // A target already covered is a no-op, including an exact match.
        array.grow(Some(4));
        assert_eq!(array.capacity(), 4);
        array.grow(Some(0));
        assert_eq!(array.capacity(), 4);
    }

    /// Gap: `resize` down never deallocates, and re-growing reuses the blocks
    /// with their old contents intact.
    #[test]
    fn a_shrinking_resize_keeps_the_blocks_and_their_contents() {
        let mut array = tree(2);

        for value in [1, 2, 3, 4] {
            array.push(value);
        }
        assert_eq!(
            (array.length(), array.capacity(), array.blocks().len()),
            (4, 4, 2)
        );

        array.resize(2);
        assert_eq!(
            (array.length(), array.capacity(), array.blocks().len()),
            (2, 4, 2)
        );

        // The dropped elements are still there and still readable at `length`.
        assert_eq!(array.get(2), Ok(Some(3)));

        // Growing back does not clear them: upstream re-exposes the stale data.
        array.resize(4);
        assert_eq!(array.get(3), Ok(Some(4)));
        assert_eq!((array.capacity(), array.blocks().len()), (4, 2));

        // resize to the current length is a no-op.
        array.resize(4);
        assert_eq!(array.length(), 4);
    }

    /// Gap: `push` after a shrinking `resize` overwrites rather than appends,
    /// which is the only way the stale data becomes reachable again by index.
    #[test]
    fn push_after_a_shrinking_resize_overwrites_the_stale_slot() {
        let mut array = tree(4);

        for value in [1, 2, 3] {
            array.push(value);
        }
        array.resize(1);
        assert_eq!(array.push(9), 2);

        assert_eq!(array.get(0), Ok(Some(1)));
        assert_eq!(array.get(1), Ok(Some(9)));
        // Index 2 still holds the 3 that `resize` dropped.
        assert_eq!(array.get(2), Ok(Some(3)));
    }

    /// Gap: the capacity arithmetic when both `initialLength` and
    /// `initialCapacity` are given. Upstream takes the larger and rounds up.
    #[test]
    fn initial_capacity_is_the_larger_of_the_two_rounded_up_to_a_block() {
        for (initial_length, initial_capacity, block_size, blocks) in [
            (0usize, 0usize, 8usize, 0usize),
            (1, 0, 8, 1),
            (0, 1, 8, 1),
            (8, 3, 8, 1),
            (9, 3, 8, 2),
            (3, 9, 8, 2),
            (23, 0, 8, 3),
        ] {
            let array = HashedArrayTree::new(
                PointerWidth::U8,
                Options {
                    initial_length,
                    initial_capacity,
                    block_size,
                },
            )
            .unwrap();

            assert_eq!(
                array.blocks().len(),
                blocks,
                "length {initial_length} capacity {initial_capacity} block {block_size}"
            );
            assert_eq!(array.capacity(), blocks * block_size);
            assert_eq!(array.length(), initial_length);
        }
    }

    /// Gap: only 27 is ever rejected upstream. The guard is a ToInt32 test, so
    /// its boundaries are worth pinning.
    #[test]
    fn rejects_every_non_power_of_two_block_size() {
        for block_size in [0usize, 3, 5, 6, 7, 9, 27, 100, 1000, 1023, 1025] {
            assert_eq!(
                HashedArrayTree::new(PointerWidth::U8, Options::with_block_size(block_size))
                    .unwrap_err(),
                Error::BlockSizeNotPowerOfTwo,
                "block size {block_size}"
            );
        }

        for block_size in [1usize, 2, 4, 8, 1024, 1 << 20, 1 << 30] {
            assert!(
                HashedArrayTree::new(PointerWidth::U8, Options::with_block_size(block_size))
                    .is_ok(),
                "block size {block_size}"
            );
        }
    }

    /// The ToInt32 in upstream's guard: `2**32` passes `powerOfTwo` because
    /// both operands truncate to 32 bits. Verified against Node, where the
    /// constructor succeeds and sets `blockMask` to 32. Refused here.
    #[test]
    fn a_block_size_upstream_only_accepts_by_truncation_is_refused() {
        assert!(power_of_two(1usize << 32));

        assert_eq!(
            HashedArrayTree::new(PointerWidth::U8, Options::with_block_size(1 << 32)).unwrap_err(),
            Error::BlockSizeUnsupported
        );
        assert_eq!(
            HashedArrayTree::new(PointerWidth::U8, Options::with_block_size(1 << 31)).unwrap_err(),
            Error::BlockSizeUnsupported
        );
    }

    /// Gap: a block size of one, where every element is its own block.
    #[test]
    fn a_block_size_of_one_gives_every_element_its_own_block() {
        let mut array = tree(1);

        for value in [4, 5, 6] {
            array.push(value);
        }

        assert_eq!(array.blocks().len(), 3);
        assert_eq!(array.capacity(), 3);
        assert_eq!(array.get(1), Ok(Some(5)));
        // Still the last-block read, but with one element per block it happens
        // to be right for the first pop and wrong afterwards.
        assert_eq!(array.pop(), Some(6));
        assert_eq!(array.pop(), Some(6));
    }

    #[test]
    fn a_fresh_tree_pops_nothing() {
        let mut array = tree(2);

        assert_eq!(array.pop(), None);
        assert_eq!(array.length(), 0);
        assert_eq!(array.blocks().len(), 0);
    }

    /// Gap: crossing a block boundary by pushing, which upstream's test does
    /// (250 pushes into 128-element blocks) without ever checking the boundary
    /// elements themselves.
    #[test]
    fn indexes_across_block_boundaries() {
        let mut array =
            HashedArrayTree::new(PointerWidth::U16, Options::with_block_size(4)).unwrap();

        for value in 0..10 {
            array.push(value * 10);
        }

        assert_eq!(array.blocks().len(), 3);
        for index in 0..10u32 {
            assert_eq!(
                array.get(index as usize),
                Ok(Some(index * 10)),
                "index {index}"
            );
        }
        assert_eq!(array.blocks()[0], PointerVec::U16(vec![0, 10, 20, 30]));
        assert_eq!(array.blocks()[2], PointerVec::U16(vec![80, 90, 0, 0]));
    }
}

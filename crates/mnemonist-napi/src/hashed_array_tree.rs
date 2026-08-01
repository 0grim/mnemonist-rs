//! JS bridge for [`mnemonist_core::structures::hashed_array_tree`].
//!
//! Thin translation only: every behavioural decision lives in the core crate.
//! Four adaptations are worth knowing about.
//!
//! 1. **`ArrayClass` arrives as a JS constructor and leaves as a
//!    [`PointerWidth`].** Rust has no runtime constructor value, so the class is
//!    identified by its `name` property and mapped. `Uint8Array`,
//!    `Uint16Array` and `Uint32Array` are supported; anything else is refused
//!    here rather than silently reinterpreted. Upstream would accept, say,
//!    `Float64Array` and store doubles.
//! 2. **`arguments.length < 1` is checked here**, because it is a
//!    JavaScript-only notion. The message is
//!    [`mnemonist_core::structures::hashed_array_tree::MISSING_ARRAY_CLASS`], so
//!    the two cannot drift.
//! 3. **`blocks` is not exposed.** It is a public array of typed arrays
//!    upstream and a JS caller can write *through* it; napi can only hand out a
//!    copy, which would silently break the write-through. Same call as the
//!    `SparseSet` bridge makes for `dense`/`sparse`. The original test file
//!    never reads it, and the differential fuzzer compares it block for block
//!    on the Rust side.
//! 4. **Values are coerced with JS typed-array store semantics.** `f64` in,
//!    ToUint32, then the core's narrowing store. `array.push(300)` on a
//!    `Uint8Array` tree stores `44` in both languages; a plain `as u8` would
//!    have saturated at `255`.

use mnemonist_core::structures::hashed_array_tree::{
    Error as CoreError, HashedArrayTree as CoreTree, Options, MISSING_ARRAY_CLASS,
};
use mnemonist_core::utils::typed_arrays::PointerWidth;
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// A dynamically growing array of fixed-size blocks.
#[napi(js_name = "HashedArrayTree")]
pub struct JsHashedArrayTree {
    inner: CoreTree,
}

#[napi]
impl JsHashedArrayTree {
    /// `new HashedArrayTree(ArrayClass, initialCapacityOrOptions)`.
    ///
    /// The second argument is upstream's number-or-object union. Every option
    /// field is read as `x || default` upstream, so a `0` falls back to the
    /// default rather than being honoured — reproduced by treating a falsy
    /// value as absent.
    #[napi(constructor)]
    pub fn new(
        array_class: Option<Unknown>,
        initial_capacity_or_options: Option<Either<f64, Object>>,
    ) -> Result<Self> {
        // `if (arguments.length < 1) throw`. `new HashedArrayTree(undefined)`
        // passes upstream's check and leaves `ArrayClass` undefined; here it is
        // indistinguishable from an omitted argument and is refused. Recorded
        // as a divergence -- upstream's version only fails later, and only if
        // it ever allocates.
        let class = width_of(
            array_class.ok_or_else(|| Error::new(Status::InvalidArg, MISSING_ARRAY_CLASS))?,
        )?;

        let options = match initial_capacity_or_options {
            None => Options::default(),
            // `var initialCapacity = initialCapacityOrOptions || 0`.
            Some(Either::A(capacity)) => Options::from_capacity(count(capacity)),
            Some(Either::B(options)) => Options {
                initial_capacity: field(&options, "initialCapacity")?,
                initial_length: field(&options, "initialLength")?,
                // `options.blockSize || DEFAULT_BLOCK_SIZE`, so a 0 here never
                // reaches the power-of-two guard.
                block_size: match field(&options, "blockSize")? {
                    0 => mnemonist_core::structures::hashed_array_tree::DEFAULT_BLOCK_SIZE,
                    size => size,
                },
            },
        };

        CoreTree::new(class, options)
            .map(|inner| Self { inner })
            .map_err(raise)
    }

    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.length() as u32
    }

    #[napi(getter)]
    pub fn capacity(&self) -> u32 {
        self.inner.capacity() as u32
    }

    #[napi(getter)]
    pub fn block_size(&self) -> u32 {
        self.inner.block_size() as u32
    }

    #[napi(getter)]
    pub fn offset_mask(&self) -> u32 {
        self.inner.offset_mask() as u32
    }

    #[napi(getter)]
    pub fn block_mask(&self) -> u32 {
        self.inner.block_mask()
    }

    /// Upstream returns `this` for chaining.
    #[napi]
    pub fn set<'a>(&mut self, this: This<'a>, index: f64, value: f64) -> Result<This<'a>> {
        self.inner
            .set(count(index), to_uint32(value))
            .map_err(raise)?;

        Ok(this)
    }

    /// `undefined` past `length`; see the core docs for why `index == length`
    /// is not past it.
    ///
    /// `Either<u32, Undefined>` rather than `Option<u32>` — D-39, which this
    /// module re-learned the hard way: napi renders `None` as `null`, and
    /// `assert.strictEqual(array.get(2), undefined)` fails against `null`.
    #[napi]
    pub fn get(&self, index: f64) -> Result<Either<u32, Undefined>> {
        self.inner.get(count(index)).map(maybe).map_err(raise)
    }

    /// Upstream's `typeof capacity !== 'number'` branch is `None` here.
    #[napi]
    pub fn grow<'a>(&mut self, this: This<'a>, capacity: Option<f64>) -> This<'a> {
        self.inner.grow(capacity.map(count));

        this
    }

    #[napi]
    pub fn resize<'a>(&mut self, this: This<'a>, length: f64) -> This<'a> {
        self.inner.resize(count(length));

        this
    }

    /// Returns the new length, as upstream does.
    #[napi]
    pub fn push(&mut self, value: f64) -> u32 {
        self.inner.push(to_uint32(value)) as u32
    }

    /// `undefined` on an empty tree; otherwise the last *block*'s byte at the
    /// popped index's offset, which is upstream's defect. See the core docs.
    #[napi]
    pub fn pop(&mut self) -> Either<u32, Undefined> {
        maybe(self.inner.pop())
    }
}

/// A missing value as JS `undefined`, never as `null`. See D-39.
fn maybe(value: Option<u32>) -> Either<u32, Undefined> {
    match value {
        Some(value) => Either::A(value),
        None => Either::B(()),
    }
}

/// Identify a typed-array constructor by its `name`.
fn width_of(array_class: Unknown) -> Result<PointerWidth> {
    let constructor = array_class
        .coerce_to_object()
        .map_err(|_| Error::new(Status::InvalidArg, MISSING_ARRAY_CLASS))?;
    let name: Option<String> = constructor.get("name")?;

    match name.as_deref() {
        Some("Uint8Array") => Ok(PointerWidth::U8),
        Some("Uint16Array") => Ok(PointerWidth::U16),
        Some("Uint32Array") => Ok(PointerWidth::U32),
        other => Err(Error::new(
            Status::InvalidArg,
            format!(
                "mnemonist-rs/hashed-array-tree: unsupported array class `{}`. \
                 The port covers Uint8Array, Uint16Array and Uint32Array.",
                other.unwrap_or("<anonymous>")
            ),
        )),
    }
}

/// One option field, defaulting to `0` exactly as upstream's `|| 0` does.
fn field(options: &Object, name: &str) -> Result<usize> {
    Ok(options.get::<f64>(name)?.map_or(0, count))
}

/// A JS number used as a length/capacity/index.
///
/// Upstream lets a non-integer through — `resize(3.5)` really does leave
/// `length === 3.5` on Node — which a `usize` cannot hold. Truncating toward
/// zero is the closest honest reading and is exact for every integral input;
/// recorded as a divergence.
fn count(value: f64) -> usize {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }

    // Saturates rather than wrapping. Any value this clamps is orders of
    // magnitude past an allocatable size, so the allocation fails either way.
    value.trunc() as usize
}

/// ToUint32, the coercion a JS typed-array element store performs.
///
/// `as u32` on an `f64` *saturates* in Rust, so `-1.0` would become `0` and
/// `300.0` into a `Uint8Array` would become `255`. JavaScript wraps modulo
/// 2^32 and then the store truncates again, which is what the core's
/// `PointerVec::set` does with the `u32` this produces.
fn to_uint32(value: f64) -> u32 {
    if !value.is_finite() {
        return 0;
    }

    let wrapped = value.trunc().rem_euclid(4_294_967_296.0);

    wrapped as u32
}

/// Surface a core error with upstream's own message.
fn raise(error: CoreError) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

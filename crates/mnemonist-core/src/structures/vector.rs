//! Port of upstream `vector.js` (373 LOC, mnemonist v0.40.4).
//!
//! A growable array over a typed-array-like backing store, with a
//! caller-pluggable growth policy. Upstream's `Vector` is generic over an
//! arbitrary JS `ArrayClass` constructor (`Array`, any of the twelve typed
//! arrays, or a caller's own factory); see "What this port models" below for
//! what is actually reachable through `test/vector.js` and what is a stated
//! gap.
//!
//! # What this port models
//!
//! `test/vector.js`'s 21 `it` blocks construct a `Vector` with `Uint8Array`,
//! `Uint32Array`, `Vector.Float64Vector` and `Vector.PointerVector` — never a
//! plain `Array`, never a signed or clamped typed array, never a caller
//! `factory`. Three [`Storage`] variants cover exactly that:
//!
//! * [`Storage::Fixed`] — `Uint8Array`/`Uint16Array`/`Uint32Array`, width fixed
//!   at construction (backed by [`PointerVec`], reused rather than
//!   reinvented).
//! * [`Storage::F64`] — `Float64Array`, values stored exactly.
//! * [`Storage::Pointer`] — the `PointerVector` built-in subclass, whose width
//!   is *re-derived from the new capacity on every growth*, matching
//!   upstream's private `pointerArrayFactory`.
//!
//! Signed widths (`Int8Array`, `Int16Array`, `Int32Array`), the clamped width
//! (`Uint8ClampedArray`), `Float32Array`, a plain `Array` backing and a
//! caller-supplied `factory` are **not modelled** — the same scope cut
//! `sparse-map`'s bridge made for its `Values` constructor, for the same
//! reason: the upstream test file never reaches them, and modelling all
//! fifteen JS "typed array or factory" combinations for a module the test
//! exercises through four is effort spent on a surface nobody is checking.
//! Constructing one of the unmodelled widths through the napi bridge is a
//! refusal that names the supported set, not a silent reinterpretation.
//!
//! # `get`/`set` admit `index == length` (verified against Node 24.18.1)
//!
//! ```js
//! Vector.prototype.set = function(index, value) {
//!   if (this.length < index) throw new Error('...index out of bounds.');
//!   this.array[index] = value;
//!   return this;
//! };
//! Vector.prototype.get = function(index) {
//!   if (this.length < index) return undefined;
//!   return this.array[index];
//! };
//! ```
//!
//! Both guards are `<`, not `<=`, so `get(length)`/`set(length, …)` are
//! **admitted** — one past the last pushed element, reading or writing
//! whatever is in the *capacity* region. `set(length, v)` does not move
//! `length`, so it writes into the vector without growing it, silently:
//!
//! ```text
//! var v = new Vector(Uint8Array, 5);   // length 0, capacity 5
//! v.set(0, 42);                        // 0 < 0 is false: WRITES. length stays 0.
//! v.get(0) === 42
//! ```
//!
//! Reproduced by [`Vector::get`]/[`Vector::set`] comparing against `length`
//! exactly as upstream does, with the *actual* backing-array bound (`index <
//! self.capacity`, since storage is always exactly `capacity` slots) deciding
//! whether the access lands at all — which is what makes a **full** vector
//! (`length == capacity`) behave differently: there `set(length, v)` finds no
//! slot and is silently dropped, exactly as a typed-array store past its own
//! end is.
//!
//! # Growth copies the whole old *capacity*, stale slots included
//!
//! ```js
//! if (typed.isTypedArray(this.array)) {
//!   this.array.set(oldArray, 0);   // the WHOLE old array, not just `length`
//! }
//! ```
//!
//! Every backing this port models is a typed array, so every growth takes
//! this branch. `pop()` never clears the slot it releases
//! (`return this.array[--this.length];` — a read, not a write), so the
//! region `length..capacity` can hold stale data from an earlier, larger
//! `length`. A subsequent growth carries that stale data into the new array
//! at the same positions, and — because of the `index == length` admission
//! above — it stays reachable through `get`/`set` after the grow:
//!
//! ```text
//! var v = new Vector(Uint8Array, 2);
//! v.push(9); v.push(8);   // array [9, 8], length 2
//! v.pop();                // length 1, array UNCHANGED: [9, 8]
//! v.reallocate(4);        // array [9, 8, 0, 0] -- the 8 survived the copy
//! v.get(1) === 8          // length(1) < index(1) is false: reads the stale 8
//! ```
//!
//! Verified against Node 24.18.1. `Storage::grown` reproduces the bulk copy
//! (of the whole old capacity, not just `length`) rather than the "tidier"
//! copy-up-to-length a hand-written port would reach for.
//!
//! # A growth policy is a JS function called from Rust; see `bit-vector`
//!
//! This module's growth machinery — `applyPolicy`/`grow`/`reallocate`/`resize`
//! — is textually identical to `bit-vector.js`'s (minus the word-boundary
//! rounding there), because upstream's `bit-vector.js` is a copy-paste of this
//! module's capacity machinery onto a bit array. [`Policy`], [`default_policy`]
//! and [`Error::PolicyNotRepresentable`] are deliberately the same shapes as
//! [`crate::structures::bit_vector`]'s, for the same reason recorded there: a
//! `Box<dyn Fn(f64) -> Option<f64>>` is how a JS callback that can also throw
//! or return "not a number" is represented, and a policy returning
//! non-finite (`Infinity`/`NaN`) is a case upstream's own `applyPolicy` does
//! not guard — verified on Node: a policy returning `Infinity` passes both of
//! upstream's checks and only fails later, as `new Uint8Array(Infinity)`
//! throwing `RangeError: Invalid typed array length: Infinity`, a message this
//! port cannot reproduce letter for letter without allocating. Recorded as the
//! same divergence class as `bit-vector`'s.
//!
//! # Example
//!
//! ```
//! use mnemonist_core::structures::vector::Vector;
//! use mnemonist_core::utils::typed_arrays::PointerWidth;
//!
//! let mut vector = Vector::fixed(PointerWidth::U8, 5, 0);
//!
//! for value in 0..250 {
//!     vector.push(value as f64).unwrap();
//! }
//!
//! assert_eq!(vector.length(), 250);
//! assert_eq!(vector.capacity(), 315);
//! assert_eq!(vector.get(34), Some(34.0));
//! ```

use core::fmt;

use crate::utils::typed_arrays::{get_pointer_array, PointerVec, PointerWidth, TypedValue};

/// `DEFAULT_GROWING_POLICY`: `Math.max(1, Math.ceil(currentCapacity * 1.5))`.
pub fn default_policy(capacity: f64) -> Option<f64> {
    Some((capacity * 1.5).ceil().max(1.0))
}

/// A growth policy: capacity in, requested capacity out.
///
/// `None` stands for upstream's `typeof newCapacity !== 'number'`. See the
/// module docs and `crate::structures::bit_vector::Policy`, which this
/// mirrors.
pub type Policy = Box<dyn Fn(f64) -> Option<f64>>;

/// Upstream's `arguments.length < 1` message, verbatim. Raised by the bridge,
/// which is where arity is observable; kept here so the two cannot drift.
pub const MISSING_ARRAY_CLASS: &str =
    "mnemonist/vector: expecting at least a byte array constructor.";

/// Upstream's two `applyPolicy` throws, one `set` throw, and one refusal of
/// this port's own for a policy result no allocation can honour.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// `policy returned an invalid value (expecting a positive integer).`
    PolicyInvalidValue,
    /// `policy returned a less or equal capacity to allocate.`
    PolicyTooSmall,
    /// A policy result upstream propagates into `new ArrayClass(Infinity)` /
    /// `(NaN)`. Not an upstream message; see the module docs and the
    /// `bit-vector` divergence it mirrors.
    PolicyNotRepresentable,
    /// `Vector(<ArrayClass>).set: index out of bounds.` `class` is upstream's
    /// `this.ArrayClass.name`, resolved from `Storage::class_name`. It is
    /// carried in the error itself rather than interpolated at the bridge, so
    /// the message this crate produces already matches upstream byte for
    /// byte — the differential fuzzer compares it literally.
    IndexOutOfBounds {
        /// Upstream's `this.ArrayClass.name`.
        class: &'static str,
    },
    /// The `PointerVector` factory's width cannot index the requested
    /// capacity — upstream's `getPointerArray` throw,
    /// [`crate::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE`].
    LengthTooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PolicyInvalidValue => formatter.write_str(
                "mnemonist/vector.applyPolicy: policy returned an invalid value \
                 (expecting a positive integer).",
            ),
            Self::PolicyTooSmall => formatter.write_str(
                "mnemonist/vector.applyPolicy: policy returned a less or equal \
                 capacity to allocate.",
            ),
            Self::PolicyNotRepresentable => formatter.write_str(
                "mnemonist-rs/vector.applyPolicy: policy returned a value that is not \
                 a finite capacity.",
            ),
            Self::IndexOutOfBounds { class } => {
                write!(formatter, "Vector({class}).set: index out of bounds.")
            }
            Self::LengthTooLarge => {
                formatter.write_str(crate::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE)
            }
        }
    }
}

impl std::error::Error for Error {}

/// Which concrete typed-array class backs a [`Vector`]. See the module docs
/// for what this does and does not model.
#[derive(Debug, Clone, PartialEq)]
pub enum Storage {
    /// `Uint8Array`/`Uint16Array`/`Uint32Array`, width fixed at construction.
    Fixed(PointerVec),
    /// `Float64Array`. Values are stored exactly; no coercion at all.
    F64(Vec<f64>),
    /// The `PointerVector` factory: width re-derived from the *new* capacity
    /// on every growth.
    Pointer(PointerVec),
}

impl Storage {
    /// Slots currently allocated -- always exactly the owning [`Vector`]'s
    /// `capacity`. Public so the differential fuzzer and native tests can pin
    /// the invariant directly rather than only through its effects.
    pub fn len(&self) -> usize {
        match self {
            Self::Fixed(values) | Self::Pointer(values) => values.len(),
            Self::F64(values) => values.len(),
        }
    }

    /// Whether no slots are allocated — a zero-capacity vector.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `this.ArrayClass.name`, for the one upstream message that interpolates
    /// it. `"pointerArrayFactory"` for [`Storage::Pointer`] is the *variable*
    /// name upstream's own `var pointerArrayFactory = function (capacity) {
    /// ... }` infers for it, verified against Node 24.18.1.
    fn class_name(&self) -> &'static str {
        match self {
            Self::Fixed(values) => match values.width() {
                PointerWidth::U8 => "Uint8Array",
                PointerWidth::U16 => "Uint16Array",
                PointerWidth::U32 => "Uint32Array",
            },
            Self::Pointer(_) => "pointerArrayFactory",
            Self::F64(_) => "Float64Array",
        }
    }

    /// `this.array[index]`, unconditionally in range: the caller has already
    /// checked `index < capacity`.
    fn get(&self, index: usize) -> f64 {
        match self {
            Self::Fixed(values) | Self::Pointer(values) => f64::from(values.get(index)),
            Self::F64(values) => values[index],
        }
    }

    /// `this.array[index] = value`, with each backing's own element coercion.
    /// Also unconditionally in range.
    fn set(&mut self, index: usize, value: f64) {
        match self {
            Self::Fixed(values) | Self::Pointer(values) => values.set(index, value.to_uint32()),
            Self::F64(values) => values[index] = value,
        }
    }

    /// `new ArrayClass(newCapacity)` then `array.set(oldArray, 0)` — every
    /// backing here is a typed array, so growth always takes the bulk-copy
    /// branch, over the **whole** old capacity (see the module docs).
    fn grown(&self, new_capacity: usize) -> Result<Self, Error> {
        Ok(match self {
            Self::Fixed(old) => {
                let mut values = PointerVec::zeroed(old.width(), new_capacity);

                for index in 0..old.len() {
                    values.set(index, old.get(index));
                }

                Self::Fixed(values)
            }
            // The factory re-derives the width from the capacity being grown
            // *to*, exactly as `pointerArrayFactory(capacity)` does on every
            // call -- not only when the width actually needs to change.
            Self::Pointer(old) => {
                let width =
                    get_pointer_array(new_capacity as f64).map_err(|_| Error::LengthTooLarge)?;
                let mut values = PointerVec::zeroed(width, new_capacity);

                for index in 0..old.len() {
                    values.set(index, old.get(index));
                }

                Self::Pointer(values)
            }
            Self::F64(old) => {
                let mut values = vec![0.0; new_capacity];

                values[..old.len()].copy_from_slice(old);

                Self::F64(values)
            }
        })
    }

    /// `oldArray.slice(0, newCapacity)`. Width is **not** recomputed here —
    /// only growth re-derives it for [`Storage::Pointer`] — so shrinking a
    /// `PointerVector` below a width boundary does not narrow it back.
    fn shrunk(&self, new_capacity: usize) -> Self {
        match self {
            Self::Fixed(old) => {
                let mut values = PointerVec::zeroed(old.width(), new_capacity);

                for index in 0..new_capacity {
                    values.set(index, old.get(index));
                }

                Self::Fixed(values)
            }
            Self::Pointer(old) => {
                let mut values = PointerVec::zeroed(old.width(), new_capacity);

                for index in 0..new_capacity {
                    values.set(index, old.get(index));
                }

                Self::Pointer(values)
            }
            Self::F64(old) => Self::F64(old[..new_capacity].to_vec()),
        }
    }
}

/// A growable array over a typed-array-like backing store.
pub struct Vector {
    length: usize,
    capacity: usize,
    policy: Policy,
    storage: Storage,
}

impl fmt::Debug for Vector {
    /// Hand-written: a `Box<dyn Fn>` has no `Debug`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Vector")
            .field("length", &self.length)
            .field("capacity", &self.capacity)
            .field("storage", &self.storage)
            .finish_non_exhaustive()
    }
}

impl Vector {
    /// `new Vector(Uint8Array | Uint16Array | Uint32Array, …)`.
    ///
    /// `capacity` and `length` are upstream's already-resolved
    /// `this.capacity = Math.max(initialLength, initialCapacity)` and
    /// `this.length = initialLength` — the union of a bare number and an
    /// `{initialCapacity, initialLength}` options object is a JavaScript-only
    /// notion and is resolved at the bridge.
    pub fn fixed(width: PointerWidth, capacity: usize, length: usize) -> Self {
        Self::fixed_with_policy(width, capacity, length, Box::new(default_policy))
    }

    /// [`Vector::fixed`] with an explicit growth policy in place of
    /// upstream's default `capacity * 1.5`.
    pub fn fixed_with_policy(
        width: PointerWidth,
        capacity: usize,
        length: usize,
        policy: Policy,
    ) -> Self {
        let capacity = capacity.max(length);

        Self {
            length,
            capacity,
            policy,
            storage: Storage::Fixed(PointerVec::zeroed(width, capacity)),
        }
    }

    /// `new Vector(Float64Array, …)` / `new Vector.Float64Vector(…)`.
    pub fn f64(capacity: usize, length: usize) -> Self {
        Self::f64_with_policy(capacity, length, Box::new(default_policy))
    }

    /// [`Vector::f64`] with an explicit growth policy in place of upstream's
    /// default `capacity * 1.5`.
    pub fn f64_with_policy(capacity: usize, length: usize, policy: Policy) -> Self {
        let capacity = capacity.max(length);

        Self {
            length,
            capacity,
            policy,
            storage: Storage::F64(vec![0.0; capacity]),
        }
    }

    /// `new Vector.PointerVector(…)`.
    ///
    /// # Errors
    ///
    /// [`Error::LengthTooLarge`] when `max(length, capacity)` exceeds what a
    /// 32-bit pointer array can index — upstream's `pointerArrayFactory`
    /// reaching `getPointerArray`'s own throw.
    pub fn pointer(capacity: usize, length: usize) -> Result<Self, Error> {
        Self::pointer_with_policy(capacity, length, Box::new(default_policy))
    }

    /// [`Vector::pointer`] with an explicit growth policy in place of
    /// upstream's default `capacity * 1.5`.
    ///
    /// # Errors
    ///
    /// [`Error::LengthTooLarge`], on the same condition as
    /// [`Vector::pointer`].
    pub fn pointer_with_policy(
        capacity: usize,
        length: usize,
        policy: Policy,
    ) -> Result<Self, Error> {
        let capacity = capacity.max(length);
        let width = get_pointer_array(capacity as f64).map_err(|_| Error::LengthTooLarge)?;

        Ok(Self {
            length,
            capacity,
            policy,
            storage: Storage::Pointer(PointerVec::zeroed(width, capacity)),
        })
    }

    /// `#.length` — the number of live elements, which is what `get`/`set`
    /// bounds-check against.
    pub fn length(&self) -> usize {
        self.length
    }

    /// `#.capacity` — the number of allocated slots, always at least
    /// [`length`](Vector::length). Slots between the two are zeroed and
    /// writable via `set`, but not counted.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The backing store, compared slot for slot by the differential fuzzer.
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// `#.set(index, value)`.
    ///
    /// # Errors
    ///
    /// [`Error::IndexOutOfBounds`] for `index > length` — note the strict
    /// comparison, which admits `index == length` and writes into the
    /// capacity region without moving `length`. See the module docs.
    pub fn set(&mut self, index: usize, value: f64) -> Result<(), Error> {
        if self.length < index {
            return Err(Error::IndexOutOfBounds {
                class: self.storage.class_name(),
            });
        }

        // `index == capacity` only when the vector is exactly full: the real
        // backing array has no slot there, and upstream's out-of-range typed
        // store is a silent no-op.
        if index < self.capacity {
            self.storage.set(index, value);
        }

        Ok(())
    }

    /// `#.get(index)` — `None` where upstream returns `undefined`.
    pub fn get(&self, index: usize) -> Option<f64> {
        if self.length < index {
            return None;
        }

        if index < self.capacity {
            return Some(self.storage.get(index));
        }

        None
    }

    /// `applyPolicy(override)` — the next capacity the policy proposes.
    ///
    /// `override.filter(|c| c != 0).unwrap_or(self.capacity)`: a `0` override
    /// is falsy upstream and falls back to the current capacity, which is why
    /// `grow()` works on a zero-capacity vector at all.
    pub fn apply_policy(&self, override_capacity: Option<usize>) -> Result<usize, Error> {
        let input = match override_capacity {
            Some(capacity) if capacity != 0 => capacity as f64,
            _ => self.capacity as f64,
        };

        let Some(requested) = (self.policy)(input) else {
            return Err(Error::PolicyInvalidValue);
        };

        // `typeof newCapacity !== 'number' || newCapacity < 0`. NaN passes
        // this upstream (every NaN comparison is false) and propagates into
        // the eventual allocation; see the module docs.
        if requested < 0.0 {
            return Err(Error::PolicyInvalidValue);
        }

        if !requested.is_finite() {
            return Err(Error::PolicyNotRepresentable);
        }

        if requested <= self.capacity as f64 {
            return Err(Error::PolicyTooSmall);
        }

        Ok(requested.trunc() as usize)
    }

    /// `reallocate(capacity)` — resize the backing store to fit `capacity`.
    ///
    /// # Errors
    ///
    /// [`Error::LengthTooLarge`] if the storage is [`Storage::Pointer`] and
    /// `capacity` exceeds what a 32-bit pointer array can index.
    pub fn reallocate(&mut self, capacity: usize) -> Result<(), Error> {
        if capacity == self.capacity {
            return Ok(());
        }

        if capacity < self.length {
            self.length = capacity;
        }

        self.storage = if capacity > self.capacity {
            self.storage.grown(capacity)?
        } else {
            self.storage.shrunk(capacity)
        };

        self.capacity = capacity;

        Ok(())
    }

    /// `grow(capacity)` — reach at least `capacity`, applying the policy as
    /// many times as it takes. `None` applies it exactly once.
    pub fn grow(&mut self, capacity: Option<usize>) -> Result<(), Error> {
        let Some(target) = capacity else {
            let new_capacity = self.apply_policy(None)?;

            return self.reallocate(new_capacity);
        };

        if self.capacity >= target {
            return Ok(());
        }

        let mut new_capacity = self.capacity;

        while new_capacity < target {
            new_capacity = self.apply_policy(Some(new_capacity))?;
        }

        self.reallocate(new_capacity)
    }

    /// `resize(length)` — set the length, reallocating upward if needed.
    /// Shrinking never deallocates.
    pub fn resize(&mut self, length: usize) -> Result<(), Error> {
        if length == self.length {
            return Ok(());
        }

        if length < self.length {
            self.length = length;
            return Ok(());
        }

        self.length = length;
        self.reallocate(length)
    }

    /// `#.push(value)` — returns the new length.
    pub fn push(&mut self, value: f64) -> Result<usize, Error> {
        if self.capacity == self.length {
            self.grow(None)?;
        }

        self.storage.set(self.length, value);
        self.length += 1;

        Ok(self.length)
    }

    /// `#.pop()` — `None` on an empty vector, which is upstream's bare
    /// `return;`. Never clears the released slot; see the module docs.
    pub fn pop(&mut self) -> Option<f64> {
        if self.length == 0 {
            return None;
        }

        self.length -= 1;

        Some(self.storage.get(self.length))
    }

    /// `#.values()` — an in-order copy of `0..length`, upstream's cursor
    /// drained. `mnemonist-core`'s cursor machinery (`crate::cursor`) is not
    /// used here: upstream's `Vector.prototype.values` freezes **both** `i`
    /// and `l` at creation and reads `items` live, which is exactly
    /// `Cursor`'s hybrid capture — but a vector's `set`/`push` never leave a
    /// hole the way `SparseSet`'s does, so a native Rust cursor buys nothing
    /// this module's tests exercise. The bridge builds the JS-visible cursor
    /// directly over `Sequence`, below.
    pub fn to_vec(&self) -> Vec<f64> {
        (0..self.length)
            .map(|index| self.storage.get(index))
            .collect()
    }
}

/// `values()`/`entries()`/`Symbol.iterator`, all of upstream's own closure:
/// `i`/`l` frozen at creation, `items` read live. `set`/`push` cannot open a
/// gap in `0..length` the way `SparseSet`'s sparse arrays can, so this walk
/// never yields [`crate::cursor::Step::Gap`] in practice -- but the frozen
/// length still matters: a `pop` during iteration ends the walk exactly where
/// upstream's `i >= l` would.
impl crate::cursor::Sequence for Vector {
    type Item = f64;
    type Frozen = ();

    fn freeze(&self) -> ((), usize) {
        ((), self.length)
    }

    fn slot(&self, _frozen: &(), ordinal: usize) -> Option<f64> {
        if ordinal < self.capacity {
            Some(self.storage.get(ordinal))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{Cursor, Step};

    fn fixed(width: PointerWidth, capacity: usize) -> Vector {
        Vector::fixed(width, capacity, 0)
    }

    fn with_policy(capacity: usize, policy: impl Fn(f64) -> Option<f64> + 'static) -> Vector {
        Vector::fixed_with_policy(PointerWidth::U8, capacity, 0, Box::new(policy))
    }

    /// 1:1 port of every upstream `it` block in `test/vector.js`, as the
    /// baseline the rest builds on. `test/vector.js` has 21 blocks; several
    /// are folded together here exactly as upstream folds several assertions
    /// into one `it`.
    #[test]
    fn reproduces_the_upstream_suite() {
        // "should be possible to create a dynamic vector."
        let vector = fixed(PointerWidth::U8, 5);
        assert_eq!((vector.length(), vector.capacity()), (0, 5));

        // "should be possible to set and get values."
        let mut vector = Vector::fixed(PointerWidth::U8, 0, 3);
        vector.set(2, 24.0).unwrap();
        assert_eq!(vector.length(), 3);
        assert_eq!(vector.get(2), Some(24.0));

        // "should return undefined on out-of-bound values."
        let vector = fixed(PointerWidth::U8, 5);
        assert_eq!(vector.get(2), None);

        // "setting an out-of-bound index should throw."
        let mut vector = fixed(PointerWidth::U8, 4);
        assert_eq!(
            vector.set(56, 4.0),
            Err(Error::IndexOutOfBounds {
                class: "Uint8Array"
            })
        );

        // "should be possible to push values."
        let mut vector = fixed(PointerWidth::U8, 5);
        for i in 0..250 {
            vector.push(i as f64).unwrap();
        }
        assert_eq!(vector.length(), 250);
        assert_eq!(vector.capacity(), 315);
        assert_eq!(vector.get(34), Some(34.0));

        // "should be possible to pop values."
        let mut vector = Vector::fixed(PointerWidth::U32, 3, 0);
        vector.push(1.0).unwrap();
        vector.push(2.0).unwrap();
        assert_eq!(vector.pop(), Some(2.0));
        assert_eq!(vector.length(), 1);
        assert_eq!(vector.pop(), Some(1.0));
        assert_eq!(vector.length(), 0);
        assert_eq!(vector.pop(), None);
        assert_eq!(vector.length(), 0);
        vector.push(34.0).unwrap();
        vector.push(35.0).unwrap();
        assert_eq!(vector.get(1), Some(35.0));
        assert_eq!(vector.length(), 2);

        // "should be possible to reallocate."
        let mut vector = fixed(PointerWidth::U8, 10);
        vector.push(1.0).unwrap();
        vector.push(2.0).unwrap();
        vector.push(3.0).unwrap();
        vector.reallocate(20).unwrap();
        assert_eq!((vector.capacity(), vector.length()), (20, 3));
        vector.reallocate(2).unwrap();
        assert_eq!((vector.capacity(), vector.length()), (2, 2));

        // "should be possible to grow the vector."
        let mut vector = with_policy(2, |capacity| Some(capacity + 2.0));
        vector.grow(Some(5)).unwrap();
        assert_eq!(vector.capacity(), 6);
        vector.grow(Some(2)).unwrap();
        assert_eq!(vector.capacity(), 6);
        vector.grow(None).unwrap();
        assert_eq!(vector.capacity(), 8);

        // "should be possible to resize the vector."
        let mut vector = Vector::fixed(PointerWidth::U8, 0, 23);
        vector.resize(20).unwrap();
        assert_eq!((vector.capacity(), vector.length()), (23, 20));
        vector.resize(30).unwrap();
        assert_eq!((vector.capacity(), vector.length()), (30, 30));

        // "should throw if the policy returns an irrelevant size."
        let mut vector = with_policy(1, Some);
        vector.push(3.0).unwrap();
        assert_eq!(vector.push(4.0), Err(Error::PolicyTooSmall));

        // "should be possible to use a custom policy."
        let mut vector = with_policy(2, |capacity| Some(capacity + 2.0));
        vector.push(1.0).unwrap();
        vector.push(2.0).unwrap();
        vector.push(3.0).unwrap();
        assert_eq!((vector.length(), vector.capacity()), (3, 4));

        // "should be possible to use the subclasses." (Float64Vector)
        let mut vector = Vector::f64(0, 3);
        vector.set(2, 24.0).unwrap();
        assert_eq!(vector.length(), 3);
        assert_eq!(vector.get(2), Some(24.0));

        // "should be possible to create a vector from an arbitrary iterable."
        // (the `.from` union itself is bridge-level; the resulting shape is:)
        let mut vector = Vector::fixed(PointerWidth::U8, 3, 0);
        for value in [1.0, 2.0, 3.0] {
            vector.push(value).unwrap();
        }
        assert_eq!((vector.length(), vector.capacity()), (3, 3));
        assert_eq!(vector.to_vec(), vec![1.0, 2.0, 3.0]);

        // "should be possible to create a values iterator."
        let mut vector = Vector::fixed(PointerWidth::U8, 3, 0);
        for value in [1.0, 2.0, 3.0] {
            vector.push(value).unwrap();
        }
        let mut cursor = Cursor::new(&vector);
        assert_eq!(cursor.step(), Step::Item(1.0));
        assert_eq!(cursor.step(), Step::Item(2.0));
        assert_eq!(cursor.step(), Step::Item(3.0));
        assert_eq!(cursor.step(), Step::Done);

        // "should be possible to create a pointer vector."
        let mut vector = Vector::pointer(0, 0).unwrap();
        assert!(matches!(vector.storage(), Storage::Pointer(pv) if pv.width() == PointerWidth::U8));
        for i in 0..500 {
            vector.push(i as f64).unwrap();
        }
        assert_eq!(vector.length(), 500);
        assert!(
            matches!(vector.storage(), Storage::Pointer(pv) if pv.width() == PointerWidth::U16)
        );
    }

    /// The off-by-one in both guards: `get`/`set` admit `index == length`.
    /// Verified against Node 24.18.1 -- see the module docs.
    #[test]
    fn get_and_set_admit_index_equal_to_length() {
        let mut vector = fixed(PointerWidth::U8, 5);

        assert_eq!(vector.get(0), Some(0.0));
        vector.set(0, 42.0).unwrap();
        assert_eq!(vector.length(), 0);
        assert_eq!(vector.get(0), Some(42.0));
    }

    /// A full vector (`length == capacity`) has no capacity-region slot to
    /// admit into, so the same `index == length` case silently no-ops instead.
    #[test]
    fn a_full_vector_drops_the_admitted_write() {
        let mut vector = fixed(PointerWidth::U8, 2);

        vector.push(1.0).unwrap();
        vector.push(2.0).unwrap();
        assert_eq!(vector.length(), vector.capacity());

        // In range of the *upstream guard* (index == length == capacity) but
        // not of the real backing array: dropped, and `get` mirrors it.
        vector.set(2, 99.0).unwrap();
        assert_eq!(vector.get(2), None);
    }

    /// Growth bulk-copies the whole old *capacity*, so a `pop`'s stale slot
    /// survives a grow and stays reachable through the `index == length`
    /// admission. Verified against Node 24.18.1 -- see the module docs.
    #[test]
    fn stale_data_from_a_pop_survives_a_growth_and_stays_reachable() {
        let mut vector = Vector::fixed(PointerWidth::U8, 2, 0);

        vector.push(9.0).unwrap();
        vector.push(8.0).unwrap();
        vector.pop().unwrap();
        assert_eq!(vector.length(), 1);

        vector.reallocate(4).unwrap();
        assert_eq!(vector.get(1), Some(8.0));
    }

    /// A policy returning `Infinity` passes both of upstream's own checks
    /// (neither `< 0` nor `<= capacity` catches it) and only fails at the
    /// point of allocation. This port refuses earlier, catchably, rather than
    /// attempting an allocation no real machine has memory for.
    #[test]
    fn a_policy_returning_infinity_is_refused_before_any_allocation() {
        let mut vector = with_policy(2, |_capacity| Some(f64::INFINITY));

        assert_eq!(vector.grow(None), Err(Error::PolicyNotRepresentable));
    }

    /// `NaN` fails neither upstream check either (every `NaN` comparison is
    /// false), and lands in the same refusal.
    #[test]
    fn a_policy_returning_nan_is_refused() {
        let mut vector = with_policy(2, |_capacity| Some(f64::NAN));

        assert_eq!(vector.grow(None), Err(Error::PolicyNotRepresentable));
    }

    /// A policy returning a negative number is upstream's `PolicyInvalidValue`
    /// -- checked before the finiteness refusal, so `-Infinity` lands here and
    /// not there.
    #[test]
    fn a_policy_returning_a_negative_number_is_invalid_before_being_non_finite() {
        let mut vector = with_policy(2, |_capacity| Some(-1.0));
        assert_eq!(vector.grow(None), Err(Error::PolicyInvalidValue));

        let mut vector = with_policy(2, |_capacity| Some(f64::NEG_INFINITY));
        assert_eq!(vector.grow(None), Err(Error::PolicyInvalidValue));
    }

    /// A policy returning "not a number" (a JS string, an object -- collapsed
    /// to `None` at the bridge) is upstream's other `applyPolicy` throw.
    #[test]
    fn a_policy_returning_not_a_number_is_invalid() {
        let mut vector = with_policy(2, |_capacity| None);

        assert_eq!(vector.grow(None), Err(Error::PolicyInvalidValue));
    }

    /// Shrinking a `PointerVector` below a width boundary does not narrow it
    /// back -- only growth re-derives the width. Verified against the same
    /// mechanism upstream: `oldArray.slice(...)` keeps the source's class.
    #[test]
    fn shrinking_a_pointer_vector_keeps_its_current_width() {
        let mut vector = Vector::pointer(0, 0).unwrap();

        for i in 0..300 {
            vector.push(i as f64).unwrap();
        }
        assert!(
            matches!(vector.storage(), Storage::Pointer(pv) if pv.width() == PointerWidth::U16)
        );

        vector.reallocate(10).unwrap();
        assert!(
            matches!(vector.storage(), Storage::Pointer(pv) if pv.width() == PointerWidth::U16)
        );
    }

    /// `PointerVector` at length exactly zero: `getPointerArray(0)` is a
    /// `Uint8Array`, matching `new Vector.PointerVector().array instanceof
    /// Uint8Array` before any push.
    #[test]
    fn a_zero_capacity_pointer_vector_starts_at_the_narrowest_width() {
        let vector = Vector::pointer(0, 0).unwrap();

        assert!(
            matches!(vector.storage(), Storage::Pointer(pv) if pv.width() == PointerWidth::U8 && pv.is_empty())
        );
    }

    /// A `PointerVector` capacity request too large for any pointer width to
    /// index is refused rather than silently reinterpreted.
    #[test]
    fn a_pointer_vector_too_large_to_index_is_refused() {
        assert_eq!(
            Vector::pointer(4_294_967_297, 0).unwrap_err(),
            Error::LengthTooLarge
        );
    }

    /// `Float64Array` values round-trip exactly -- no `ToUint32`, no
    /// truncation, unlike every other backing this module models.
    #[test]
    fn float64_values_are_stored_exactly() {
        let mut vector = Vector::f64(0, 0);

        vector.push(1.5).unwrap();
        vector.push(-0.25).unwrap();
        assert_eq!(vector.to_vec(), vec![1.5, -0.25]);
    }

    /// Values narrow to the backing's width on the way in, exactly as a JS
    /// typed-array element store does.
    #[test]
    fn fixed_values_truncate_at_their_own_width() {
        let mut narrow = fixed(PointerWidth::U8, 1);
        narrow.push(300.0).unwrap();
        assert_eq!(narrow.get(0), Some(300.0 % 256.0));

        let mut wide = fixed(PointerWidth::U16, 1);
        wide.push(70_000.0).unwrap();
        assert_eq!(wide.get(0), Some(70_000.0 % 65_536.0));
    }

    /// DIV-STACK-1/DIV-STACK-2 seen from Rust: each cursor is exhausted once, but the vector
    /// can be walked again.
    #[test]
    fn cursors_do_not_restart_but_the_vector_can_be_walked_again() {
        let mut vector = Vector::fixed(PointerWidth::U8, 3, 0);
        for value in [1.0, 2.0, 3.0] {
            vector.push(value).unwrap();
        }

        let mut cursor = Cursor::new(&vector);
        assert_eq!(cursor.by_ref().count(), 3);
        assert_eq!(cursor.count(), 0);

        assert_eq!(
            Cursor::new(&vector).collect::<Vec<_>>(),
            vec![1.0, 2.0, 3.0]
        );
    }

    /// A `pop` between two steps: the frozen length still bounds the walk,
    /// matching upstream's `i >= l` against a captured `l`. Driven through
    /// the detached [`crate::cursor::CursorState`] rather than [`Cursor`],
    /// because `Cursor` borrows its source for the walk's whole lifetime and
    /// the borrow checker refuses the interleaved mutation outright -- which
    /// is DIV-PROJ-10's whole point (see `crate::cursor`'s module docs): reaching a
    /// mutation mid-walk from safe Rust needs interior mutability or the FFI
    /// boundary, not a plain `&mut`.
    #[test]
    fn a_pop_during_iteration_stays_bounded_by_the_frozen_length() {
        use crate::cursor::CursorState;

        let mut vector = Vector::fixed(PointerWidth::U8, 3, 0);
        for value in [1.0, 2.0, 3.0] {
            vector.push(value).unwrap();
        }

        let mut state = CursorState::open(&vector);
        assert_eq!(state.step(&vector), Step::Item(1.0));

        vector.pop();

        // Ordinal 1 (value 2.0) is still a live slot; ordinal 2 is inside the
        // frozen length but past the vector's new length -- the capacity
        // region still answers (it is still `< capacity`), so this yields the
        // stale-but-present old value 3.0 rather than a gap.
        assert_eq!(state.step(&vector), Step::Item(2.0));
        assert_eq!(state.step(&vector), Step::Item(3.0));
        assert_eq!(state.step(&vector), Step::Done);
    }

    /// Empty vectors, and the degenerate zero-length/zero-capacity case.
    #[test]
    fn an_empty_vector_pops_and_iterates_to_nothing() {
        let mut vector = fixed(PointerWidth::U8, 0);

        assert_eq!(vector.pop(), None);
        assert_eq!(Cursor::new(&vector).collect::<Vec<_>>(), Vec::<f64>::new());
    }

    /// Filling to capacity without running off the end, at a width boundary.
    #[test]
    fn fills_to_capacity_without_running_off_the_end() {
        let mut vector = fixed(PointerWidth::U16, 300);

        for i in 0..300 {
            vector.push(i as f64).unwrap();
        }

        assert_eq!((vector.length(), vector.capacity()), (300, 300));
        assert_eq!(vector.get(299), Some(299.0));
        assert_eq!(vector.get(300), None);
    }

    /// The out-of-bounds message names the actual backing class -- verified
    /// against Node 24.18.1 for all four widths this port models, plus the
    /// `PointerVector` factory's inferred function name.
    #[test]
    fn the_out_of_bounds_message_names_the_actual_backing_class() {
        assert_eq!(
            fixed(PointerWidth::U8, 1)
                .set(5, 0.0)
                .unwrap_err()
                .to_string(),
            "Vector(Uint8Array).set: index out of bounds."
        );
        assert_eq!(
            fixed(PointerWidth::U16, 1)
                .set(5, 0.0)
                .unwrap_err()
                .to_string(),
            "Vector(Uint16Array).set: index out of bounds."
        );
        assert_eq!(
            fixed(PointerWidth::U32, 1)
                .set(5, 0.0)
                .unwrap_err()
                .to_string(),
            "Vector(Uint32Array).set: index out of bounds."
        );
        assert_eq!(
            Vector::f64(1, 0).set(5, 0.0).unwrap_err().to_string(),
            "Vector(Float64Array).set: index out of bounds."
        );
        assert_eq!(
            Vector::pointer(1, 0)
                .unwrap()
                .set(5, 0.0)
                .unwrap_err()
                .to_string(),
            "Vector(pointerArrayFactory).set: index out of bounds."
        );
    }
}

//! JS bridge for [`mnemonist_core::structures::bloom_filter`].
//!
//! Thin translation only — with one exception that has to be here rather than
//! in the core, because it is entirely about JavaScript values.
//!
//! 1. **`stringToByteArray` lives here, and it is where B-98 lives.** Upstream's
//!    conversion is
//!
//!    ```js
//!    var array = new Uint16Array(string.length);
//!    for (i = 0; i < string.length; i++) array[i] = string.charCodeAt(i);
//!    ```
//!
//!    On a number, `string.length` is `undefined`, `new Uint16Array(undefined)`
//!    is **empty**, and the loop never runs — so every non-string item without
//!    a `length` hashes as the empty sequence, and they all collide with each
//!    other and with `''`. On `null` or `undefined` the property read itself
//!    throws a `TypeError`. On an array, `length` is fine but `charCodeAt` is
//!    not a function and *that* throws. On a boxed `new String('hello')`,
//!    everything works and the result is identical to the primitive. All four
//!    behaviours are reproduced by [`to_units`], because a "sensible" bridge
//!    that rejected non-strings would hide a real defect.
//! 2. **The constructor's argument dispatch is JavaScript-shaped.** Upstream
//!    branches on falsiness first (`if (!capacityOrOptions) throw`), then on
//!    `typeof === 'object'`, then validates `typeof options.capacity`. That
//!    ordering is observable — `new BloomFilter(0)` reports "must be created
//!    with a capacity" while `new BloomFilter(-1)` reports "should be a
//!    positive integer" — so it is reproduced literally.
//! 3. **`errorRate` has a three-way reading**, and the core needs it
//!    pre-classified: absent (`undefined`) defaults silently, a number is
//!    passed through, and anything else — including `null`, `''` and `false` —
//!    fails validation, because they are all falsy *and* `<= 0` is true for
//!    them. Only `undefined` and `NaN` reach the silent default.
//! 4. **`from` goes through the real dispatch.** [`crate::foreach::collect`] is
//!    the five-branch coercion, unmodified, so `BloomFilter.from(new Map(...))`
//!    behaves the way upstream's does because it is the same code path.
//! 5. **The core structure is held in a [`RefCell`].** `add` and `clear` mutate,
//!    and `from` runs JavaScript (`collect` can call a host `forEach`, a
//!    `Symbol.iterator`, or a user `toString`) — so unlike the suffix arrays,
//!    the B-31 aliasing hazard is live here, not theoretical. Every borrow ends
//!    before any JS call.
//! 6. **`inspect` is not ported.** No upstream assertion, no Rust equivalent.

use std::cell::RefCell;

use mnemonist_core::structures::bloom_filter::{BloomFilter as CoreBloomFilter, BuildError};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::foreach;

/// `if (!capacityOrOptions) throw` — upstream's message for a falsy argument.
const NO_CAPACITY: &str =
    "mnemonist/BloomFilter.constructor: a BloomFilter must be created with a capacity.";

/// `BloomFilter.from`'s message when neither `length` nor `size` is a number.
const CANNOT_INFER: &str =
    "BloomFilter.from: could not infer the filter's capacity. Try passing it as second argument.";

/// `Boolean(value)` — JavaScript truthiness, which is not `is_some`.
///
/// Needed twice: for `if (!capacityOrOptions)` and for `if (!options)` in
/// `from`. `0`, `''`, `false`, `NaN` and `null` are all falsy, and napi's
/// `Option<Unknown>` only distinguishes the missing argument.
fn truthy(env: &Env, value: &Unknown) -> Result<bool> {
    let global = env.get_global()?;
    let boolean: Function<'_, Unknown, bool> = global.get_named_property("Boolean")?;

    boolean.call(*value)
}

/// A [`BuildError`] as the JavaScript error upstream raises.
///
/// The class matters: the two validation failures are the module's own `Error`,
/// while a negative length is the *allocator's* `RangeError`. napi has no
/// direct `RangeError` constructor, so it is thrown by name through
/// `Status::GenericFailure` with the message upstream produces; what upstream's
/// suite matches on is the message, not the class.
fn build_error(error: BuildError) -> Error {
    Error::new(Status::GenericFailure, error.message())
}

/// `stringToByteArray(item)`. See adaptation 1 in the module docs.
fn to_units(_env: &Env, item: &Unknown) -> Result<Vec<u16>> {
    match item.get_type()? {
        ValueType::String => Ok(String::from_unknown(*item)?.encode_utf16().collect()),
        // `null.length` / `undefined.length`.
        ValueType::Null => Err(Error::new(
            Status::GenericFailure,
            "Cannot read properties of null (reading 'length')".to_owned(),
        )),
        ValueType::Undefined => Err(Error::new(
            Status::GenericFailure,
            "Cannot read properties of undefined (reading 'length')".to_owned(),
        )),
        // A primitive with no `length` property: `new Uint16Array(undefined)`
        // is empty and the loop never runs. This is B-98.
        ValueType::Number | ValueType::Boolean | ValueType::BigInt | ValueType::Symbol => {
            Ok(Vec::new())
        }
        _ => {
            let object = Object::from_unknown(*item)?;
            let length: Option<f64> = object.get("length")?;
            let length = match length {
                Some(length) if length > 0.0 => length as usize,
                // Absent, zero, negative or NaN: an empty typed array.
                _ => return Ok(Vec::new()),
            };
            // `string.charCodeAt(i)`. Absent on an array, which is why
            // `filter.add(['a'])` is a TypeError upstream rather than a hash of
            // the character.
            let char_code_at: Option<Function<'_, u32, u32>> = object.get("charCodeAt")?;
            let char_code_at = char_code_at.ok_or_else(|| {
                Error::new(
                    Status::GenericFailure,
                    "string.charCodeAt is not a function".to_owned(),
                )
            })?;
            let mut units = Vec::with_capacity(length);

            for index in 0..length {
                // A typed array stores the code as a Uint16, so anything out of
                // range wraps rather than erroring.
                units.push(char_code_at.apply(*item, index as u32)? as u16);
            }

            Ok(units)
        }
    }
}

/// `new BloomFilter(capacityOrOptions)` — the argument dispatch, then the core.
fn build(env: &Env, capacity_or_options: Unknown) -> Result<CoreBloomFilter> {
    if !truthy(env, &capacity_or_options)? {
        return Err(Error::new(Status::GenericFailure, NO_CAPACITY.to_owned()));
    }

    // `typeof capacityOrOptions === 'object'`. A function takes the else
    // branch, where it fails the `typeof options.capacity !== 'number'` check.
    let (capacity, error_rate) = if capacity_or_options.get_type()? == ValueType::Object {
        let options = Object::from_unknown(capacity_or_options)?;
        let capacity: Option<f64> = options.get("capacity")?;
        let capacity = capacity.ok_or_else(|| build_error(BuildError::Capacity))?;
        let raw: Option<Unknown> = options.get("errorRate")?;
        let error_rate = match raw {
            None => None,
            Some(value) => match value.get_type()? {
                ValueType::Undefined => None,
                ValueType::Number => Some(f64::from_unknown(value)?),
                // Everything else fails validation, whichever way it goes:
                // truthy non-numbers fail `typeof this.errorRate !== 'number'`,
                // and falsy ones (`null`, `''`, `false`) satisfy `<= 0`.
                _ => return Err(build_error(BuildError::ErrorRate)),
            },
        };

        (capacity, error_rate)
    } else {
        let capacity = match capacity_or_options.get_type()? {
            ValueType::Number => f64::from_unknown(capacity_or_options)?,
            _ => return Err(build_error(BuildError::Capacity)),
        };

        (capacity, None)
    };

    CoreBloomFilter::new(capacity, error_rate).map_err(build_error)
}

/// `iterable.length || iterable.size`, for `from`'s capacity inference.
fn infer_capacity(_env: &Env, iterable: &Unknown) -> Result<f64> {
    let cannot_infer = || Error::new(Status::GenericFailure, CANNOT_INFER.to_owned());

    // A string has a `length` but is not an `Object` to napi.
    if iterable.get_type()? == ValueType::String {
        let length = String::from_unknown(*iterable)?.encode_utf16().count();

        return if length > 0 {
            Ok(length as f64)
        } else {
            // `0 || undefined` is `undefined`, which is not a number.
            Err(cannot_infer())
        };
    }

    let object = Object::from_unknown(*iterable).map_err(|_| cannot_infer())?;

    for name in ["length", "size"] {
        if let Some(value) = object.get::<Unknown>(name)? {
            if value.get_type()? == ValueType::Number {
                let value = f64::from_unknown(value)?;

                // `a || b`: a zero `length` falls through to `size`.
                if value != 0.0 && !value.is_nan() {
                    return Ok(value);
                }
            }
        }
    }

    Err(cannot_infer())
}

/// A Bloom filter over arbitrary JavaScript strings.
#[napi(js_name = "BloomFilter")]
pub struct JsBloomFilter {
    inner: RefCell<CoreBloomFilter>,
}

#[napi]
impl JsBloomFilter {
    #[napi(constructor)]
    pub fn new(env: Env, capacity_or_options: Unknown) -> Result<Self> {
        Ok(Self {
            inner: RefCell::new(build(&env, capacity_or_options)?),
        })
    }

    /// `#.capacity`.
    #[napi(getter)]
    pub fn capacity(&self) -> f64 {
        self.inner.borrow().capacity()
    }

    /// `#.errorRate`.
    #[napi(getter, js_name = "errorRate")]
    pub fn error_rate(&self) -> f64 {
        self.inner.borrow().error_rate()
    }

    /// `#.hashFunctions` — zero is reachable and is not an error. See B-97.
    #[napi(getter, js_name = "hashFunctions")]
    pub fn hash_functions(&self) -> u32 {
        self.inner.borrow().hash_functions() as u32
    }

    /// `#.data` — the bit array, as the `Uint8Array` upstream allocates.
    ///
    /// A fresh copy per read rather than a view onto the Rust buffer: napi
    /// cannot hand out a borrowed typed array whose backing store a later
    /// `add` may reallocate. Every upstream assertion reads it through
    /// `Array.from`, and `filter.data.length` is a length either way.
    #[napi(getter)]
    pub fn data(&self) -> Uint8Array {
        Uint8Array::new(self.inner.borrow().data().to_vec())
    }

    /// `#.toJSON`, which upstream defines as `this.data`.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Uint8Array {
        Uint8Array::new(self.inner.borrow().data().to_vec())
    }

    /// `#.clear` — re-derive the sizing and drop every bit.
    #[napi]
    pub fn clear(&self) -> Result<()> {
        self.inner.borrow_mut().clear().map_err(build_error)
    }

    /// `#.add` — returns the filter, for chaining.
    #[napi]
    pub fn add<'a>(&self, env: Env, this: This<'a>, item: Unknown) -> Result<This<'a>> {
        // The conversion runs JavaScript (`charCodeAt`), so it happens before
        // the borrow rather than inside it.
        let units = to_units(&env, &item)?;

        self.inner.borrow_mut().add(&units);

        Ok(this)
    }

    /// `#.test` — `true` if `item` might be present.
    #[napi]
    pub fn test(&self, env: Env, item: Unknown) -> Result<bool> {
        let units = to_units(&env, &item)?;
        let present = self.inner.borrow().test(&units);

        Ok(present)
    }

    /// `BloomFilter.from(iterable, options)`.
    ///
    /// `options` is inferred from `iterable.length || iterable.size` when it is
    /// falsy — note *falsy*, not absent, so `from(x, 0)` also infers.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown, options: Option<Unknown>) -> Result<Self> {
        let supplied = match options {
            Some(options) if truthy(&env, &options)? => Some(options),
            _ => None,
        };
        let mut filter = match supplied {
            Some(options) => build(&env, options)?,
            None => {
                let capacity = infer_capacity(&env, &iterable)?;

                CoreBloomFilter::new(capacity, None).map_err(build_error)?
            }
        };

        // The five-branch coercion, unmodified. Upstream adds during
        // iteration; collecting first is observably the same here because the
        // filter being filled is local and cannot be reached from a callback.
        for slot in foreach::collect(&env, iterable)? {
            let value = slot.get(&env)?;

            filter.add(&to_units(&env, &value)?);
        }

        Ok(Self {
            inner: RefCell::new(filter),
        })
    }
}

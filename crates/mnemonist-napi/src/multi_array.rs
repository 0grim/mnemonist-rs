//! JS bridge for [`mnemonist_core::structures::multi_array`].
//!
//! # `capacity || null`: a falsy `capacity` is no capacity at all
//!
//! Upstream's constructor is `this.capacity = capacity || null;` — **any**
//! JS-falsy `capacity` (`0`, `NaN`, `undefined`, omitted) resolves to
//! dynamic mode, not just an omitted argument. `new MultiArray(Uint8Array,
//! 0)` is therefore a **dynamic**, unbounded `MultiArray`, not a
//! zero-capacity fixed one — reproduced here by [`truthy_capacity`] rather
//! than treating `Some(0.0)` as a real capacity.
//!
//! # `Container` is resolved by identity, same scope cut as `vector.rs`
//!
//! Only `Uint8Array`/`Uint16Array`/`Uint32Array` are modelled for the fixed
//! mode, and only in combination with a truthy `capacity` — see the core
//! module's docs for exactly which two `(Container, capacity)` combinations
//! `test/multi-array.js` exercises and which two it does not.
//!
//! # `containers`/`associations`/`values`/`entries`/`keys` are real iterators
//!
//! Upstream builds a genuine `obliterator/iterator` instance for each of
//! these — an object whose *shape* (an opaque `{next: fn}`, not an
//! `Array`) is part of what a caller can observe, even though
//! `test/multi-array.js` only ever drains one through `take()` (which
//! accepts either shape and so cannot tell the difference). An earlier
//! draft of this bridge returned a plain `Vec`/`Array` from all five methods
//! instead — every original assertion still passed, but the very first
//! differential-fuzz run caught it directly: `s.containers()` on a fresh,
//! empty `MultiArray` returned `[]` from the port against `{}` from upstream
//! (an `Iterator`'s own enumerable properties, which is what `encode()`
//! reduces an opaque object with no special-cased shape to). Fixed by
//! building a real `#[napi(iterator)]` generator for each, over a snapshot
//! taken at creation — matching upstream's frozen-`l`/live-read hybrid
//! exactly on every input either can reach, since nothing here can shrink
//! `dimension` or a bucket's length once created (only `push`/`set` ever
//! run, and `clear` is not part of upstream's own surface for this module).

use std::vec::IntoIter;

use mnemonist_core::structures::multi_array::{CapacityExceeded, MultiArray as CoreMultiArray};
use mnemonist_core::utils::typed_arrays::PointerWidth;
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

/// `mnemonist/multi-array.constructor: container should be a function.`
const CONTAINER_NOT_A_FUNCTION: &str =
    "mnemonist/multi-array.constructor: container should be a function.";

/// This port's own message when a fixed `capacity` is combined with a
/// `Container` other than the three modelled typed-array widths — untested
/// upstream (see the core module docs) and refused rather than silently
/// reinterpreted, the same call `vector.rs` makes for its own unmodelled
/// `ArrayClass` values.
const UNSUPPORTED_FIXED_CONTAINER: &str =
    "mnemonist-rs/MultiArray: a fixed `capacity` is only supported together with \
     Uint8Array/Uint16Array/Uint32Array as `Container`. Upstream would accept any \
     function here (including `Array`, which then breaks its own `.push` calls with no \
     capacity bound at all); this port models only the combination \
     `test/multi-array.js` exercises.";

const ARRAY_CLASSES: &[(&str, PointerWidth)] = &[
    ("Uint8Array", PointerWidth::U8),
    ("Uint16Array", PointerWidth::U16),
    ("Uint32Array", PointerWidth::U32),
];

/// `capacity || null` — any JS-falsy `capacity` (`0`, `NaN`, omitted) means
/// dynamic mode.
fn truthy_capacity(capacity: Option<f64>) -> Option<f64> {
    capacity.filter(|value| *value != 0.0 && !value.is_nan())
}

/// Which typed-array global `Container` matches by identity, if any.
fn resolve_container_width(env: &Env, container: &Unknown) -> Result<Option<PointerWidth>> {
    let global = env.get_global()?;

    for (name, width) in ARRAY_CLASSES {
        let candidate: Unknown = global.get_named_property_unchecked(name)?;

        if env.strict_equals(*container, candidate)? {
            return Ok(Some(*width));
        }
    }

    Ok(None)
}

fn raise(error: CapacityExceeded) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

/// A JS number used as an index/capacity — truncated, clamped to `0` for
/// anything non-finite or negative. Same treatment as `vector.rs`'s `count`.
fn count(value: f64) -> usize {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }

    value.trunc() as usize
}

/// The bucket [`JsMultiArray::get`]/`containers`/`associations` render:
/// a plain JS `Array` of exact numbers in dynamic mode, or the real typed
/// array (already width-narrowed by the core) in fixed mode.
pub struct RenderedBucket {
    values: Vec<f64>,
    width: Option<PointerWidth>,
}

impl ToNapiValue for RenderedBucket {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        match val.width {
            None => unsafe { ToNapiValue::to_napi_value(env, val.values) },
            Some(PointerWidth::U8) => {
                let bytes: Vec<u8> = val.values.iter().map(|&v| v as u8).collect();
                unsafe { ToNapiValue::to_napi_value(env, Uint8Array::new(bytes)) }
            }
            Some(PointerWidth::U16) => {
                let words: Vec<u16> = val.values.iter().map(|&v| v as u16).collect();
                unsafe { ToNapiValue::to_napi_value(env, Uint16Array::new(words)) }
            }
            Some(PointerWidth::U32) => {
                let words: Vec<u32> = val.values.iter().map(|&v| v as u32).collect();
                unsafe { ToNapiValue::to_napi_value(env, Uint32Array::new(words)) }
            }
        }
    }
}

/// `[index, container]`, for `#.associations`.
pub struct Association(u32, RenderedBucket);

impl ToNapiValue for Association {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let index = unsafe { ToNapiValue::to_napi_value(env, val.0)? };
        let container = unsafe { ToNapiValue::to_napi_value(env, val.1)? };

        let mut pair = std::ptr::null_mut();
        napi::check_status!(
            unsafe { sys::napi_create_array_with_length(env, 2, &mut pair) },
            "mnemonist-rs: failed to build a [index, container] pair"
        )?;
        napi::check_status!(
            unsafe { sys::napi_set_element(env, pair, 0, index) },
            "mnemonist-rs: failed to set an association's index"
        )?;
        napi::check_status!(
            unsafe { sys::napi_set_element(env, pair, 1, container) },
            "mnemonist-rs: failed to set an association's container"
        )?;

        Ok(pair)
    }
}

/// `[index, value]`, for `#.entries`.
pub struct Entry(u32, f64);

impl ToNapiValue for Entry {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let index = unsafe { ToNapiValue::to_napi_value(env, val.0)? };
        let value = unsafe { ToNapiValue::to_napi_value(env, val.1)? };

        let mut pair = std::ptr::null_mut();
        napi::check_status!(
            unsafe { sys::napi_create_array_with_length(env, 2, &mut pair) },
            "mnemonist-rs: failed to build an [index, value] pair"
        )?;
        napi::check_status!(
            unsafe { sys::napi_set_element(env, pair, 0, index) },
            "mnemonist-rs: failed to set an entry's index"
        )?;
        napi::check_status!(
            unsafe { sys::napi_set_element(env, pair, 1, value) },
            "mnemonist-rs: failed to set an entry's value"
        )?;

        Ok(pair)
    }
}

#[napi(js_name = "MultiArray")]
pub struct JsMultiArray {
    inner: CoreMultiArray,
    width: Option<PointerWidth>,
}

#[napi]
impl JsMultiArray {
    /// `new MultiArray(Container, capacity)`.
    #[napi(constructor)]
    pub fn new(env: Env, container: Option<Unknown>, capacity: Option<f64>) -> Result<Self> {
        let capacity = truthy_capacity(capacity);

        if let Some(container) = &container {
            if container.get_type()? != ValueType::Function {
                return Err(Error::new(Status::InvalidArg, CONTAINER_NOT_A_FUNCTION));
            }
        }

        match capacity {
            None => Ok(Self {
                inner: CoreMultiArray::new(),
                width: None,
            }),
            Some(capacity) => {
                let width = match &container {
                    Some(container) => resolve_container_width(&env, container)?,
                    None => None,
                };

                let Some(width) = width else {
                    return Err(Error::new(Status::InvalidArg, UNSUPPORTED_FIXED_CONTAINER));
                };

                Ok(Self {
                    inner: CoreMultiArray::fixed(width, count(capacity)),
                    width: Some(width),
                })
            }
        }
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.size() as u32
    }

    #[napi(getter)]
    pub fn dimension(&self) -> u32 {
        self.inner.dimension() as u32
    }

    fn rendered(&self, values: Vec<f64>) -> RenderedBucket {
        RenderedBucket {
            values,
            width: self.width,
        }
    }

    /// `#.set(index, item)`. Upstream returns `this`.
    #[napi]
    pub fn set<'a>(&mut self, this: This<'a>, index: f64, item: f64) -> Result<This<'a>> {
        self.inner.set(count(index), item).map_err(raise)?;

        Ok(this)
    }

    /// `#.push(item)`. Upstream returns `this`.
    #[napi]
    pub fn push<'a>(&mut self, this: This<'a>, item: f64) -> Result<This<'a>> {
        self.inner.push(item).map_err(raise)?;

        Ok(this)
    }

    #[napi]
    pub fn has(&self, index: f64) -> bool {
        self.inner.has(count(index))
    }

    #[napi]
    pub fn multiplicity(&self, index: f64) -> u32 {
        self.inner.multiplicity(count(index)) as u32
    }

    /// Upstream's `count`, an alias of `multiplicity`.
    #[napi]
    pub fn count(&self, index: f64) -> u32 {
        self.multiplicity(index)
    }

    /// `undefined` past `dimension`, not `null` — same fix `bi_map`/
    /// `multi_map` make over napi's own `Option` rendering.
    #[napi]
    pub fn get(&self, index: f64) -> Either<RenderedBucket, Undefined> {
        match self.inner.get(count(index)) {
            Some(values) => Either::A(self.rendered(values)),
            None => Either::B(()),
        }
    }

    /// A fresh iterator over every bucket, `get`'s order — see the module
    /// docs for why this is a real generator and not a plain array.
    #[napi]
    pub fn containers(&self) -> JsMultiArrayContainers {
        let values: Vec<RenderedBucket> = self
            .inner
            .containers()
            .into_iter()
            .map(|values| self.rendered(values))
            .collect();

        JsMultiArrayContainers {
            values: values.into_iter(),
        }
    }

    /// A fresh iterator over `[index, container]`, `get`'s order.
    #[napi]
    pub fn associations(&self) -> JsMultiArrayAssociations {
        let values: Vec<Association> = self
            .inner
            .associations()
            .into_iter()
            .map(|(index, values)| Association(index as u32, self.rendered(values)))
            .collect();

        JsMultiArrayAssociations {
            values: values.into_iter(),
        }
    }

    /// `#.values(index)` — a fresh iterator over global insertion order with
    /// no argument, or one bucket's reverse-insertion order with an index.
    /// See the core module docs for why the two orders differ.
    #[napi]
    pub fn values(&self, index: Option<f64>) -> JsMultiArrayValues {
        let values = match index {
            Some(index) => self.inner.values_at(count(index)),
            None => self.inner.values(),
        };

        JsMultiArrayValues {
            values: values.into_iter(),
        }
    }

    /// A fresh iterator over `[index, value]`.
    #[napi]
    pub fn entries(&self) -> JsMultiArrayEntries {
        let values: Vec<Entry> = self
            .inner
            .entries()
            .into_iter()
            .map(|(index, value)| Entry(index as u32, value))
            .collect();

        JsMultiArrayEntries {
            values: values.into_iter(),
        }
    }

    /// A fresh iterator over `0..dimension`.
    #[napi]
    pub fn keys(&self) -> JsMultiArrayKeys {
        let values: Vec<u32> = self.inner.keys().into_iter().map(|k| k as u32).collect();

        JsMultiArrayKeys {
            values: values.into_iter(),
        }
    }
}

macro_rules! snapshot_iterator {
    ($name:ident, $yield:ty, $js_name:literal) => {
        #[napi(iterator, js_name = $js_name)]
        pub struct $name {
            values: IntoIter<$yield>,
        }

        impl Generator for $name {
            type Yield = $yield;
            type Next = ();
            type Return = ();

            fn next(&mut self, _value: Option<()>) -> Option<$yield> {
                self.values.next()
            }

            /// A native `Iterator` has no `.return` method.
            fn complete(&mut self, _value: Option<()>) -> Option<$yield> {
                None
            }
        }
    };
}

snapshot_iterator!(
    JsMultiArrayContainers,
    RenderedBucket,
    "MultiArrayContainers"
);
snapshot_iterator!(
    JsMultiArrayAssociations,
    Association,
    "MultiArrayAssociations"
);
snapshot_iterator!(JsMultiArrayValues, f64, "MultiArrayValues");
snapshot_iterator!(JsMultiArrayEntries, Entry, "MultiArrayEntries");
snapshot_iterator!(JsMultiArrayKeys, u32, "MultiArrayKeys");

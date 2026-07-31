//! JS bridge for [`mnemonist_core::structures::static_disjoint_set`].
//!
//! Thin translation only: every behavioural decision lives in the core crate.
//! What happens here is shape adaptation between Rust and JS conventions.
//!
//! Three adaptations are worth knowing about:
//!
//! 1. **`mapping()` returns a real typed array.** Upstream picks the
//!    constructor at call time from `getPointerArray(this.dimension)` and the
//!    choice is observable to JS (`instanceof Uint8Array`). The core hands back
//!    a [`Mapping`] carrying the chosen [`PointerWidth`]; this layer rebuilds
//!    the matching JS type, expressed as a three-way union.
//! 2. **`union()` returns `this`.** Upstream returns the instance for chaining;
//!    the core returns a `bool` saying whether a merge happened. The bool is
//!    dropped here so the JS surface matches.
//! 3. **Out-of-range indices throw.** Upstream reads past a typed array, gets
//!    `undefined`, and returns garbage. The core panics instead (documented
//!    there). Panicking across the FFI boundary would take the Node process
//!    down, so indices are checked here and surfaced as a JS `RangeError`
//!    instead. This is a deliberate divergence from "garbage", on the grounds
//!    that no honest reproduction of `undefined` arithmetic exists in Rust.

use mnemonist_core::structures::static_disjoint_set::StaticDisjointSet as CoreSet;
use mnemonist_core::utils::typed_arrays::PointerWidth;
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// Static disjoint set (union-find) over the items `0..size`.
#[napi(js_name = "StaticDisjointSet")]
pub struct JsStaticDisjointSet {
    inner: CoreSet,
}

#[napi]
impl JsStaticDisjointSet {
    #[napi(constructor)]
    pub fn new(size: u32) -> Result<Self> {
        CoreSet::new(size as usize)
            .map(|inner| Self { inner })
            .map_err(|message| Error::new(Status::GenericFailure, message))
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.size() as u32
    }

    #[napi(getter)]
    pub fn dimension(&self) -> u32 {
        self.inner.dimension() as u32
    }

    #[napi]
    pub fn find(&mut self, x: u32) -> Result<u32> {
        self.check(x)?;

        Ok(self.inner.find(x as usize) as u32)
    }

    #[napi]
    pub fn union<'a>(&mut self, this: This<'a>, x: u32, y: u32) -> Result<This<'a>> {
        self.check(x)?;
        self.check(y)?;
        // Core reports whether a merge occurred; upstream exposes that only
        // through `dimension`, and returns the instance for chaining.
        self.inner.union(x as usize, y as usize);

        Ok(this)
    }

    #[napi]
    pub fn connected(&mut self, x: u32, y: u32) -> Result<bool> {
        self.check(x)?;
        self.check(y)?;

        Ok(self.inner.connected(x as usize, y as usize))
    }

    /// Set id per item, as the same typed array upstream would have allocated.
    #[napi]
    pub fn mapping(&mut self) -> Either3<Uint8Array, Uint16Array, Uint32Array> {
        let mapping = self.inner.mapping();
        let width = mapping.width();
        let values = mapping.into_values();

        // Widths are chosen so every value fits; the casts cannot truncate.
        match width {
            PointerWidth::U8 => Either3::A(Uint8Array::new(
                values.into_iter().map(|v| v as u8).collect(),
            )),
            PointerWidth::U16 => Either3::B(Uint16Array::new(
                values.into_iter().map(|v| v as u16).collect(),
            )),
            PointerWidth::U32 => Either3::C(Uint32Array::new(values)),
        }
    }

    /// One ascending item list per set, sets in first-encounter order.
    #[napi]
    pub fn compile(&mut self) -> Vec<Vec<u32>> {
        self.inner
            .compile()
            .into_iter()
            .map(|set| set.into_iter().map(|item| item as u32).collect())
            .collect()
    }

    /// Guard the core's documented panic on out-of-range indices (see 3 above).
    fn check(&self, index: u32) -> Result<()> {
        if (index as usize) < self.inner.size() {
            return Ok(());
        }

        Err(Error::new(
            Status::InvalidArg,
            format!(
                "index {index} is out of range for a StaticDisjointSet of size {}",
                self.inner.size()
            ),
        ))
    }
}

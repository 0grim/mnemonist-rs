//! JS bridge for [`mnemonist_core::structures::kd_tree`].
//!
//! Upstream's own "raw" constructor, `function KDTree(dimensions, build)`,
//! takes an already-built internal shape (`{axes, labels, pivots, lefts,
//! rights}`) that only `.from`/`.fromAxes` themselves ever produce — no test
//! anywhere calls `new KDTree(...)` directly. This bridge does not expose it
//! either: [`JsKdTree`] is reachable only through its two static factories,
//! exactly how the original module is used.
//!
//! Labels and points are read once, at construction, directly off the
//! caller's row/axis arrays -- there is no per-query callback here at all
//! (unlike `vp-tree.js`'s `distance`), so none of that module's re-entrancy
//! questions apply.

use mnemonist_core::structures::kd_tree::KdTree as CoreTree;
use mnemonist_core::utils::typed_arrays::{PointerVec, PointerWidth};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::foreach;
use crate::js_slot::JsSlot;

fn typed_from_pointer_vec(values: &PointerVec) -> Either3<Uint8Array, Uint16Array, Uint32Array> {
    match values.width() {
        PointerWidth::U8 => Either3::A(Uint8Array::new(
            (0..values.len()).map(|i| values.get(i) as u8).collect(),
        )),
        PointerWidth::U16 => Either3::B(Uint16Array::new(
            (0..values.len()).map(|i| values.get(i) as u16).collect(),
        )),
        PointerWidth::U32 => Either3::C(Uint32Array::new(
            (0..values.len()).map(|i| values.get(i)).collect(),
        )),
    }
}

/// `KDTree.from`'s `data` shape, `[label, [x, y, ...]]` per row, materialised
/// off the caller's iterable via `foreach::collect` (each row is one
/// top-level element) and then indexed directly -- upstream's own
/// `reshapeIntoAxes` does exactly this once `iterables.toArray` has run.
fn rows_from_iterable(
    env: &Env,
    iterable: Unknown,
    dimensions: usize,
) -> Result<Vec<(JsSlot, Vec<f64>)>> {
    let row_slots = foreach::collect(env, iterable)?;
    let mut rows = Vec::with_capacity(row_slots.len());

    for row_slot in row_slots {
        let row_value = row_slot.get(env)?;

        // SAFETY: upstream's own `row[1][d]` access assumes `row` is
        // array-like; a row that is not one throws upstream too (reading a
        // numeric property off a non-object), which `cast::<Array>` failing
        // here reproduces as a catchable error rather than a panic.
        let row_array = unsafe { row_value.cast::<Array>()? };

        // `Array::get` answers `Ok(None)` for any index past the array's
        // length -- it bounds-checks before it ever calls into V8 -- so
        // `undefined` and "absent" are the same answer here, and neither is
        // an error. Every `expect` on these three reads was wrong: each one
        // panicked on a short row, and a panic crossing N-API aborts the host
        // process instead of throwing. Verified against Node 24.18.1 for what
        // upstream does with each shape.
        //
        // Upstream reads `row[1][d]` *before* `labels[i] = row[0]`, and both
        // live inside `for (d = 0; d < dimensions; d++)`, so a row missing
        // index 1 throws before the label is ever read, and a zero-dimension
        // tree reads neither.
        let point_value: Option<Unknown> = row_array.get(1)?;

        let point = match point_value {
            Some(value) => {
                // SAFETY: same reasoning as the row cast above.
                let point_array = unsafe { value.cast::<Array>()? };
                let mut point = Vec::with_capacity(dimensions);

                for d in 0..dimensions as u32 {
                    // A component past the end of the point is `undefined`,
                    // which upstream stores into a `Float64Array` as `NaN`.
                    // `KDTree.from([['a', [1]]], 2)` builds upstream; it used
                    // to abort the process here.
                    point.push(point_array.get(d)?.unwrap_or(f64::NAN));
                }

                point
            }
            // `row[1][d]` on a row with no index 1 is a read off `undefined`,
            // which throws upstream -- but only if the dimension loop runs at
            // all.
            None if dimensions > 0 => {
                return Err(foreach::type_error(
                    env,
                    "Cannot read properties of undefined (reading '0')",
                ))
            }
            None => Vec::new(),
        };

        // Reachable only past the check above, so either the row has an index
        // 1 -- and therefore an index 0 -- or `dimensions` is 0, where
        // upstream leaves `labels` a hole and never reads this either.
        let label_slot = match row_array.get::<Unknown>(0)? {
            Some(label) => JsSlot::new(env, &label)?,
            None => JsSlot::Undefined,
        };

        rows.push((label_slot, point));
    }

    Ok(rows)
}

#[napi(js_name = "KDTree")]
pub struct JsKdTree {
    inner: CoreTree<JsSlot>,
}

#[napi]
impl JsKdTree {
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.size() as u32
    }

    #[napi(getter)]
    pub fn dimensions(&self) -> u32 {
        self.inner.dimensions() as u32
    }

    #[napi(getter)]
    pub fn pivots(&self) -> Either3<Uint8Array, Uint16Array, Uint32Array> {
        typed_from_pointer_vec(self.inner.pivots())
    }

    #[napi(getter)]
    pub fn lefts(&self) -> Either3<Uint8Array, Uint16Array, Uint32Array> {
        typed_from_pointer_vec(self.inner.lefts())
    }

    #[napi(getter)]
    pub fn rights(&self) -> Either3<Uint8Array, Uint16Array, Uint32Array> {
        typed_from_pointer_vec(self.inner.rights())
    }

    /// `#.nearestNeighbor`. `None`/`undefined` on an empty tree; see the core
    /// module's Deliberate divergences.
    #[napi]
    pub fn nearest_neighbor(&self, query: Vec<f64>) -> Option<JsSlot> {
        self.inner.nearest_neighbor(&query).cloned()
    }

    /// `#.kNearestNeighbors`.
    #[napi]
    pub fn k_nearest_neighbors(&self, k: u32, query: Vec<f64>) -> Result<Vec<JsSlot>> {
        self.inner
            .k_nearest_neighbors(k as usize, &query)
            .map_err(|message| Error::new(Status::GenericFailure, message))
    }

    /// `#.linearKNearestNeighbors`.
    #[napi]
    pub fn linear_k_nearest_neighbors(&self, k: u32, query: Vec<f64>) -> Result<Vec<JsSlot>> {
        self.inner
            .linear_k_nearest_neighbors(k as usize, &query)
            .map_err(|message| Error::new(Status::GenericFailure, message))
    }

    /// `KDTree.from(iterable, dimensions)`.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown, dimensions: u32) -> Result<Self> {
        let dimensions = dimensions as usize;
        let rows = rows_from_iterable(&env, iterable, dimensions)?;

        Ok(Self {
            inner: CoreTree::from_rows(rows, dimensions)
                .map_err(|thrown| foreach::type_error(&env, thrown.0))?,
        })
    }

    /// `KDTree.fromAxes(axes, labels)`. `labels` defaults to
    /// `typed.indices(axes[0].length)` upstream -- a typed array of positional
    /// indices, each a plain JS number when read back. [`JsSlot::Number`]
    /// constructs that number directly rather than round-tripping through a
    /// real napi value, since a `JsSlot` needs no live handle for a
    /// primitive.
    #[napi(factory, js_name = "fromAxes")]
    pub fn from_axes(
        env: Env,
        axes: Vec<Vec<f64>>,
        labels: Option<Vec<JsUnknownLabel>>,
    ) -> Result<Self> {
        let n = axes.first().map(Vec::len).unwrap_or(0);

        let labels: Vec<JsSlot> = match labels {
            Some(given) => given.into_iter().map(|label| label.0).collect(),
            None => (0..n).map(|i| JsSlot::Number(i as f64)).collect(),
        };

        Ok(Self {
            inner: CoreTree::from_axes(axes, labels)
                .map_err(|thrown| foreach::type_error(&env, thrown.0))?,
        })
    }
}

/// A thin wrapper so `Vec<JsUnknownLabel>` can be a `#[napi]` parameter type
/// (`JsSlot` itself implements only [`ToNapiValue`], never the `From`
/// direction — see `js_slot.rs`'s own docs on why capture is asymmetric).
pub struct JsUnknownLabel(JsSlot);

impl FromNapiValue for JsUnknownLabel {
    unsafe fn from_napi_value(env: sys::napi_env, napi_val: sys::napi_value) -> Result<Self> {
        let js_env = Env::from_raw(env);
        // SAFETY: `napi_val` is a live handle from `env`, handed in by napi-rs
        // itself for this parameter.
        let unknown = unsafe { Unknown::from_raw_unchecked(env, napi_val) };

        JsSlot::new(&js_env, &unknown).map(JsUnknownLabel)
    }
}

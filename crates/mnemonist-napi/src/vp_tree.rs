//! JS bridge for [`mnemonist_core::structures::vp_tree`].
//!
//! Items are arbitrary JS values, held as [`JsSlot`] exactly as
//! `crate::bk_tree`'s are. Unlike `bk_tree.rs`, this bridge needs **no
//! `RefCell`**: `VPTree` has no `add`/mutation after construction (see the
//! core module's docs), so every method here only ever needs `&self`, and a
//! distance function that calls back into the same tree's query methods just
//! runs an independent, fully-formed query rather than meeting an
//! outstanding borrow. That is a real behavioural difference from upstream,
//! whose `this.heap`/`this.D` are shared instance state a reentrant call
//! would corrupt — recorded in `docs/modules/vp-tree.md`'s Deliberate
//! divergences rather than silently left "more correct."
//!
//! `nodes`/`lefts`/`rights` are exposed as whichever real typed-array class
//! [`mnemonist_core::utils::typed_arrays::get_pointer_array`] chose for the
//! tree's size — the same `Either3<Uint8Array, Uint16Array, Uint32Array>`
//! shape `crate::static_disjoint_set`'s `mapping()` already established for
//! exactly this situation.

use mnemonist_core::structures::vp_tree::{Neighbor, VpTree as CoreTree};
use mnemonist_core::utils::typed_arrays::{PointerVec, PointerWidth};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::foreach;
use crate::js_slot::JsSlot;

/// Distance function: `(a, b) -> number`.
type Distance = FunctionRef<FnArgs<(JsSlot, JsSlot)>, f64>;

const NOT_A_FUNCTION: &str = "mnemonist/VPTree.constructor: given `distance` must be a function.";
const NO_ITEMS: &str = "mnemonist/VPTree.constructor: you must provide items to the tree. A \
     VPTree cannot be updated after its creation.";

fn call_distance(env: &Env, distance: &Distance, a: &JsSlot, b: &JsSlot) -> Result<f64> {
    let callable = distance.borrow_back(env)?;

    callable.call((a.clone(), b.clone()).into())
}

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

/// A falsy `items` argument -- upstream's `if (!items) throw`. `Option`
/// alone only catches `undefined`; `null` is falsy too and the constructor
/// test (`new VPTree(Function.prototype)`, `items` omitted) only reaches
/// `undefined`, but `null` is one JS call away and just as falsy.
fn is_falsy_items(items: &Option<Unknown>) -> Result<bool> {
    match items {
        None => Ok(true),
        Some(value) => Ok(value.get_type()? == ValueType::Null),
    }
}

#[napi(js_name = "VPTree")]
pub struct JsVpTree {
    inner: CoreTree<JsSlot>,
    distance: Distance,
}

#[napi]
impl JsVpTree {
    /// `new VPTree(distance, items)`.
    #[napi(constructor)]
    pub fn new(env: Env, distance: Unknown, items: Option<Unknown>) -> Result<Self> {
        if distance.get_type()? != ValueType::Function {
            return Err(Error::new(Status::InvalidArg, NOT_A_FUNCTION));
        }

        if is_falsy_items(&items)? {
            return Err(Error::new(Status::InvalidArg, NO_ITEMS));
        }

        // SAFETY: `get_type` has just reported `Function`.
        let function = unsafe { distance.cast::<Function<FnArgs<(JsSlot, JsSlot)>, f64>>()? };
        let distance_ref = function.create_ref()?;

        let materialized = foreach::collect(&env, items.expect("checked non-falsy above"))?;

        let inner = CoreTree::try_new(materialized, |a, b| {
            call_distance(&env, &distance_ref, a, b)
        })?;

        Ok(Self {
            inner,
            distance: distance_ref,
        })
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.size() as u32
    }

    #[napi(getter)]
    pub fn nodes(&self) -> Either3<Uint8Array, Uint16Array, Uint32Array> {
        typed_from_pointer_vec(self.inner.nodes())
    }

    #[napi(getter)]
    pub fn lefts(&self) -> Either3<Uint8Array, Uint16Array, Uint32Array> {
        typed_from_pointer_vec(self.inner.lefts())
    }

    #[napi(getter)]
    pub fn rights(&self) -> Either3<Uint8Array, Uint16Array, Uint32Array> {
        typed_from_pointer_vec(self.inner.rights())
    }

    #[napi(getter)]
    pub fn mus(&self) -> Float64Array {
        Float64Array::new(self.inner.mus().to_vec())
    }

    /// `#.nearestNeighbors`.
    #[napi]
    pub fn nearest_neighbors(&self, env: Env, k: u32, query: Unknown) -> Result<Vec<JsNeighbor>> {
        let query_slot = JsSlot::new(&env, &query)?;

        let neighbors = self.inner.try_nearest_neighbors(k as usize, &query_slot, |a, b| {
            call_distance(&env, &self.distance, a, b)
        })?;

        Ok(neighbors.into_iter().map(JsNeighbor::from).collect())
    }

    /// `#.neighbors`.
    #[napi]
    pub fn neighbors(&self, env: Env, radius: f64, query: Unknown) -> Result<Vec<JsNeighbor>> {
        let query_slot = JsSlot::new(&env, &query)?;

        let found = self.inner.try_neighbors(radius, &query_slot, |a, b| {
            call_distance(&env, &self.distance, a, b)
        })?;

        Ok(found.into_iter().map(JsNeighbor::from).collect())
    }

    /// `VPTree.from(iterable, distance)` -- note the swapped argument order
    /// relative to the constructor, upstream's own:
    /// `VPTree.from = function(iterable, distance) { return new VPTree(distance, iterable); };`
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown, distance: Unknown) -> Result<Self> {
        Self::new(env, distance, Some(iterable))
    }
}

/// One `nearestNeighbors`/`neighbors` hit: upstream's `{distance, item}`.
pub struct JsNeighbor {
    distance: f64,
    item: JsSlot,
}

impl From<Neighbor<JsSlot>> for JsNeighbor {
    fn from(neighbor: Neighbor<JsSlot>) -> Self {
        Self {
            distance: neighbor.distance,
            item: neighbor.item,
        }
    }
}

impl ToNapiValue for JsNeighbor {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let js_env = Env::from_raw(env);
        let mut object = Object::new(&js_env)?;

        object.set("distance", val.distance)?;
        object.set("item", val.item)?;

        // SAFETY: `object` is a live handle from `env`, produced above.
        unsafe { ToNapiValue::to_napi_value(env, object) }
    }
}

//! JS bridge for [`mnemonist_core::structures::bk_tree`].
//!
//! Not a T3 module — `bk-tree.js` never touches a `Map`, so nothing here
//! needs `JsKey` or `OrderedMap`. Items are arbitrary JS values, held exactly
//! as [`crate::stack`]/[`crate::queue`] hold theirs: as [`JsSlot`], and built
//! through the same [`crate::foreach::collect`] dispatch for `.from`.
//!
//! # The re-entrancy hazard this module actually has
//!
//! `distance` is a JS callback invoked repeatedly *from inside* both `add`'s
//! descent and `search`'s traversal — the same shape as
//! `crate::bit_vector`'s growth policy (PORTBUG-1), and for the same reason: the
//! `RefCell` borrow this bridge takes for the whole call cannot be released
//! between distance calls, because `mnemonist-core`'s `try_add`/`try_search`
//! own the loop and know nothing about `RefCell`. So, exactly as
//! `bit_vector.rs` does:
//!
//! * every method that touches the tree returns `Result`, because the borrow
//!   can fail;
//! * `read`/`write` use `try_borrow`/`try_borrow_mut`, never the panicking
//!   forms — a `RefCell` panic inside a `#[napi]` method aborts the process
//!   (napi 3.12 does not `catch_unwind` a sync call; measured, not assumed,
//!   the same finding `bit_vector.rs` and `default_map.rs` both record);
//! * a distance function that calls back into the *same* tree while it is
//!   walking meets that outstanding borrow and gets a clear, catchable
//!   [`REENTRANT_DISTANCE`] error instead of a crash. Upstream would serve
//!   such a call from a tree that is mid-traversal and get whatever
//!   half-built state it finds; this port refuses instead — narrower than
//!   upstream, and recorded as a divergence rather than hidden, the same
//!   trade `bit_vector.rs` makes for its growth policy.
//!
//! One improvement over that precedent, made possible by writing this core
//! type from scratch rather than inheriting `bit_vector`'s pre-existing
//! `Box<dyn Fn(f64) -> Option<f64>>` shape: [`mnemonist_core::structures::bk_tree::BkTree::try_add`]/
//! [`try_search`](mnemonist_core::structures::bk_tree::BkTree::try_search) are
//! genuinely fallible (`FnMut(&I, &I) -> Result<i64, E>`), so a JS distance
//! function that *throws* propagates as a real `Err` through core's own
//! `Result`, with no side-channel `Rc<RefCell<Option<Error>>>` needed to park
//! an exception the core type cannot otherwise express.

use std::cell::{Ref, RefCell, RefMut};

use mnemonist_core::structures::bk_tree::{BkTree as CoreTree, Found};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::foreach;
use crate::js_slot::JsSlot;

/// Distance function: `(a, b) -> number`. `JsSlot` round-trips through
/// `ToNapiValue` already, so — unlike `fuzzy_map`'s hash functions — no
/// `Unknown<'static>` reconstruction is needed: the stored items themselves
/// are valid, owned arguments to hand back to JavaScript.
type Distance = FunctionRef<FnArgs<(JsSlot, JsSlot)>, f64>;

const NOT_A_FUNCTION: &str = "mnemonist/BKTree.constructor: given `distance` should be a function.";

const REENTRANT_DISTANCE: &str = "mnemonist-rs/BKTree: the distance function called back into \
     the tree while it was walking. Upstream would serve such a call from a tree that is \
     mid-traversal and get whatever half-built state it finds; this port refuses it instead, \
     catchably. See PORTBUG-1 and the module docs.";

#[napi(js_name = "BKTree")]
pub struct JsBkTree {
    inner: RefCell<CoreTree<JsSlot>>,
    distance: Distance,
}

impl JsBkTree {
    /// A shared borrow, or the re-entrancy error. Never `borrow()` — see the
    /// module docs for why a panic here would take the process down rather
    /// than reach JavaScript.
    fn read(&self) -> Result<Ref<'_, CoreTree<JsSlot>>> {
        self.inner
            .try_borrow()
            .map_err(|_| Error::new(Status::GenericFailure, REENTRANT_DISTANCE))
    }

    /// A mutable borrow, or the re-entrancy error.
    fn write(&self) -> Result<RefMut<'_, CoreTree<JsSlot>>> {
        self.inner
            .try_borrow_mut()
            .map_err(|_| Error::new(Status::GenericFailure, REENTRANT_DISTANCE))
    }

    fn call_distance(&self, env: &Env, a: &JsSlot, b: &JsSlot) -> Result<i64> {
        let callable = self.distance.borrow_back(env)?;
        let result = callable.call((a.clone(), b.clone()).into())?;

        Ok(result as i64)
    }
}

#[napi]
impl JsBkTree {
    /// `new BKTree(distance)`.
    #[napi(constructor)]
    pub fn new(distance: Unknown) -> Result<Self> {
        if distance.get_type()? != ValueType::Function {
            return Err(Error::new(Status::InvalidArg, NOT_A_FUNCTION));
        }

        // SAFETY: `get_type` has just reported `Function`, which is the
        // precondition `Unknown::cast` documents.
        let function = unsafe { distance.cast::<Function<FnArgs<(JsSlot, JsSlot)>, f64>>()? };

        Ok(Self {
            inner: RefCell::new(CoreTree::new()),
            distance: function.create_ref()?,
        })
    }

    #[napi(getter)]
    pub fn size(&self) -> Result<u32> {
        Ok(self.read()?.size() as u32)
    }

    #[napi]
    pub fn clear(&self) -> Result<()> {
        *self.write()? = CoreTree::new();

        Ok(())
    }

    /// Upstream's `add`, which returns `this` for chaining.
    ///
    /// The borrow is held for the whole call — see the module docs for why it
    /// cannot be released between the distance function's repeated calls the
    /// way `forEach`'s per-step borrow can.
    #[napi]
    pub fn add<'a>(&self, this: This<'a>, env: Env, item: Unknown) -> Result<This<'a>> {
        let slot = JsSlot::new(&env, &item)?;
        let mut tree = self.write()?;

        tree.try_add(slot, |a, b| self.call_distance(&env, a, b))?;

        Ok(this)
    }

    /// Upstream's `search`, returning `[]` when the tree is empty.
    #[napi]
    pub fn search(&self, env: Env, n: f64, query: Unknown) -> Result<Vec<JsFound>> {
        let query_slot = JsSlot::new(&env, &query)?;
        let tree = self.read()?;

        let found =
            tree.try_search(n as i64, &query_slot, |a, b| self.call_distance(&env, a, b))?;

        Ok(found.into_iter().map(JsFound::from).collect())
    }

    /// `BKTree.from(iterable, distance)`.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown, distance: Unknown) -> Result<Self> {
        let built = Self::new(distance)?;
        let items = foreach::collect(&env, iterable)?;

        {
            // The object is not yet handed to JS, so nothing can re-enter it
            // here: a plain `borrow_mut` (rather than `write()`) would also be
            // sound, but going through the same fallible path keeps this
            // method exercising the identical borrow discipline as `add`.
            let mut tree = built
                .inner
                .try_borrow_mut()
                .map_err(|_| Error::new(Status::GenericFailure, REENTRANT_DISTANCE))?;

            for item in items {
                tree.try_add(item, |a, b| built.call_distance(&env, a, b))?;
            }
        }

        Ok(built)
    }
}

/// One `search` hit: upstream's `{item, distance}`.
pub struct JsFound {
    item: JsSlot,
    distance: i64,
}

impl From<Found<JsSlot>> for JsFound {
    fn from(found: Found<JsSlot>) -> Self {
        Self {
            item: found.item,
            distance: found.distance,
        }
    }
}

impl ToNapiValue for JsFound {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let js_env = Env::from_raw(env);
        let mut object = Object::new(&js_env)?;

        object.set("item", val.item)?;
        object.set("distance", val.distance as f64)?;

        // SAFETY: `object` is a live handle from `env`, produced above.
        unsafe { ToNapiValue::to_napi_value(env, object) }
    }
}

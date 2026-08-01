//! JS bridge for [`mnemonist_core::structures::sparse_set`].
//!
//! Thin translation only: every behavioural decision lives in the core crate.
//! Four adaptations are worth knowing about.
//!
//! 1. **`add` returns `this`.** Upstream returns the instance for chaining;
//!    the core returns whether the member was newly inserted, which upstream
//!    exposes only through `size`. The bool is dropped here.
//! 2. **Out-of-range members are *not* guarded.** Unlike the
//!    `StaticDisjointSet` bridge, which raises a `RangeError` because upstream
//!    propagates `NaN` from an out-of-range read, everything `SparseSet` does
//!    off the end of its arrays is well defined and is reproduced in the core.
//!    A member past `length` therefore behaves exactly as it does upstream,
//!    corruption included.
//! 3. **`dense` and `sparse` are not exposed.** They are public typed arrays
//!    upstream, and a JS caller can write through them. napi can only hand out
//!    a *copy* of a Rust `Vec`, so exposing them would silently break that
//!    write-through and be worse than their absence. The original test file
//!    never reads either.
//! 4. **`forEach`'s `scope` argument.** Upstream keys off `arguments.length`,
//!    which napi's typed signature cannot see; see [`JsSparseSet::for_each`].
//!
//! Like [`crate::queue`] and [`crate::stack`], the core structure is held in a
//! [`RefCell`] so that `&self` is not `noalias readonly` and a JS callback's
//! mutation is actually seen. See [`crate::cursor::CellCursor`] for the
//! measured failure; B-31 is this module's instance of it.

use std::cell::RefCell;

use mnemonist_core::structures::sparse_set::SparseSet as CoreSet;
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::cursor::{yielded, CellCursor};

/// A sparse set over the members `0..length`.
#[napi(js_name = "SparseSet")]
pub struct JsSparseSet {
    inner: RefCell<CoreSet>,
}

#[napi]
impl JsSparseSet {
    #[napi(constructor)]
    pub fn new(length: u32) -> Result<Self> {
        CoreSet::new(length as usize)
            .map(|inner| Self {
                inner: RefCell::new(inner),
            })
            .map_err(|message| Error::new(Status::GenericFailure, message))
    }

    /// Members currently in the set. Can exceed `length`; see the core docs.
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    /// Capacity the set was built with.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.borrow().length() as u32
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    #[napi]
    pub fn has(&self, member: u32) -> bool {
        self.inner.borrow().has(member as usize)
    }

    /// Upstream returns `this` for chaining.
    #[napi]
    pub fn add<'a>(&self, this: This<'a>, member: u32) -> This<'a> {
        self.inner.borrow_mut().add(member as usize);

        this
    }

    #[napi(js_name = "delete")]
    pub fn delete(&self, member: u32) -> bool {
        self.inner.borrow_mut().delete(member as usize)
    }

    /// A fresh cursor over the members, in `dense` order.
    ///
    /// This is the *factory* half of D-07: every call constructs a new cursor
    /// object, so `[...set]` works repeatedly, while each cursor is
    /// individually non-restartable. `crate::cursor::install_iterator_factories`
    /// aliases `Symbol.iterator` onto this method, exactly as upstream's last
    /// line of `sparse-set.js` does.
    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsSparseSet>) -> Result<JsSparseSetValues> {
        // `share_with` projects the JS-owned instance to the `RefCell` around
        // the core structure and keeps the instance alive for the cursor's
        // whole life. The projection deliberately stops at the cell rather
        // than at the set: `CellCursor` re-borrows on every `step`, so element
        // writes during iteration are visible (D-08) *and* the borrow is never
        // outstanding while JS is on the stack.
        let source = this.share_with(env, |set| Ok(&set.inner))?;

        Ok(JsSparseSetValues {
            cursor: CellCursor::open(source),
        })
    }

    /// Upstream's own `forEach` — a plain loop over `dense`, not obliterator's.
    ///
    /// Two fidelity notes:
    ///
    /// * The callback is invoked as `(item, item)`. Upstream passes the member
    ///   as both value *and* key, which is how a `SparseSet` mimics a `Set`.
    /// * `scope` is upstream's `arguments.length > 1 ? scope : this`. napi's
    ///   typed signature cannot distinguish "second argument omitted" from
    ///   "second argument passed as `undefined`", so `forEach(cb, undefined)`
    ///   binds the callback's `this` to the set here, where upstream binds it
    ///   to `undefined` (and therefore, for a sloppy-mode callback, to
    ///   `globalThis`). Recorded as a deliberate divergence; the omitted-
    ///   argument case, which is the only one the original suite uses, is
    ///   exact.
    ///
    /// Unlike `size` this loop re-reads the set on every iteration, matching
    /// upstream's `i < this.size` — a callback that deletes members shortens
    /// the loop. That is *not* the frozen-length cursor behaviour, and the
    /// difference between the two is upstream's, not ours.
    #[napi]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(u32, u32)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let mut index = 0;

        // Each iteration re-borrows and drops before the callback runs: the
        // callback may `add`, `delete` or `clear` through the same object, and
        // an outstanding borrow would turn upstream's ordinary behaviour into
        // a `BorrowMutError`. The `RefCell` is also what stops LLVM hoisting
        // the `size` read out of this loop entirely (B-31).
        while index < self.inner.borrow().size() {
            // `undefined` past the end of `dense` reaches the callback as
            // `undefined` upstream; the loop bound makes that unreachable for
            // every in-range set, and for a corrupted one the core reports it
            // the same way the cursor does.
            let item = self
                .inner
                .borrow()
                .dense()
                .try_get(index)
                .unwrap_or_default();

            match &scope {
                Some(scope) => callback.apply(*scope, (item, item).into())?,
                None => callback.apply(this, (item, item).into())?,
            };

            index += 1;
        }

        Ok(())
    }
}

/// The cursor `SparseSet.prototype.values()` hands out.
///
/// `#[napi(iterator)]` supplies the identity half of D-07 for free: this
/// object's own `Symbol.iterator` returns itself, so it is non-restartable,
/// and partial consumption followed by a spread continues rather than
/// restarting. Verified pre-kickoff against Node 24 (DESIGN.md 11.3).
#[napi(iterator, js_name = "SparseSetValues")]
pub struct JsSparseSetValues {
    cursor: CellCursor<JsSparseSet, CoreSet>,
}

impl Generator for JsSparseSetValues {
    /// `Either<u32, Undefined>`, not `Option<u32>`: napi renders `None` as
    /// `null`, and the shrink window needs a real `undefined`. See
    /// `crate::cursor::yielded`.
    type Yield = Either<u32, Undefined>;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        yielded(self.cursor.step())
    }

    /// `Generator.return()`, which is what a `break` out of a `for…of` calls.
    ///
    /// Upstream cursors have no `return` method at all, so `break` leaves the
    /// cursor exactly where it stopped and a later `next()` resumes. napi's
    /// default `complete` returns `None`, which is the same observable
    /// behaviour — the walk is not reset and not force-finished — so it is
    /// left alone rather than overridden.
    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}

//! JS bridge for [`mnemonist_core::structures::linked_list`].
//!
//! Values are [`JsSlot`], exactly as in [`crate::stack`]/[`crate::queue`]:
//! arbitrary JS values that survive between calls, `Clone` cheaply (an `Rc`
//! bump), and release themselves through their own `Drop` — no
//! `#[napi(custom_finalize)]`, no explicit `.release(env)` calls anywhere in
//! this file, unlike the T3 modules' [`crate::js_value::Retained`]. That
//! difference is what makes the arena-retention divergence the core module's
//! docs describe self-cleaning rather than a leak-tracking obligation this
//! bridge has to remember: when the whole `RefCell<Core>` is dropped (the JS
//! `LinkedList` object is collected), every arena slot's `JsSlot` drops with
//! it and releases its `napi_ref` there — later than upstream would, per the
//! core module's own docs, but never never.
//!
//! The core structure is held in a [`RefCell`] for the same reason as every
//! other bridge in this port past `default-map`: `&self` on a bare core value
//! is `noalias readonly`, and a `forEach`/factory callback that calls back
//! into the same list must not meet an outstanding borrow (B-31).
//!
//! # The three iterators share one core cursor
//!
//! `values()`, `entries()` and `forEach` all drive
//! `mnemonist_core::structures::linked_list::ListCursor` — see that module's
//! docs for why one primitive covers all three here, unlike `lru-cache`'s
//! split between `Sequence` and `ForEachWalk` (D-90).

use std::cell::RefCell;

use mnemonist_core::structures::linked_list::{LinkedList as CoreList, ListCursor};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::foreach;
use crate::js_slot::JsSlot;

/// The list as core sees it: arbitrary JS values.
type Core = CoreList<JsSlot>;

/// A singly linked list of arbitrary JavaScript values.
#[napi(js_name = "LinkedList")]
pub struct JsLinkedList {
    inner: RefCell<Core>,
}

impl Default for JsLinkedList {
    fn default() -> Self {
        Self::new()
    }
}

#[napi]
impl JsLinkedList {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(Core::new()),
        }
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    /// Upstream's `first`.
    #[napi]
    pub fn first(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow().first().cloned().into()
    }

    /// Upstream's `last`.
    ///
    /// See `mnemonist_core::structures::linked_list`'s module docs for
    /// B-241: after `shift()` has emptied the list, this reproduces
    /// upstream's stale answer (the just-removed item) rather than
    /// `undefined`, because core's `last()` does — nothing bridge-specific
    /// is needed to reach the bug, which is the point of fixing it at the
    /// one place both the mocha bridge and the fuzzer read from.
    #[napi]
    pub fn last(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow().last().cloned().into()
    }

    /// Upstream's `peek`, an alias for `first`.
    #[napi]
    pub fn peek(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow().peek().cloned().into()
    }

    /// Upstream returns the new size.
    #[napi]
    pub fn push(&self, env: Env, item: Unknown) -> Result<u32> {
        let slot = JsSlot::new(&env, &item)?;

        Ok(self.inner.borrow_mut().push(slot) as u32)
    }

    /// Upstream returns the new size.
    #[napi]
    pub fn unshift(&self, env: Env, item: Unknown) -> Result<u32> {
        let slot = JsSlot::new(&env, &item)?;

        Ok(self.inner.borrow_mut().unshift(slot) as u32)
    }

    /// `undefined` on an empty list, upstream's bare `return undefined;`.
    #[napi]
    pub fn shift(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow_mut().shift().into()
    }

    #[napi(js_name = "toArray")]
    pub fn to_array(&self) -> Vec<JsSlot> {
        self.inner.borrow().to_array()
    }

    /// Upstream defines `toJSON` as `toArray`.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Vec<JsSlot> {
        self.inner.borrow().to_array()
    }

    /// Upstream defines `toString` as `toArray().join(',')`.
    #[napi(js_name = "toString")]
    pub fn to_js_string(&self, env: Env) -> Result<String> {
        let items = self.inner.borrow().to_array();

        foreach::join(&env, &items)
    }

    /// Upstream's `forEach`. See the module docs and
    /// `mnemonist_core::structures::linked_list` for why this shares one
    /// walk primitive with `values`/`entries` rather than needing its own
    /// timing, unlike `lru-cache`'s `ForEachWalk`.
    // The callback type is spelled out rather than aliased so napi's macro
    // keeps recognising the `Function` shape -- see `crate::queue`'s
    // identical note.
    #[allow(clippy::type_complexity)]
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(JsSlot, u32, Object)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let list_object = this.object;
        let mut cursor = self.inner.borrow().values();
        let mut index = 0u32;

        // The borrow is taken per step, inside the closure, and dropped
        // before the callback runs -- a callback that pushes/unshifts/shifts
        // through this same list never meets an outstanding borrow, and the
        // walk sees exactly what it did, live, per the core module's own
        // liveness rules.
        let mut step = || {
            let inner = self.inner.borrow();

            cursor.step(&inner).cloned()
        };

        while let Some(item) = step() {
            let arguments = FnArgs::from((item, index, list_object));

            match &scope {
                Some(scope) => callback.apply(*scope, arguments)?,
                None => callback.apply(this, arguments)?,
            };

            index += 1;
        }

        Ok(())
    }

    /// A fresh cursor over the list's values.
    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsLinkedList>) -> Result<JsLinkedListValues> {
        let start = self.inner.borrow().values();
        let source = this.share_with(env, |list| Ok(&list.inner))?;

        Ok(JsLinkedListValues {
            cursor: CellListCursor::open(source, start),
        })
    }

    /// A fresh cursor over `[index, value]` pairs.
    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsLinkedList>) -> Result<JsLinkedListEntries> {
        let start = self.inner.borrow().values();
        let source = this.share_with(env, |list| Ok(&list.inner))?;

        Ok(JsLinkedListEntries {
            cursor: CellListCursor::open(source, start),
            index: 0,
        })
    }

    /// `LinkedList.from(iterable)` — enqueue every element of an arbitrary
    /// JS iterable, in order.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown) -> Result<Self> {
        Ok(Self {
            inner: RefCell::new(foreach::collect(&env, iterable)?.into_iter().collect()),
        })
    }
}

/// The JS half of [`ListCursor`]: core walk state, plus a live handle to a
/// JS-owned parent, in the shape `crate::map_cursor::CellMapCursor` uses for
/// `OrderedMap` — a `RefCell`-backed source a JS cursor can outlive the call
/// that produced it and still walk, re-borrowing per step.
///
/// A plain `SharedReference` (no cell) would do for `values`/`entries` alone,
/// since nothing about *reading* the list needs re-entrancy protection by
/// itself -- but `forEach` on the very same list must be able to mutate
/// through the identical borrow discipline, so all three go through the one
/// `RefCell` the list already holds.
pub struct CellListCursor<Owner: 'static, T: 'static> {
    source: SharedReference<Owner, &'static RefCell<CoreList<T>>>,
    state: ListCursor,
}

impl<Owner: 'static, T: 'static> CellListCursor<Owner, T> {
    /// Wrap a cursor already opened against the list's *current* head —
    /// upstream's `var n = this.head`, captured once, at `values()`/
    /// `entries()`/`from`'s call time, and never again re-read here.
    pub fn open(
        source: SharedReference<Owner, &'static RefCell<CoreList<T>>>,
        state: ListCursor,
    ) -> Self {
        Self { source, state }
    }

    /// One step, against the list as it is **now**.
    pub fn step(&mut self) -> Option<T>
    where
        T: Clone,
    {
        let inner = self.source.borrow();

        self.state.step(&inner).cloned()
    }
}

/// The cursor `LinkedList.prototype.values()` hands out.
#[napi(iterator, js_name = "LinkedListValues")]
pub struct JsLinkedListValues {
    cursor: CellListCursor<JsLinkedList, JsSlot>,
}

impl Generator for JsLinkedListValues {
    type Yield = JsSlot;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<JsSlot> {
        self.cursor.step()
    }

    /// A native list iterator has no `.return`, same as every other cursor in
    /// this port: `break` leaves it where it stopped, resumable by a later
    /// `next()`.
    fn complete(&mut self, _value: Option<()>) -> Option<JsSlot> {
        None
    }
}

/// The cursor `LinkedList.prototype.entries()` hands out.
#[napi(iterator, js_name = "LinkedListEntries")]
pub struct JsLinkedListEntries {
    cursor: CellListCursor<JsLinkedList, JsSlot>,
    /// `i` in upstream's `[i++, value]`, advanced only on a yielded step.
    index: u32,
}

impl Generator for JsLinkedListEntries {
    type Yield = (u32, JsSlot);
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<(u32, JsSlot)> {
        let value = self.cursor.step()?;
        let index = self.index;

        self.index += 1;

        Some((index, value))
    }

    fn complete(&mut self, _value: Option<()>) -> Option<(u32, JsSlot)> {
        None
    }
}

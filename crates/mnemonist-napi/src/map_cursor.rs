//! The JS half of the `Map` cursor contract, for tier T3.
//!
//! [`crate::cursor::BridgeCursor`] does this for `obliterator` sequences;
//! this is the same shape for [`mnemonist_core::map::OrderedMap`], and it
//! exists separately for the same reason the core types do — a `Map` iterator
//! and an `obliterator` iterator obey different rules, and one wrapper over
//! both would get one of them wrong. See `mnemonist_core::map`.
//!
//! The two halves of the problem this solves:
//!
//! 1. **A JS cursor outlives the call that made it**, and the map stays
//!    mutable underneath it — which is the whole point, because `Map`
//!    iterators are live. `&'a OrderedMap` cannot express that, so napi's
//!    [`SharedReference`] holds the JS parent alive and projects into it.
//!    The `unsafe` is napi's, not ours, and none of it reaches
//!    `mnemonist-core`.
//! 2. **Compaction.** [`mnemonist_core::map::MapCursor`] is already immune to
//!    it, which is why this type is a two-field struct and not a protocol.

use std::cell::RefCell;

use mnemonist_core::map::{MapCursor, OrderedMap};
use napi::bindgen_prelude::*;

/// A `Map` cursor over a JS-owned parent: core walk state, plus a live handle.
///
/// `Owner` is the `#[napi]` class; `K`/`V` are the core map's key and value
/// types inside it.
pub struct MapBridgeCursor<Owner: 'static, K: 'static, V: 'static> {
    source: SharedReference<Owner, &'static OrderedMap<K, V>>,
    state: MapCursor,
}

impl<Owner: 'static, K: 'static, V: 'static> MapBridgeCursor<Owner, K, V> {
    /// Open a cursor positioned before the first entry.
    ///
    /// Nothing is captured here, unlike the `obliterator` cursor, which
    /// freezes a length at construction. A `Map` cursor has nothing to freeze:
    /// it is defined entirely by where it has got to.
    pub fn open(source: SharedReference<Owner, &'static OrderedMap<K, V>>) -> Self {
        Self {
            source,
            state: MapCursor::open(),
        }
    }

    /// One step, against the map as it is **now**.
    ///
    /// `None` is `{done: true}` and is permanent.
    pub fn step(&mut self) -> Option<(&K, &V)> {
        self.state.step(*self.source)
    }
}

/// A [`MapBridgeCursor`] over a source the JS side can mutate *while a `&self`
/// method is on the stack* — the `Map` counterpart of
/// [`crate::cursor::CellCursor`], and it exists for exactly the reason
/// documented there (B-31: `&T` on a `Freeze` `T` is `noalias readonly`, so
/// LLVM may hoist reads across a re-entrant JS callback).
///
/// `S` is the owner's whole core structure and `project` picks the
/// [`OrderedMap`] out of it, because the cell has to sit at the field the
/// bridge owns rather than around the map alone.
///
/// # Why `step` takes a closure
///
/// A `Ref` cannot outlive the call that produced it, so this cursor cannot
/// hand back `(&K, &V)` the way [`MapBridgeCursor`] does. Rather than force
/// every caller to clone into an owned pair, `step` runs the caller's
/// projection *inside* the borrow and returns its result — which keeps the
/// borrow provably shorter than the callback that follows it.
pub struct CellMapCursor<Owner: 'static, S: 'static, K: 'static, V: 'static> {
    source: SharedReference<Owner, &'static RefCell<S>>,
    project: fn(&S) -> &OrderedMap<K, V>,
    state: MapCursor,
}

impl<Owner: 'static, S: 'static, K: 'static, V: 'static> CellMapCursor<Owner, S, K, V> {
    /// Open a cursor positioned before the first entry. Nothing is captured;
    /// see [`MapBridgeCursor::open`].
    pub fn open(
        source: SharedReference<Owner, &'static RefCell<S>>,
        project: fn(&S) -> &OrderedMap<K, V>,
    ) -> Self {
        Self {
            source,
            project,
            state: MapCursor::open(),
        }
    }

    /// One step, against the map as it is **now**, projected by `then`.
    ///
    /// `None` is `{done: true}` and is permanent.
    pub fn step<R>(&mut self, then: impl FnOnce(&K, &V) -> R) -> Option<R> {
        let owner = self.source.borrow();
        let (key, value) = self.state.step((self.project)(&owner))?;

        Some(then(key, value))
    }
}

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

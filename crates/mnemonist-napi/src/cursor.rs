//! The JS half of the cursor contract `docs/DIVERGENCES.md`'s iteration section, 3.6, 3.7).
//!
//! `mnemonist-core`'s [`CursorState`] is the faithful walk; this module is the
//! two things that only exist once JavaScript is in the picture.
//!
//! # 1. A cursor that outlives any borrow of its parent
//!
//! A JS cursor is an independent object. It is reachable after the call that
//! made it returns, and the collection stays mutable underneath it — that
//! aliasing is exactly what makes upstream's hybrid capture observable. Rust's
//! `&'a Parent` cannot express it, so [`BridgeCursor`] holds napi's
//! [`SharedReference`] instead: a refcounted handle that keeps the JS parent
//! alive and projects to the core structure inside it.
//!
//! This is the aliasing the crate split exists for. The reference is handed
//! out by napi's own `share_with`, whose `unsafe` lives in `napi`, and it never
//! appears in `mnemonist-core` — which keeps `#![forbid(unsafe_code)]`
//! literally true while still reproducing a behaviour that is *about*
//! aliasing.
//!
//! # 2. The factory half of the two-level `Symbol.iterator`
//!
//! Upstream writes one line per module:
//!
//! ```js
//! SparseSet.prototype[Symbol.iterator] = SparseSet.prototype.values;
//! ```
//!
//! which produces two different behaviours one level apart (DIV-STACK-2):
//!
//! | expression | behaviour |
//! |---|---|
//! | `[...set]` twice | **works twice** — the collection's `Symbol.iterator` is a *factory* |
//! | `const it = set.values(); [...it]` twice | **second is empty** — the cursor's `Symbol.iterator` is *identity* |
//!
//! napi-rs's `#[napi(iterator)]` already gives the identity half. The factory
//! half is [`install_iterator_factories`], which performs the same prototype
//! assignment from Rust, once, driven by a table — so a new module is one
//! `(class, method)` row rather than a hand-written shim.
//!
//! Doing it here rather than in `tests/bridge/*.js` is deliberate: the shims
//! are test scaffolding, and a semantic that upstream ships inside the module
//! belongs inside the addon. `require('@port/addon').SparseSet` is spreadable
//! on its own.

use mnemonist_core::cursor::{CursorState, Sequence, Step};
use mnemonist_core::structures::bits::BitWalk;
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// A cursor over a JS-owned parent: core walk state, plus a live handle.
///
/// `Owner` is the `#[napi]` class; `S` is the core structure inside it.
/// [`SharedReference`] keeps the JS object alive for as long as the cursor is
/// reachable from JS, which is what stops the walk from reading freed memory
/// when the last JS reference to the collection is dropped mid-iteration.
pub struct BridgeCursor<Owner: 'static, S: Sequence + 'static> {
    source: SharedReference<Owner, &'static S>,
    state: CursorState<S>,
}

impl<Owner: 'static, S: Sequence + 'static> BridgeCursor<Owner, S> {
    /// Freeze the source now — at `values()` time, not at first `next()`.
    ///
    /// The distinction is observable: upstream captures `l` when the iterator
    /// is *constructed*, so mutations between construction and the first
    /// `next()` are already inside the window.
    pub fn open(source: SharedReference<Owner, &'static S>) -> Self {
        let state = CursorState::open(*source);

        Self { source, state }
    }

    /// As [`open`](BridgeCursor::open), for a source offering several walks.
    ///
    /// `SparseMap` hands out three cursors over one frozen `size`; which one is
    /// a [`Sequence::Frozen`] payload rather than a separate type. See
    /// `mnemonist_core::cursor::CursorState::open_projected`.
    pub fn open_projected(source: SharedReference<Owner, &'static S>, frozen: S::Frozen) -> Self {
        let state = CursorState::open_projected(*source, frozen);

        Self { source, state }
    }

    /// One step, against the parent as it is *now*.
    pub fn step(&mut self) -> Step<S::Item> {
        self.state.step(*self.source)
    }
}

/// A [`BridgeCursor`] over a source the JS side can mutate *while a `&self`
/// method is on the stack*.
///
/// # The bug this exists to prevent, measured rather than reasoned about
///
/// napi hands the same wrapped struct to JS as `&self` for one method and
/// `&mut self` for another, and JavaScript may call the second from inside a
/// callback the first is running. rustc marks a `&T` parameter `noalias`
/// `readonly` whenever `T: Freeze`, so LLVM is entitled to hoist a read out of
/// such a loop — and it does:
///
/// ```js
/// var q = Queue.from([1, 2, 3, 4]);
/// q.forEach(function (value, i) { if (i === 0) { q.dequeue(); q.dequeue(); } });
/// ```
///
/// Upstream sees `1, 4, undefined, undefined`, because its `forEach` re-reads
/// `this.items` every iteration and the second dequeue rebinds it. A bridge
/// holding a plain `&self` saw `1, 2, 3, 4`: the load was hoisted, and the
/// mutation was invisible even though the *same* object reported the new
/// `offset` through a separate call one line later.
///
/// The fix is not a `volatile` read or a compiler barrier, it is the type: a
/// struct with a [`RefCell`](std::cell::RefCell) inline is not `Freeze`, so
/// `&self` carries neither attribute and every read is genuinely a read. The
/// bridges therefore wrap their core structure in a `RefCell`, which makes this
/// cursor's projected source a `&RefCell<S>` rather than a `&S`.
///
/// The borrow is taken per step and released immediately, so a JS callback that
/// re-enters and mutates never meets an outstanding borrow.
pub struct CellCursor<Owner: 'static, S: Sequence + 'static> {
    source: SharedReference<Owner, &'static std::cell::RefCell<S>>,
    state: CursorState<S>,
}

impl<Owner: 'static, S: Sequence + 'static> CellCursor<Owner, S> {
    /// Freeze the source now — at `values()` time, not at first `next()`.
    pub fn open(source: SharedReference<Owner, &'static std::cell::RefCell<S>>) -> Self {
        let state = CursorState::open(&*source.borrow());

        Self { source, state }
    }

    /// As [`open`](CellCursor::open), for a source offering several walks.
    ///
    /// The [`BridgeCursor::open_projected`] contract, over a `RefCell` source:
    /// `SparseMap` hands out three cursors over one frozen `size`, and which
    /// one is a [`Sequence::Frozen`] payload rather than a separate type.
    pub fn open_projected(
        source: SharedReference<Owner, &'static std::cell::RefCell<S>>,
        frozen: S::Frozen,
    ) -> Self {
        let state = CursorState::open_projected(&*source.borrow(), frozen);

        Self { source, state }
    }

    /// One step, against the parent as it is *now*.
    pub fn step(&mut self) -> Step<S::Item> {
        self.state.step(&self.source.borrow())
    }
}

/// Translate a core [`Step`] into what `Generator::next` must return.
///
/// The three-way mapping is the whole of `docs/DIVERGENCES.md`'s iteration section Option A, and it is
/// this function that answers the question §3.7 left open — whether napi can
/// express `undefined` in a `Yield` slot. It can, but **not** through
/// `Option<T>`: napi maps `Option::None` to `null`, and `null` is not
/// `undefined` to `assert.deepStrictEqual`. `Either<T, Undefined>` maps
/// `Either::B(())` to a real `undefined`, so the yield type is an `Either` and
/// the `Option` is reserved for its actual meaning here — `None` is
/// `{done: true}`.
///
/// | core | JS |
/// |---|---|
/// | [`Step::Item`] | `{done: false, value: <item>}` |
/// | [`Step::Gap`] | `{done: false, value: undefined}` |
/// | [`Step::Done`] | `{done: true}` |
pub fn yielded<T>(step: Step<T>) -> Option<Either<T, Undefined>> {
    match step {
        Step::Item(item) => Some(Either::A(item)),
        Step::Gap => Some(Either::B(())),
        Step::Done => None,
    }
}

/// A cursor over a bit store, which needs no handle to its parent.
///
/// The contrast with [`BridgeCursor`] is the point. `SparseSet.prototype.values`
/// captures only `this.size` and then reads `this.dense` on every step, so its
/// cursor must reach the live parent. `BitSet.prototype.values` captures the
/// `length`, the array **object** and its length, and never touches `this`
/// again — so the core [`BitWalk`] already owns everything it reads, and this
/// is a plain wrapper. It is also why a `clear()` is invisible to an open
/// cursor: the array it holds is the pre-clear one.
pub struct BridgeBitCursor {
    walk: BitWalk,
}

impl BridgeBitCursor {
    /// Wrap an already-frozen core walk. The walk owns everything it reads,
    /// so nothing here needs a handle on the parent structure.
    pub fn new(walk: BitWalk) -> Self {
        Self { walk }
    }

    /// One bit, or `None` for `{done: true}`.
    ///
    /// No `Either<_, Undefined>` here, unlike `SparseSet`: [`Step::Gap`] is
    /// unreachable over a frozen array that the cursor itself keeps alive, so
    /// the two-state `Option` really is the whole domain. See the `BitWalk`
    /// docs for why.
    pub fn next_bit(&mut self) -> Option<u32> {
        self.walk.step().item()
    }

    /// One `[index, bit]` pair, as a two-element JS array.
    pub fn next_entry(&mut self) -> Option<Vec<u32>> {
        self.walk
            .step_entry()
            .item()
            .map(|(index, bit)| vec![index as u32, bit])
    }
}

/// Collection classes whose `Symbol.iterator` must be their cursor factory.
///
/// One row per upstream `X.prototype[Symbol.iterator] = X.prototype.values`.
/// Kept as data rather than as code per module so the count of modules and the
/// count of places to get this wrong stay unrelated.
///
/// The aliased method is **not** always `values`, so each row records it: a
/// map-like class usually aliases `entries`, and two classes alias something
/// else again. Getting one wrong leaves `[...x]` yielding the wrong shape.
///
/// This table is append-only. It is a shared registry edited concurrently
/// from several worktrees, and appending keeps git's conflict boundaries off
/// the existing rows rather than inside the array literal.
const ITERATOR_FACTORIES: &[(&str, &str)] = &[
    ("SparseSet", "values"),
    // Note the method: upstream aliases `SparseMap.prototype[Symbol.iterator]`
    // to `entries`, not to `values`. Getting that wrong would leave `[...map]`
    // yielding bare values and every `deepStrictEqual` against pairs failing.
    ("SparseMap", "entries"),
    ("SparseQueueSet", "values"),
    ("BitSet", "values"),
    ("BitVector", "values"),
    ("Stack", "values"),
    ("Queue", "values"),
    // Not always `values`: `DefaultMap`'s last line aliases `entries` too, so a
    // spread of one yields `[key, value]` pairs.
    ("DefaultMap", "entries"),
    ("FixedStack", "values"),
    ("FixedDeque", "values"),
    ("CircularBuffer", "values"),
    ("Vector", "values"),
    ("BiMap", "entries"),
    ("BiMapInverse", "entries"),
    ("FuzzyMap", "values"),
    // All four alias `entries`, not `values` -- upstream's own last line for
    // all of `lru-cache.js`/`lru-map.js`/the two `-with-delete` files is
    // `X.prototype[Symbol.iterator] = X.prototype.entries`.
    ("LRUCache", "entries"),
    ("LRUCacheWithDelete", "entries"),
    ("LRUMap", "entries"),
    ("LRUMapWithDelete", "entries"),
    // `TrieMap`'s last line aliases `entries`; `trie.js`'s own last line
    // aliases `keys`, not `entries` -- a `Trie` has no value to pair a key
    // with.
    ("TrieMap", "entries"),
    ("Trie", "keys"),
    // `MultiMap`'s last line aliases `entries`, matching `[...map]` yielding
    // `[key, value]` pairs; `MultiSet`'s aliases `values`.
    ("MultiMap", "entries"),
    ("MultiSet", "values"),
    ("FuzzyMultiMap", "values"),
    // `InvertedIndex`'s last line aliases `documents`, not `values` -- there
    // is no bare `values()` method on it at all.
    ("LinkedList", "values"),
    ("InvertedIndex", "documents"),
    // `PassjoinIndex`'s last line aliases `values`, matching
    // `test/passjoin-index.js`'s `for (var string of index)`.
    ("PassjoinIndex", "values"),
];

/// Wire every collection's `Symbol.iterator` to its cursor factory.
///
/// Runs once, from napi's module-export hook, after the classes are on
/// `exports`. Failing loudly matters more than degrading gracefully: a missing
/// class here means the table and the exports have drifted apart, and the
/// symptom otherwise would be a spread that silently yields nothing.
#[napi(module_exports)]
pub fn install_iterator_factories(mut exports: Object, env: Env) -> Result<()> {
    // `Object::get` rather than `get_named_property`: the latter *validates*
    // the target type, and both `Symbol` and a class constructor are JS
    // functions, not plain objects. `get` skips validation and reports a
    // missing property as `None`, which is the check actually wanted here.
    let global = env.get_global()?;
    // `globalThis` is a `JsGlobal`, which has no inherent `get`; the trait's
    // `_unchecked` form is the equivalent escape from the same type validation.
    let symbol: Object = global.get_named_property_unchecked("Symbol")?;
    let iterator: Unknown = symbol
        .get("iterator")?
        .ok_or_else(|| missing("Symbol.iterator"))?;

    // The addon's only module-export hook, so the other load-time semantics
    // upstream ships inside its modules ride along here: `X.of`, and taking
    // `#.return` off the cursors. See `crate::statics`.
    //
    // BEFORE the aliasing below, not after: that loop copies whatever
    // `X.prototype.values` is at the time, so patching afterwards would leave
    // `Symbol.iterator` pointing at the unpatched factory and the two ways of
    // getting a cursor would behave differently.
    crate::statics::install_variadic_factories(&mut exports, &env)?;

    // `Vector`'s width-named subclasses and `Vector.PointerVector` have no
    // JS-representable `ArrayClass` of their own to construct with (DIV-STACK-2's
    // reasoning again: this belongs in the addon, not in test scaffolding).
    // Before the `Symbol.iterator` aliasing loop below is fine either way --
    // every subclass instance is a real `Vector` under the hood, so it picks
    // up the same prototype-level `Symbol.iterator` regardless of order.
    crate::vector::install_vector_subclasses(&exports, &env)?;

    for (class, factory) in ITERATOR_FACTORIES {
        let constructor: Object = exports
            .get(class)?
            .ok_or_else(|| missing(&format!("exports.{class}")))?;
        let mut prototype: Object = constructor
            .get("prototype")?
            .ok_or_else(|| missing(&format!("{class}.prototype")))?;
        let values: Unknown = prototype
            .get(factory)?
            .ok_or_else(|| missing(&format!("{class}.prototype.{factory}")))?;

        prototype.set_property(iterator, values)?;
    }

    Ok(())
}

fn missing(what: &str) -> Error {
    Error::new(
        Status::GenericFailure,
        format!(
            "cannot install the Symbol.iterator factory: `{what}` does not exist. \
             The ITERATOR_FACTORIES table and the addon's exports have drifted apart."
        ),
    )
}

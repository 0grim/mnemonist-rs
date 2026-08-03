//! `mnemonist-napi` is the Node.js test bridge for `mnemonist-core`, built
//! on napi-rs. It exists purely as scaffolding to exercise the FFI
//! toolchain end-to-end from the test harness; it is never a dependency of
//! `mnemonist-core`, and `mnemonist-core` must never depend on it.
//!
//! Unsafe code at this FFI boundary is expected and sanctioned -- napi-rs
//! generates `unsafe` glue to cross the Rust/JS boundary -- in contrast to
//! `mnemonist-core`, which forbids `unsafe` entirely.
//!
//! # What lives here rather than in core
//!
//! Everything that needs a JavaScript value to answer: `typeof` guards and
//! the exact `TypeError` text upstream throws for them, SameValueZero key
//! identity ([`js_key`]), `undefined` as a value distinct from absence
//! ([`js_slot`]), the `Symbol.iterator` protocol ([`iterables`]), calling a
//! caller-supplied callback ([`foreach`], [`comparators`]), and the typed
//! array constructors ([`typed_arrays`], [`array_class`]). Core takes
//! already-typed Rust values; the coercion happens here, once per module.
//!
//! # Where `missing_docs` is enforced, and where it is not
//!
//! `mnemonist-core` denies `missing_docs` crate-wide: it is the shipped
//! library, and a Rust caller reads its rustdoc. This crate is not that.
//! Most items below are `#[napi]` classes and methods whose only consumer is
//! JavaScript, whose contract is upstream `mnemonist`'s own API, and whose
//! rustdoc nobody reads — a `/// #.push` on each of 250 of them would be
//! noise standing in for documentation.
//!
//! So the lint is applied per module, to the ones with a genuine *Rust*
//! surface: the value-representation and protocol types every bridge module
//! calls into. Those are documented and the compiler keeps them that way.
//! The per-structure bridges are not, deliberately; where one reproduces an
//! upstream bug on purpose it is marked with the same `BUG-<MODULE>-<n>`
//! identifier the core module and `docs/modules/<unit>.md` use.

use napi_derive::napi;

// rustfmt keeps this list alphabetical, so there is no position to choose
// when adding a module -- which is what makes concurrent edits to it safe.
#[warn(missing_docs)]
pub mod array_class;
pub mod bi_map;
pub mod binary_search;
pub mod bit_set;
pub mod bit_vector;
pub mod bk_tree;
pub mod bloom_filter;
pub mod circular_buffer;
#[warn(missing_docs)]
pub mod comparators;
pub mod critbit_tree_map;
#[warn(missing_docs)]
pub mod cursor;
pub mod default_map;
pub mod default_weak_map;
pub mod fibonacci_heap;
pub mod fixed_critbit_tree_map;
pub mod fixed_deque;
pub mod fixed_reverse_heap;
pub mod fixed_stack;
#[warn(missing_docs)]
pub mod foreach;
pub mod fuzzy_map;
pub mod fuzzy_multi_map;
pub mod hash_tables;
pub mod hashed_array_tree;
pub mod heap;
pub mod inverted_index;
#[warn(missing_docs)]
pub mod iterables;
#[warn(missing_docs)]
pub mod js_array;
#[warn(missing_docs)]
pub mod js_key;
#[warn(missing_docs)]
pub mod js_slot;
#[warn(missing_docs)]
pub mod js_value;
pub mod kd_tree;
pub mod linked_list;
pub mod lru_cache;
pub mod lru_cache_with_delete;
pub mod lru_map;
pub mod lru_map_with_delete;
#[warn(missing_docs)]
pub mod map_cursor;
pub mod merge;
pub mod multi_array;
pub mod multi_map;
pub mod multi_set;
pub mod passjoin_index;
pub mod queue;
pub mod set;
pub mod sort;
pub mod sparse_map;
pub mod sparse_queue_set;
pub mod sparse_set;
pub mod stack;
pub mod static_disjoint_set;
pub mod static_interval_tree;
pub mod statics;
pub mod suffix_array;
pub mod symspell;
pub mod trie;
pub mod trie_map;
pub mod typed_arrays;
pub mod vector;
pub mod vp_tree;

/// A liveness check for the addon: returns `"pong"`.
///
/// The test harness calls this first, so a failure here means the native
/// module did not load at all rather than that some structure is wrong.
#[napi]
pub fn ping() -> &'static str {
    "pong"
}

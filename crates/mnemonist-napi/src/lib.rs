//! `mnemonist-napi` is the Node.js test bridge for `mnemonist-core`, built
//! on napi-rs. It exists purely as scaffolding to exercise the FFI
//! toolchain end-to-end from the test harness; it is never a dependency of
//! `mnemonist-core`, and `mnemonist-core` must never depend on it.
//!
//! Unsafe code at this FFI boundary is expected and sanctioned -- napi-rs
//! generates `unsafe` glue to cross the Rust/JS boundary -- in contrast to
//! `mnemonist-core`, which forbids `unsafe` entirely.

use napi_derive::napi;

pub mod bit_set;
pub mod bit_vector;
pub mod cursor;
pub mod foreach;
pub mod hashed_array_tree;
pub mod js_slot;
pub mod queue;
pub mod sparse_map;
pub mod sparse_queue_set;
pub mod sparse_set;
pub mod stack;
pub mod static_disjoint_set;
pub mod statics;

#[napi]
pub fn ping() -> &'static str {
    "pong"
}

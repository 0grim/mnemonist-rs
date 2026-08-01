#![forbid(unsafe_code)]

//! `mnemonist-core` is the Rust port of the `mnemonist` JavaScript data
//! structures library. This crate is the port proper: it has zero
//! JavaScript/Node awareness, pulls in no dependencies, and must build and
//! run correctly with no Node/JS runtime present at all. Node-facing
//! bindings live entirely in the separate `mnemonist-napi` crate; this
//! crate never depends on it and never will.

pub mod cursor;
pub mod map;
pub mod structures;
pub mod utils;

#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! `mnemonist-core` is the Rust port of the `mnemonist` JavaScript data
//! structures library. This crate is the port proper: it has zero
//! JavaScript/Node awareness, pulls in no dependencies, and must build and
//! run correctly with no Node/JS runtime present at all. Node-facing
//! bindings live entirely in the separate `mnemonist-napi` crate; this
//! crate never depends on it and never will.
//!
//! # Reading the docs on these types
//!
//! The port is bug-for-bug faithful. Where upstream `mnemonist` does
//! something surprising — a counter that goes stale, a bounds check that
//! admits one index too many, an iterator that reads past the end of a
//! shrinking source — this crate reproduces it and the doc comment says so,
//! naming the behaviour with a `BUG-<MODULE>-<n>` identifier. Those are not
//! defects in the port; correcting one would be. Where a JavaScript
//! behaviour has no Rust counterpart at all, the doc names a
//! `DIV-<MODULE>-<n>` divergence. Both are written up per module in
//! `docs/modules/<unit>.md`, and `docs/BUGS.md` collects the upstream bugs.
//!
//! Method docs quote the upstream member they port — `#.size`,
//! `Map.prototype.set` — so a reader who knows the JavaScript API can find
//! the Rust name for it.

pub mod cursor;
pub mod map;
pub mod structures;
pub mod utils;
// New modules are appended rather than filed alphabetically: this list is
// edited concurrently from several worktrees, and appending keeps git's
// conflict boundaries off the existing entries.
pub mod sort;

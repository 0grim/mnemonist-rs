//! Ports of the `utils/` helpers.

pub mod bitwise;
pub mod typed_arrays;

// Appended, never interleaved: this list is a shared registry and every
// reordering becomes another agent's merge conflict (CLAUDE.md, Git).
pub mod binary_search;

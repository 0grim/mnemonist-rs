//! Ports of the `utils/` helpers.

pub mod bitwise;
pub mod comparators;
pub mod typed_arrays;

// Appended, never interleaved: this list is a shared registry and every
// reordering becomes another agent's merge conflict (CLAUDE.md, Git).
pub mod binary_search;
pub mod hash_tables;
pub mod murmurhash3;
// Appended at the end, never inserted: this file is edited by several agents
// at once and a new line anywhere else is a merge conflict (CLAUDE.md, Git).
pub mod merge;

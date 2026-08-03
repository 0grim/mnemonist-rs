//! Ports of the `utils/` helpers.

pub mod bitwise;
pub mod comparators;
pub mod typed_arrays;

// This list is append-only. It is a shared registry edited concurrently from
// several worktrees, and appending keeps git's conflict boundaries off the
// existing entries; reordering or inserting puts one in the middle of them.
pub mod binary_search;
pub mod hash_tables;
pub mod merge;
pub mod murmurhash3;

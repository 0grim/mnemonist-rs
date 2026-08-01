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
pub mod default_map;
pub mod foreach;
pub mod hashed_array_tree;
pub mod js_key;
pub mod js_slot;
pub mod js_value;
pub mod map_cursor;
pub mod queue;
pub mod sparse_map;
pub mod sparse_queue_set;
pub mod sparse_set;
pub mod stack;
pub mod static_disjoint_set;
pub mod statics;
// Appended, not merged into the alphabetical run above: this file is edited by
// several agents at once and a new line at the end can never land inside
// another one's hunk.
pub mod array_class;
pub mod circular_buffer;
pub mod fixed_deque;
pub mod fixed_stack;
pub mod iterables;
// Appended rather than filed alphabetically: this list is edited from several
// worktrees at once, and a conflict boundary that lands inside it has already
// broken three merges. New modules go on the end.
pub mod set;
pub mod sort;

// Appended, never interleaved: this list is a shared registry (CLAUDE.md, Git).
pub mod bloom_filter;
pub mod suffix_array;
// Appended at the end, never inserted: this file is edited by several agents
// at once and a new line anywhere but the end can land inside another one's
// hunk (CLAUDE.md, Git).
pub mod static_interval_tree;
pub mod vector;
// at once and a new line at the end can never land inside another one's hunk.
pub mod lru_cache;
pub mod lru_cache_with_delete;
pub mod lru_map;
pub mod lru_map_with_delete;

// Appended at the end of the list rather than in alphabetical position: this
// file is shared, and a conflict boundary landing mid-list has broken three
// merges already.
pub mod comparators;
pub mod fixed_reverse_heap;
pub mod heap;
pub mod js_array;

// Appended at the end, never inserted: this file is edited by several agents
// at once and a new line anywhere else is a merge conflict.
pub mod bi_map;
pub mod bk_tree;
pub mod fuzzy_map;

// Appended at the end, never inserted: this file is a shared registry edited
// by several agents at once (CLAUDE.md, Git).
pub mod binary_search;
pub mod hash_tables;
pub mod merge;
pub mod typed_arrays;
// Appended at the end, never inserted: this file is a shared registry
// (CLAUDE.md, Git) and a new line anywhere else is a merge conflict.
pub mod trie;
pub mod trie_map;
// Appended at the end, never inserted (CLAUDE.md, Git): a line anywhere else
// is a merge conflict inside this shared registry.
pub mod multi_map;
pub mod multi_set;

// Appended at the end, never inserted (CLAUDE.md, Git).
pub mod fuzzy_multi_map;

// Appended at the end, never inserted: this file is a shared registry
// (CLAUDE.md, Git) and a new line anywhere else is a merge conflict.
pub mod fibonacci_heap;

// Appended at the end, never inserted: this file is a shared registry
// (CLAUDE.md, Git) and a new line anywhere else is a merge conflict.
pub mod default_weak_map;
pub mod inverted_index;
pub mod linked_list;

#[napi]
pub fn ping() -> &'static str {
    "pong"
}

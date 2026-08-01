//! Ports of the mnemonist data structures.

pub mod bit_set;
pub mod bit_vector;
pub mod bits;
pub mod default_map;
pub mod hashed_array_tree;
pub mod queue;
pub mod sparse_map;
pub mod sparse_queue_set;
pub mod sparse_set;
pub mod stack;
pub mod static_disjoint_set;
// Appended, not merged into the alphabetical run above: this file is edited by
// several agents at once and a new line at the end can never land inside
// another one's hunk.
pub mod backing;
pub mod circular_buffer;
pub mod fixed_deque;
pub mod fixed_stack;
// Appended rather than filed alphabetically: this list is edited from several
// worktrees at once, and a conflict boundary that lands inside it has already
// broken three merges. New modules go on the end.
pub mod set;

// Appended, never interleaved: this list is a shared registry (CLAUDE.md, Git).
pub mod bloom_filter;
pub mod suffix_array;
// Appended at the end, never inserted: this file is edited by several agents
// at once and a new line anywhere but the end can land inside another one's
// hunk (CLAUDE.md, Git).
pub mod static_interval_tree;
pub mod vector;

// Appended at the end, never inserted: this file is edited by several agents
// at once and a new line anywhere else is a merge conflict.
pub mod bi_map;
pub mod bk_tree;
pub mod fuzzy_map;
// at once and a new line at the end can never land inside another one's hunk.
pub mod lru_cache;

// Appended at the end of the list rather than in alphabetical position: this
// file is shared, and three merges have broken on a conflict boundary landing
// mid-list.
pub mod fixed_reverse_heap;
pub mod heap;

// Appended at the end, never inserted: this file is a shared registry
// (CLAUDE.md, Git) and a new line anywhere else is a merge conflict.
pub mod trie;
pub mod trie_map;
// Appended at the end, never inserted: this file is edited by several agents
// at once and a new line anywhere else is a merge conflict (CLAUDE.md, Git).
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

// Appended at the end, never inserted: this file is a shared registry
// (CLAUDE.md, Git) and a new line anywhere else is a merge conflict.
pub mod critbit_tree_map;
pub mod fixed_critbit_tree_map;

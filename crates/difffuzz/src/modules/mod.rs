//! One [`ModuleSpec`](crate::ModuleSpec) per fuzzable module.
//!
//! Adding a module means adding a file here. The oracle, the driver and the
//! shrinking all stay untouched — that is the whole point of the generic
//! harness (P3: machinery before modules).

pub mod bit_set;
pub mod bit_vector;
pub mod default_map;
pub mod hashed_array_tree;
pub mod queue;
pub mod sparse_map;
pub mod sparse_queue_set;
pub mod sparse_set;
pub mod stack;
pub mod static_disjoint_set;
// Appended at the end, never inserted -- this file is edited by several agents
// at once.
pub mod circular_buffer;
pub mod fixed_deque;
pub mod fixed_stack;
// Appended rather than filed alphabetically: this list is edited from several
// worktrees at once, and a conflict boundary that lands inside it has already
// broken three merges. New modules go on the end.
pub mod set;
pub mod sort;

// Appended, never interleaved: this list is a shared registry (CLAUDE.md, Git).
// `suffix_array` declares two specs, `SuffixArraySpec` and
// `GeneralizedSuffixArraySpec`, because they are two exports of one upstream
// file; see that module's docs.
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

// Appended at the end of the list rather than in alphabetical position: this
// file is shared, and a conflict boundary landing mid-list has broken three
// merges already.
pub mod fixed_reverse_heap;
pub mod heap;

// Appended at the end, never inserted: this file is edited by several agents
// at once and a new line at the end can never land inside another one's hunk.
pub mod lru_cache;

// Appended at the end, never inserted: this file is a shared registry edited
// by several agents at once (CLAUDE.md, Git).
#[path = "_utils.rs"]
pub mod utils_unit;
// Appended at the end, never inserted: this file is a shared registry
// (CLAUDE.md, Git) and a new line anywhere else is a merge conflict.
// `trie` shares `trie_map`'s prefix pool and tokenisation -- see that
// module's docs -- so `trie_map` is listed first.
pub mod trie;
pub mod trie_map;
// Appended at the end, never inserted (CLAUDE.md, Git): a new line anywhere
// else is a merge conflict.
pub mod fuzzy_multi_map;
pub mod multi_map;
pub mod multi_set;

//! One [`ModuleSpec`](crate::ModuleSpec) per fuzzable module.
//!
//! Adding a module means adding a file here. The oracle, the driver and the
//! shrinking all stay untouched — that is the whole point of the generic
//! harness (P3: machinery before modules).
//!
//! Two entries are not one file to one spec. `suffix_array` declares *two*
//! specs, `SuffixArraySpec` and `GeneralizedSuffixArraySpec`, because they
//! are two exports of one upstream file. And `_utils.rs` is mounted as
//! `utils_unit`, because `utils` would collide with the crate's own module
//! of that name.

pub mod bi_map;
pub mod bit_set;
pub mod bit_vector;
pub mod bk_tree;
pub mod bloom_filter;
pub mod circular_buffer;
pub mod critbit_tree_map;
pub mod default_map;
pub mod default_weak_map;
pub mod fibonacci_heap;
pub mod fixed_critbit_tree_map;
pub mod fixed_deque;
pub mod fixed_reverse_heap;
pub mod fixed_stack;
pub mod fuzzy_map;
pub mod fuzzy_multi_map;
pub mod hashed_array_tree;
pub mod heap;
pub mod inverted_index;
pub mod kd_tree;
pub mod linked_list;
pub mod lru_cache;
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
pub mod suffix_array;
pub mod symspell;
pub mod trie;
pub mod trie_map;
pub mod vector;
pub mod vp_tree;

#[path = "_utils.rs"]
pub mod utils_unit;

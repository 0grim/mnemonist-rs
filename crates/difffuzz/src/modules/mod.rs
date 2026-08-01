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

// Appended, never interleaved: this list is a shared registry (CLAUDE.md, Git).
// `suffix_array` declares two specs, `SuffixArraySpec` and
// `GeneralizedSuffixArraySpec`, because they are two exports of one upstream
// file; see that module's docs.
pub mod suffix_array;

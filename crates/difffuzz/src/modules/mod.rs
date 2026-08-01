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
pub mod fixed_stack;

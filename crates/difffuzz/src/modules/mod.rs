//! One [`ModuleSpec`](crate::ModuleSpec) per fuzzable module.
//!
//! Adding a module means adding a file here. The oracle, the driver and the
//! shrinking all stay untouched — that is the whole point of the generic
//! harness (P3: machinery before modules).

pub mod queue;
pub mod sparse_set;
pub mod stack;
pub mod static_disjoint_set;

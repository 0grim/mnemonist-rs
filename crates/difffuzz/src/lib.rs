#![forbid(unsafe_code)]

//! Generic op-sequence differential fuzzer (DESIGN.md 4).
//!
//! ```text
//! seed -> proptest generates an op sequence -> apply to mnemonist-core (in process)
//!                                           -> apply to upstream JS (one node subprocess)
//!                                           -> compare observable state after EVERY op
//! ```
//!
//! Three design points carry the weight, and all three are in the spec rather
//! than invented here:
//!
//! * **One Node process for the whole campaign** ([`Oracle`]). Line-delimited
//!   JSON over a pipe. Spawning per op is the failure mode DESIGN.md 4 names
//!   explicitly — it would turn a 60-second run into an hour.
//! * **proptest owns generation and shrinking** ([`Campaign`]). A raw
//!   divergence is a several-hundred-op program; the useful artifact is the
//!   three-op version proptest shrinks it to.
//! * **Per-module declaration only** ([`ModuleSpec`]). A module contributes an
//!   op alphabet, arg strategies and an observable-state list. Nothing else in
//!   the crate knows what a disjoint set is.
//!
//! Divergences are kept strictly separate from harness failures: a dead oracle
//! returns `Err`, never "zero divergences".

pub mod campaign;
pub mod modules;
pub mod oracle;
pub mod spec;

pub use campaign::{run, run_with, Campaign, Report};
pub use oracle::{Observation, Oracle, OracleError};
pub use spec::{check_program, CheckFailure, Divergence, DivergenceKind, ModuleSpec, Op, Program};

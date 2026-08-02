//! The module registry `main.rs` dispatches through.
//!
//! Before this file, `main.rs` matched on the module name string twice — once
//! for the timed workload, once for `--structure` — with a wildcard arm that
//! had already produced a real incident (see `main.rs`'s own comment on the
//! subject). Adding module 9 through 45 now means: implement `run_mixed` (and
//! `run_drain`, if the module offers one) plus `build_structure` in a new
//! file, then add one line to [`MODULES`] below. `main.rs` itself does not
//! change.
//!
//! Deliberately *not* a trait object. Every module here is a handful of free
//! functions with no state to hold between calls, so a table of function
//! pointers is the whole abstraction `bench/drive.js` needs on this side, and
//! it costs nothing extra at the call site: dispatch happens once per
//! warmup/measured *pass* (a handful of times per process), never inside the
//! per-op loop that is actually timed, so this refactor cannot move the
//! numbers for `sparse-set`/`static-disjoint-set` — their loop bodies are
//! untouched, just relocated one level for `build_structure`.

use crate::workload::Workload;

/// One measured pass over the `mixed` workload: per-batch nanoseconds and a
/// checksum over every non-mutating op's results.
pub type MixedFn = fn(&Workload, usize) -> (Vec<u64>, u64);

/// One measured pass over a `drain`-style walk: per-batch nanoseconds, a
/// checksum, and members yielded per walk (sparse-set's existing convention,
/// generalised here so `main.rs` needs only one drain loop, not a
/// module-specific one).
pub type DrainFn = fn(u32, u32, usize) -> (Vec<u64>, u64, usize);

/// `--structure`: build the structure at `size`, touch it so nothing is
/// elided, and stop. No return value — the only thing this measures is the
/// process's own peak RSS afterwards.
pub type StructureFn = fn(u32);

pub struct ModuleEntry {
    pub name: &'static str,
    pub kinds: &'static [&'static str],
    pub mixed: Option<MixedFn>,
    pub drain: Option<DrainFn>,
    pub structure: StructureFn,
}

pub const MODULES: &[ModuleEntry] = &[
    ModuleEntry {
        name: "static-disjoint-set",
        kinds: &["mixed"],
        mixed: Some(crate::static_disjoint_set::run_once),
        drain: None,
        structure: crate::static_disjoint_set::build_structure,
    },
    ModuleEntry {
        name: "sparse-set",
        kinds: &["mixed", "drain"],
        mixed: Some(crate::sparse_set::run_mixed),
        drain: Some(crate::sparse_set::run_drain),
        structure: crate::sparse_set::build_structure,
    },
    ModuleEntry {
        name: "bit-set",
        kinds: &["mixed"],
        mixed: Some(crate::bit_set::run_mixed),
        drain: None,
        structure: crate::bit_set::build_structure,
    },
    ModuleEntry {
        name: "lru-cache",
        kinds: &["mixed"],
        mixed: Some(crate::lru_cache::run_mixed),
        drain: None,
        structure: crate::lru_cache::build_structure,
    },
    ModuleEntry {
        name: "heap",
        kinds: &["mixed"],
        mixed: Some(crate::heap::run_mixed),
        drain: None,
        structure: crate::heap::build_structure,
    },
    ModuleEntry {
        name: "trie",
        kinds: &["mixed"],
        mixed: Some(crate::trie::run_mixed),
        drain: None,
        structure: crate::trie::build_structure,
    },
    ModuleEntry {
        name: "vector",
        kinds: &["mixed"],
        mixed: Some(crate::vector::run_mixed),
        drain: None,
        structure: crate::vector::build_structure,
    },
];

pub fn find(name: &str) -> Option<&'static ModuleEntry> {
    MODULES.iter().find(|entry| entry.name == name)
}

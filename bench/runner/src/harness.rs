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
    // Appended for the sequence-backed batch, never inserted: adding a module
    // is "one line in harness.rs" per this file's own docs, and a conflict
    // boundary landing mid-array is exactly the split-block risk CLAUDE.md
    // warns about. `main.rs` does not change for any of these eleven.
    ModuleEntry {
        name: "stack",
        kinds: &["mixed"],
        mixed: Some(crate::stack::run_mixed),
        drain: None,
        structure: crate::stack::build_structure,
    },
    ModuleEntry {
        name: "queue",
        kinds: &["mixed"],
        mixed: Some(crate::queue::run_mixed),
        drain: None,
        structure: crate::queue::build_structure,
    },
    ModuleEntry {
        name: "fixed-stack",
        kinds: &["mixed"],
        mixed: Some(crate::fixed_stack::run_mixed),
        drain: None,
        structure: crate::fixed_stack::build_structure,
    },
    ModuleEntry {
        name: "fixed-deque",
        kinds: &["mixed"],
        mixed: Some(crate::fixed_deque::run_mixed),
        drain: None,
        structure: crate::fixed_deque::build_structure,
    },
    ModuleEntry {
        name: "circular-buffer",
        kinds: &["mixed"],
        mixed: Some(crate::circular_buffer::run_mixed),
        drain: None,
        structure: crate::circular_buffer::build_structure,
    },
    ModuleEntry {
        name: "hashed-array-tree",
        kinds: &["mixed"],
        mixed: Some(crate::hashed_array_tree::run_mixed),
        drain: None,
        structure: crate::hashed_array_tree::build_structure,
    },
    ModuleEntry {
        name: "sparse-map",
        kinds: &["mixed"],
        mixed: Some(crate::sparse_map::run_mixed),
        drain: None,
        structure: crate::sparse_map::build_structure,
    },
    ModuleEntry {
        name: "sparse-queue-set",
        kinds: &["mixed"],
        mixed: Some(crate::sparse_queue_set::run_mixed),
        drain: None,
        structure: crate::sparse_queue_set::build_structure,
    },
    ModuleEntry {
        name: "bit-vector",
        kinds: &["mixed"],
        mixed: Some(crate::bit_vector::run_mixed),
        drain: None,
        structure: crate::bit_vector::build_structure,
    },
    // `suffix-array` and `sort` have no per-element op stream (see each
    // file's own module docs), so both reuse the `drain` kind rather than
    // `mixed` -- one measured sample per construction/sort, exactly the
    // convention `sparse-set`'s iteration walk already established.
    ModuleEntry {
        name: "suffix-array",
        kinds: &["drain"],
        mixed: None,
        drain: Some(crate::suffix_array::run_drain),
        structure: crate::suffix_array::build_structure,
    },
    ModuleEntry {
        name: "sort",
        kinds: &["drain"],
        mixed: None,
        drain: Some(crate::sort::run_drain),
        structure: crate::sort::build_structure,
    },
    // Appended for the map-like/multi-container Gate 10 batch, never
    // inserted: same reasoning as the eleven-module batch above. `main.rs`
    // does not change for any of these nine.
    ModuleEntry {
        name: "default-map",
        kinds: &["mixed"],
        mixed: Some(crate::default_map::run_mixed),
        drain: None,
        structure: crate::default_map::build_structure,
    },
    ModuleEntry {
        name: "bi-map",
        kinds: &["mixed"],
        mixed: Some(crate::bi_map::run_mixed),
        drain: None,
        structure: crate::bi_map::build_structure,
    },
    ModuleEntry {
        name: "multi-map",
        kinds: &["mixed"],
        mixed: Some(crate::multi_map::run_mixed),
        drain: None,
        structure: crate::multi_map::build_structure,
    },
    ModuleEntry {
        name: "multi-set",
        kinds: &["mixed"],
        mixed: Some(crate::multi_set::run_mixed),
        drain: None,
        structure: crate::multi_set::build_structure,
    },
    ModuleEntry {
        name: "multi-array",
        kinds: &["mixed"],
        mixed: Some(crate::multi_array::run_mixed),
        drain: None,
        structure: crate::multi_array::build_structure,
    },
    ModuleEntry {
        name: "fuzzy-map",
        kinds: &["mixed"],
        mixed: Some(crate::fuzzy_map::run_mixed),
        drain: None,
        structure: crate::fuzzy_map::build_structure,
    },
    ModuleEntry {
        name: "fuzzy-multi-map",
        kinds: &["mixed"],
        mixed: Some(crate::fuzzy_multi_map::run_mixed),
        drain: None,
        structure: crate::fuzzy_multi_map::build_structure,
    },
    ModuleEntry {
        name: "inverted-index",
        kinds: &["mixed"],
        mixed: Some(crate::inverted_index::run_mixed),
        drain: None,
        structure: crate::inverted_index::build_structure,
    },
    // `set` has no per-element op stream (see `set_ops.rs`'s own module
    // docs), so it reuses the `drain` kind -- same convention `sort`/
    // `suffix-array` already established above.
    ModuleEntry {
        name: "set",
        kinds: &["drain"],
        mixed: None,
        drain: Some(crate::set_ops::run_drain),
        structure: crate::set_ops::build_structure,
    },
    // Appended for the final Gate 10 batch (the last fourteen units), never
    // inserted -- same reasoning as the two blocks above.
    ModuleEntry {
        name: "trie-map",
        kinds: &["mixed"],
        mixed: Some(crate::trie_map::run_mixed),
        drain: None,
        structure: crate::trie_map::build_structure,
    },
    ModuleEntry {
        name: "critbit-tree-map",
        kinds: &["mixed"],
        mixed: Some(crate::critbit_tree_map::run_mixed),
        drain: None,
        structure: crate::critbit_tree_map::build_structure,
    },
    ModuleEntry {
        name: "fixed-critbit-tree-map",
        kinds: &["mixed"],
        mixed: Some(crate::fixed_critbit_tree_map::run_mixed),
        drain: None,
        structure: crate::fixed_critbit_tree_map::build_structure,
    },
    ModuleEntry {
        name: "bk-tree",
        kinds: &["mixed"],
        mixed: Some(crate::bk_tree::run_mixed),
        drain: None,
        structure: crate::bk_tree::build_structure,
    },
    ModuleEntry {
        name: "vp-tree",
        kinds: &["mixed"],
        mixed: Some(crate::vp_tree::run_mixed),
        drain: None,
        structure: crate::vp_tree::build_structure,
    },
    ModuleEntry {
        name: "kd-tree",
        kinds: &["mixed"],
        mixed: Some(crate::kd_tree::run_mixed),
        drain: None,
        structure: crate::kd_tree::build_structure,
    },
    ModuleEntry {
        name: "static-interval-tree",
        kinds: &["mixed"],
        mixed: Some(crate::static_interval_tree::run_mixed),
        drain: None,
        structure: crate::static_interval_tree::build_structure,
    },
    ModuleEntry {
        name: "fibonacci-heap",
        kinds: &["mixed"],
        mixed: Some(crate::fibonacci_heap::run_mixed),
        drain: None,
        structure: crate::fibonacci_heap::build_structure,
    },
    ModuleEntry {
        name: "fixed-reverse-heap",
        kinds: &["mixed"],
        mixed: Some(crate::fixed_reverse_heap::run_mixed),
        drain: None,
        structure: crate::fixed_reverse_heap::build_structure,
    },
];

pub fn find(name: &str) -> Option<&'static ModuleEntry> {
    MODULES.iter().find(|entry| entry.name == name)
}

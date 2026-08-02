//! The `heap` mixed workload — a binary min-heap over `f64`, using upstream's
//! default numeric comparator exactly (`a < b` / `a > b`, ported verbatim as
//! `utils::comparators::DefaultComparator`).
//!
//! Picked because the comparator is where the port pays a bridge cost
//! invisible on the other four modules: every `Heap` method here borrows a
//! `RefCell<VecStore<f64>>` and calls through the `Comparator` trait once per
//! comparison — because upstream's comparator is arbitrary, re-entrant JS
//! that can mutate the very heap it is comparing (see `mnemonist-core`'s
//! `heap` module docs) — where V8's JIT inlines a numeric closure directly
//! into the sift loop. This benchmark does not exercise the re-entrant path
//! (that belongs to the differential fuzzer); it measures what the
//! non-re-entrant common case costs to keep the door open for it.
//!
//! Op mix, from the shared `kind % 4` stream: 50% `push` (mutating, no
//! checksum contribution — matches `union`/`add` elsewhere), 25% `pop`
//! (mutating *and* a read, contributing the popped value: the same shape as
//! `find`'s path compression), 25% `peek` (pure read). `pop`/`peek` on an
//! empty heap return upstream's `undefined` / core's `None`, both
//! contributing 0 — no guard needed, because 50% `push` against 25% `pop`
//! means the heap is only briefly empty at the very start of a run.
//!
//! `workload.size` bounds the numeric range pushed values are drawn from. A
//! range much larger than the batch size keeps most pushed values distinct,
//! so the comparator is actually exercised on non-equal pairs most of the
//! time — a heap of all-tied keys would satisfy its invariant without moving
//! anything.

use mnemonist_core::structures::heap::{Heap, VecStore};
use mnemonist_core::utils::comparators::DefaultComparator;

use crate::workload::Workload;

/// One measured pass: fresh heap, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let heap = Heap::new(VecStore::new(), DefaultComparator);

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let value = f64::from(workload.a[i]);

            match workload.kind[i] {
                0 | 1 => {
                    heap.push(Some(value)).expect("VecStore never fails");
                }
                2 => {
                    let popped = heap.pop().expect("VecStore never fails");
                    checksum += popped.map(|v| v as u64).unwrap_or(0);
                }
                _ => {
                    let peeked = heap.peek().expect("VecStore never fails");
                    checksum += peeked.map(|v| v as u64).unwrap_or(0);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&heap);

    (batches, checksum)
}

/// `--structure`: unlike `bit-set`/`lru-cache`/`sparse-set`/
/// `static-disjoint-set`, a `Heap` has no separate "capacity" from "occupied
/// size" — it grows under `Vec::push` like `Vector` and `Trie` do. So `size`
/// here means "prefilled to `size` elements", the natural equivalent for a
/// growable structure, isolating the same thing (the structure's own
/// footprint, without the ~9 MB of op arrays) by a different mechanism.
pub fn build_structure(size: u32) {
    let heap = Heap::new(VecStore::new(), DefaultComparator);

    for i in 0..size {
        heap.push(Some(f64::from(i))).expect("VecStore never fails");
    }

    std::hint::black_box(&heap);
}

/// A bare binary min-heap over `Vec<f64>`: no `RefCell`, no `Cell`, no
/// `Store`/`Comparator` trait, `<` inlined directly at the call site — the
/// counterfactual `run_mixed`'s own module docs describe as "what V8 inlines
/// directly into the sift loop". Same algorithm shape as
/// `mnemonist_core::structures::heap`'s free `push`/`pop` functions (sift-up
/// on insert, swap-pop-sift-down on extract), but every layer that module's
/// bench doc names as a hypothesis — the `RefCell<VecStore<f64>>` borrow, the
/// `Rc` clone, the `Comparator` trait call — is structurally absent here
/// rather than merely fast.
///
/// Checksums are invariant to which of two numerically-equal elements a tie
/// break returns (both sides sum the *value*, not an identity), so this does
/// not need to reproduce `heap.rs`'s own right-child-on-ties rule to agree
/// with [`run_mixed`]'s checksum on the same workload — and at this
/// workload's ~500,000 pushes into a 1,000,000-value domain, ties are
/// frequent enough (birthday-paradox collisions) that this is worth stating
/// rather than assuming.
struct BareHeap {
    items: Vec<f64>,
}

impl BareHeap {
    fn new() -> Self {
        Self { items: Vec::new() }
    }

    fn push(&mut self, value: f64) {
        self.items.push(value);
        let mut i = self.items.len() - 1;

        while i > 0 {
            let parent = (i - 1) / 2;

            if self.items[i] < self.items[parent] {
                self.items.swap(i, parent);
                i = parent;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<f64> {
        self.items.first().copied()
    }

    fn pop(&mut self) -> Option<f64> {
        let last = self.items.pop()?;

        if self.items.is_empty() {
            return Some(last);
        }

        let root = std::mem::replace(&mut self.items[0], last);
        let mut i = 0;
        let n = self.items.len();

        loop {
            let l = 2 * i + 1;
            let r = 2 * i + 2;
            let mut smallest = i;

            if l < n && self.items[l] < self.items[smallest] {
                smallest = l;
            }

            if r < n && self.items[r] < self.items[smallest] {
                smallest = r;
            }

            if smallest == i {
                break;
            }

            self.items.swap(i, smallest);
            i = smallest;
        }

        Some(root)
    }
}

/// The bare counterpart to [`run_mixed`], same op mix and same op stream —
/// see `main.rs`'s `--heap-probe` for the comparison this exists for. Not
/// part of `harness::MODULES`: there is no upstream JS analogue of "a bare
/// Rust `Vec`-backed heap" to publish a `regressions` array against, same
/// reasoning as `sparse_set.rs::run_mixed_refcell`.
pub fn run_mixed_bare(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut heap = BareHeap::new();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let value = f64::from(workload.a[i]);

            match workload.kind[i] {
                0 | 1 => heap.push(value),
                2 => checksum += heap.pop().map(|v| v as u64).unwrap_or(0),
                _ => checksum += heap.peek().map(|v| v as u64).unwrap_or(0),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&heap);

    (batches, checksum)
}

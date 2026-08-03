//! The `sparse-set` timed loops. Three of them, measuring different things.
//!
//! * [`run_mixed`] is the membership workload — `add`/`has`/`delete` — and is
//!   the direct analogue of the `static-disjoint-set` loop. This is what
//!   `bench/results.json`'s `sparse-set` entry measures, and it is the port
//!   linked directly against `mnemonist-core` — the RefCell described below
//!   is not in this loop at all.
//! * [`run_drain`] measures **iteration**, which is the whole reason this
//!   module was ported now. It is the only benchmark in the repo that puts the
//!   cursor machinery of DESIGN.md 3.4 on the clock, against the JS closure it
//!   was ported from. A cursor that reached the parent through a trait call
//!   per element would show up here and nowhere else.
//! * [`run_mixed_refcell`] is not part of the gate-10 protocol above and is
//!   never written to `bench/results.json` or `tests/scope.txt` — see its own
//!   docs for what it measures and why it exists as a separate, out-of-band
//!   probe rather than a thirteenth registered module.

use std::cell::RefCell;

use mnemonist_core::structures::sparse_set::SparseSet;

use crate::workload::{Workload, ADD_A, ADD_B, HAS};
use crate::xorshift::XorShift32;

/// Membership workload: 50% `add`, 25% `has`, 25% `delete`.
///
/// Members are drawn in range, so this measures the structure rather than
/// upstream's out-of-range corruption path. That path is exhaustively covered
/// by the differential fuzzer, where it belongs; a benchmark of it would
/// measure a bug rather than a data structure.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    // Fresh set per pass, outside the timed region: a set left full by pass 1
    // would make pass 2 mostly duplicate-`add` early returns.
    let mut set =
        SparseSet::new(workload.size as usize).expect("benchmark sizes are inside the limit");

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let member = workload.a[i] as usize;

            match workload.kind[i] {
                // `add` mutates and cannot be elided, so like `union` on the
                // other module it contributes nothing to the checksum.
                ADD_A | ADD_B => {
                    set.add(member);
                }
                HAS => checksum += u64::from(set.has(member)),
                _ => checksum += u64::from(set.delete(member)),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&set);

    (batches, checksum)
}

/// How many members a `passes`-pass drain will yield, and the set to drain.
///
/// Prefill is `size` random `add`s over `0..size`, so the set ends up with the
/// expected `size * (1 - 1/e)` ≈ 63% distinct members. Both sides draw from
/// the same PRNG and therefore build the identical set; the checksum proves
/// it rather than assuming it.
fn prefilled(size: u32, seed: u32) -> SparseSet {
    let mut set = SparseSet::new(size as usize).expect("benchmark sizes are inside the limit");
    let mut rng = XorShift32::new(seed);

    for _ in 0..size {
        set.add(rng.below(size) as usize);
    }

    set
}

/// Iteration workload: build once, then drain the whole set `passes` times.
///
/// One timed sample per pass, rather than per fixed batch of elements: a
/// cursor's cost is per *walk* as well as per element (it freezes state at
/// creation), and splitting a walk across samples would hide the creation cost
/// in whichever batch happened to contain it. The returned batch size is
/// therefore the number of members yielded per pass, which the driver divides
/// by to get ns/element.
pub fn run_drain(size: u32, seed: u32, passes: usize) -> (Vec<u64>, u64, usize) {
    let set = prefilled(size, seed);
    let per_pass = set.size();

    let mut batches = Vec::with_capacity(passes);
    let mut checksum: u64 = 0;

    for _ in 0..passes {
        let clock = std::time::Instant::now();

        // A fresh cursor per pass, which is what `[...set]` does: the
        // collection's Symbol.iterator is a factory (DIV-STACK-2). Reusing one would
        // measure an exhausted cursor from pass 2 onwards.
        for member in set.values() {
            checksum += u64::from(member);
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&set);

    (batches, checksum, per_pass)
}

/// `--structure`: preallocate a `size`-capacity set and touch it. Moved here
/// from `main.rs`'s former inline match verbatim, when the registry in
/// `harness.rs` replaced that match with a function pointer per module.
pub fn build_structure(size: u32) {
    let set = SparseSet::new(size as usize).expect("benchmark sizes are inside the limit");

    std::hint::black_box(&set);
}

/// The reason `sparse-set` is descoped (PORTBUG-1, `tests/scope.txt`): the napi
/// bridge (`crates/mnemonist-napi/src/sparse_set.rs`) holds its `CoreSet` in a
/// `RefCell` and calls `.borrow()` for `has` / `.borrow_mut()` for
/// `add`/`delete` on every single access — one borrow-flag check and
/// increment/decrement per op, on top of whatever `SparseSet` itself costs.
/// [`run_mixed`] above never goes anywhere near that: it links `SparseSet`
/// directly, which is correct per DESIGN.md 5.1 ("never through N-API") for
/// *comparing the port against upstream*, but it also means nobody has ever
/// measured the RefCell's own cost, because the one harness that could
/// measure it is the one methodologically forbidden from going through the
/// layer that has it.
///
/// This function is the resolution: the exact same mixed workload, over the
/// exact same `SparseSet`, wrapped in a bare `RefCell` and accessed through
/// `.borrow()`/`.borrow_mut()` exactly as the bridge does — reproducing the
/// *mechanism* without reproducing the bridge (still no N-API, still no
/// `mnemonist-napi` dependency here). It answers one question only: what does
/// the borrow-flag check cost, isolated from everything else napi adds
/// (argument marshalling, `Result`-to-`Error` conversion, and so on)? It is
/// deliberately not part of [`crate::harness::MODULES`]: there is no upstream
/// JS analogue of "a Rust RefCell" to compare it against, so it has no
/// `original` figure and cannot be gate-10 evidence. Call it directly with
/// `--refcell-probe` (see `main.rs`); it never touches `bench/results.json`.
pub fn run_mixed_refcell(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let cell = RefCell::new(
        SparseSet::new(workload.size as usize).expect("benchmark sizes are inside the limit"),
    );

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let member = workload.a[i] as usize;

            match workload.kind[i] {
                // `.borrow_mut()`, exactly as the bridge's `add`/`delete` do.
                ADD_A | ADD_B => {
                    cell.borrow_mut().add(member);
                }
                // `.borrow()`, exactly as the bridge's `has` does.
                HAS => checksum += u64::from(cell.borrow().has(member)),
                _ => checksum += u64::from(cell.borrow_mut().delete(member)),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&cell);

    (batches, checksum)
}

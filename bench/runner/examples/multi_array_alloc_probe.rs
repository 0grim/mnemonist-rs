//! Allocation-counting probe for `multi-array`'s bench regression.
//!
//! `docs/modules/multi-array.md`'s own "Bench" section records an
//! *unconfirmed* hypothesis for the p50 loss on `mixed-1e6`: `MultiArray::get`
//! walks the pointer chain and materialises a fresh `Vec<f64>` on every call,
//! one bounds-checked write per step, where upstream's plain `Array` access
//! lets V8 speculate once the shape is monomorphic. The doc says explicitly
//! this "has not been checked against a metric ... that would let it be
//! falsified" — this probe is that metric, for the allocation half of the
//! claim (the walk-vs-allocation split is reported as a magnitude comparison,
//! not decomposed further; see the printed ratios below for what that limits
//! this probe to concluding).
//!
//! Same reasoning as `kd_tree_alloc_probe.rs` for why this is a separate
//! example binary with its own global allocator, not a `bench-runner` CLI
//! flag: allocation counts are not batch-timing numbers, and a counting
//! allocator installed globally in `bench-runner` itself would contaminate
//! every other flag it supports, including the ones `bench/results.json`
//! actually publishes.
//!
//! Run: `cargo run -p bench-runner --release --example multi_array_alloc_probe`.
//!
//! Not gate-10 evidence, not part of `harness::MODULES`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use mnemonist_core::structures::multi_array::MultiArray;

struct CountingAlloc;

static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwards verbatim to `System` -- counting only, no change
        // to layout, alignment or ownership semantics.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same as `alloc` -- forwarded verbatim to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAlloc = CountingAlloc;

fn reset_counters() {
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
}

fn snapshot() -> (u64, u64) {
    (
        ALLOC_COUNT.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
    )
}

/// `bench/runner/src/multi_array.rs`'s own `mixed-1e6` workload: 20,000
/// indices, ~25 values/bucket by the end of a real run. Built here
/// deterministically (25 pushes per index, not a random walk) rather than
/// replaying the matched PRNG -- this probe is about `get`'s own allocation
/// behaviour at a representative bucket size, not a bit-exact replay of the
/// published workload.
const INDICES: usize = 20_000;
const BUCKET_LEN: usize = 25;
const CALLS: usize = 250_000;

fn build_array() -> MultiArray {
    let mut array = MultiArray::new();

    for index in 0..INDICES {
        for item in 0..BUCKET_LEN {
            array
                .set(index, (index * 1000 + item) as f64)
                .expect("dynamic mode never runs out of capacity");
        }
    }

    array
}

fn main() {
    let array = build_array();

    // Warm the structure into cache before either measured pass.
    for index in 0..INDICES {
        std::hint::black_box(array.multiplicity(index));
    }

    reset_counters();
    let get_clock = std::time::Instant::now();
    let mut get_checksum: u64 = 0;
    for call in 0..CALLS {
        let index = call % INDICES;
        if let Some(bucket) = array.get(index) {
            get_checksum += bucket.len() as u64;
        }
    }
    let get_elapsed = get_clock.elapsed();
    let (get_allocs, get_bytes) = snapshot();
    std::hint::black_box(get_checksum);

    reset_counters();
    let mult_clock = std::time::Instant::now();
    let mut mult_checksum: u64 = 0;
    for call in 0..CALLS {
        let index = call % INDICES;
        mult_checksum += array.multiplicity(index) as u64;
    }
    let mult_elapsed = mult_clock.elapsed();
    let (mult_allocs, mult_bytes) = snapshot();
    std::hint::black_box(mult_checksum);

    // A pure allocation baseline: `Vec::with_capacity` + the same
    // `BUCKET_LEN` writes `get`'s own `vec![0.0; length]` plus fill-in-reverse
    // loop performs, with NO pointer-chain walk at all. If this is close to
    // `get`'s own per-call cost minus `multiplicity`'s, the walk is cheap and
    // the allocation is what the doc's hypothesis should be judged against;
    // if it is much smaller, the walk (not the allocation) is carrying more
    // of the gap than the doc's wording credited it for.
    reset_counters();
    let alloc_clock = std::time::Instant::now();
    let mut alloc_checksum: u64 = 0;
    for _ in 0..CALLS {
        let mut v = vec![0.0f64; BUCKET_LEN];
        for (i, slot) in v.iter_mut().enumerate() {
            *slot = i as f64;
        }
        alloc_checksum += std::hint::black_box(&v).len() as u64;
    }
    let alloc_elapsed = alloc_clock.elapsed();
    let (alloc_allocs, alloc_bytes) = snapshot();
    std::hint::black_box(alloc_checksum);

    let get_ns = get_elapsed.as_nanos() as f64 / CALLS as f64;
    let mult_ns = mult_elapsed.as_nanos() as f64 / CALLS as f64;
    let alloc_ns = alloc_elapsed.as_nanos() as f64 / CALLS as f64;

    println!("mode: multi-array allocation probe (not bench/results.json evidence)");
    println!("array: {INDICES} indices, {BUCKET_LEN} values/bucket (deterministic prefill)");
    println!("calls: {CALLS} per variant");
    println!();
    println!("get(index) [walks + materialises a Vec<f64>]:");
    println!(
        "  {get_ns:.2} ns/call, {get_allocs} allocations total ({:.3}/call), {get_bytes} bytes total",
        get_allocs as f64 / CALLS as f64,
    );
    println!("multiplicity(index) [O(1), no walk, no allocation]:");
    println!(
        "  {mult_ns:.2} ns/call, {mult_allocs} allocations total ({:.3}/call), {mult_bytes} bytes total",
        mult_allocs as f64 / CALLS as f64,
    );
    println!("bare Vec::with_capacity({BUCKET_LEN}) + fill, no chain walk:");
    println!(
        "  {alloc_ns:.2} ns/call, {alloc_allocs} allocations total ({:.3}/call), {alloc_bytes} bytes total",
        alloc_allocs as f64 / CALLS as f64,
    );
    println!();
    println!(
        "get - multiplicity (walk + allocation, combined): {:.2} ns/call",
        get_ns - mult_ns
    );
    println!(
        "bare allocation alone (no walk):                  {:.2} ns/call",
        alloc_ns
    );
}

//! Allocation-counting probe for `kd-tree`'s bench regression.
//!
//! `docs/modules/kd-tree.md`'s own "Bench" section records an *unconfirmed*
//! hypothesis for why `k_nearest_neighbors` costs 20× its own
//! `nearest_neighbor` baseline here, against upstream's 3.4×: `recurse_knn`
//! (`crates/mnemonist-core/src/structures/kd_tree.rs`) heap-allocates a fresh
//! 3-element `Vec<f64>` per node visited, pushed into
//! `FixedReverseHeap`'s backing `VecStore`. Reading the source confirms the
//! `vec![dist, *visited as f64, pivot as f64]` call exists on that path and
//! not on `nearest_neighbor`'s — but a hypothesis about what a call *does* is
//! not the same as a measurement of what actually happens at runtime (the
//! compiler is free to elide an allocation whose result never escapes, and it
//! does not here, but that is exactly the kind of claim this project's own
//! rules say must be checked rather than assumed).
//!
//! This is a **counting global allocator**, not a CLI flag on `bench-runner`
//! itself (contrast `--refcell-probe`/`--heap-probe`/`--bit-set-probe` in
//! `main.rs`): "how many mallocs happened" is not expressible as a
//! batch-timing number, and installing a counting allocator globally would
//! contaminate every other flag `bench-runner` supports, including the ones
//! that write to `bench/results.json`. A separate example binary keeps the
//! counting allocator entirely out of the published numbers' process.
//!
//! Run: `cargo run -p bench-runner --release --example kd_tree_alloc_probe`.
//!
//! Not gate-10 evidence, not part of `harness::MODULES`: there is no upstream
//! JS analogue of "a Rust allocation count" to publish a `regressions` array
//! against.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use mnemonist_core::structures::kd_tree::KdTree;

struct CountingAlloc;

static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwards verbatim to `System`, whose own contract this call
        // already satisfies -- this allocator only counts, it does not
        // change layout, alignment or ownership semantics.
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

/// A local xorshift32, algorithm-identical to `bench/runner/src::xorshift`
/// but not required to match its stream bit-for-bit -- this probe reports
/// allocation counts, not a number that goes into `bench/results.json`, so it
/// does not need `bench-runner`'s own matched-PRNG guarantee.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: u32) -> u32 {
        self.next() % bound
    }
}

const SIZE: u32 = 100_000;
const CALLS: usize = 20_000;
const K: usize = 10;

fn scattered_points(size: u32) -> (Vec<Vec<f64>>, Vec<u32>) {
    let mut rng = Rng(1);
    let mut axis0 = Vec::with_capacity(size as usize);
    let mut axis1 = Vec::with_capacity(size as usize);
    let mut labels = Vec::with_capacity(size as usize);

    for i in 0..size {
        axis0.push(f64::from(rng.below(size.max(1))));
        axis1.push(f64::from(rng.below(size.max(1))));
        labels.push(i);
    }

    (vec![axis0, axis1], labels)
}

fn main() {
    let (axes, labels) = scattered_points(SIZE);
    let tree = KdTree::from_axes(axes, labels);

    let mut query_rng = Rng(7);
    let queries: Vec<[f64; 2]> = (0..CALLS)
        .map(|_| {
            [
                f64::from(query_rng.below(SIZE)),
                f64::from(query_rng.below(SIZE)),
            ]
        })
        .collect();

    // Warm the tree/queries into cache before either measured pass, same
    // reasoning as gate 10's own warmup -- this is a `perf`-substitute, not a
    // batch-timed comparison, but "first touch is slower" still applies.
    for query in &queries {
        std::hint::black_box(tree.nearest_neighbor(query));
    }

    reset_counters();
    let nn_clock = std::time::Instant::now();
    let mut nn_checksum: u64 = 0;
    for query in &queries {
        if let Some(label) = tree.nearest_neighbor(query) {
            nn_checksum += u64::from(*label);
        }
    }
    let nn_elapsed = nn_clock.elapsed();
    let (nn_allocs, nn_bytes) = snapshot();
    std::hint::black_box(nn_checksum);

    reset_counters();
    let knn_clock = std::time::Instant::now();
    let mut knn_checksum: u64 = 0;
    for query in &queries {
        if let Ok(hits) = tree.k_nearest_neighbors(K, query) {
            for label in &hits {
                knn_checksum += u64::from(*label);
            }
        }
    }
    let knn_elapsed = knn_clock.elapsed();
    let (knn_allocs, knn_bytes) = snapshot();
    std::hint::black_box(knn_checksum);

    println!("mode: kd-tree allocation probe (not bench/results.json evidence)");
    println!("tree: {SIZE} scattered 2-D points, seed 1 (independent of bench/runner's own workload seed)");
    println!("calls: {CALLS} per method, k = {K}");
    println!();
    println!("nearest_neighbor:");
    println!(
        "  {:.1} ns/call, {} allocations total ({:.3} allocations/call), {} bytes total",
        nn_elapsed.as_nanos() as f64 / CALLS as f64,
        nn_allocs,
        nn_allocs as f64 / CALLS as f64,
        nn_bytes,
    );
    println!("k_nearest_neighbors:");
    println!(
        "  {:.1} ns/call, {} allocations total ({:.3} allocations/call), {} bytes total",
        knn_elapsed.as_nanos() as f64 / CALLS as f64,
        knn_allocs,
        knn_allocs as f64 / CALLS as f64,
        knn_bytes,
    );
    println!();
    println!(
        "ratio: k_nearest_neighbors is {:.1}x nearest_neighbor's own ns/call, \
         {:.1}x its allocation count",
        (knn_elapsed.as_nanos() as f64 / CALLS as f64)
            / (nn_elapsed.as_nanos() as f64 / CALLS as f64).max(1e-9),
        (knn_allocs as f64 / CALLS as f64) / (nn_allocs as f64 / CALLS as f64).max(1e-9),
    );
}

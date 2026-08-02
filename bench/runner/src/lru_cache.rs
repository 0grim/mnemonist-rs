//! The `lru-cache` mixed workload — `Map`/plain-object-backed upstream, a
//! genuinely hot real-world structure, and the one module in this batch the
//! brief predicts we may lose on: V8's own hash structures are heavily
//! optimised, and our `HashMap<u32, usize>` index plus two parallel
//! `Vec<Option<T>>` arrays pay allocation and indirection costs a specialised
//! JIT does not have to.
//!
//! # Capacity vs. key domain — the one parameter this module cannot share
//!
//! Every other module in this batch treats `workload.size` as both the op
//! domain *and* the structure's capacity. Doing that here would be exactly
//! the rigging the brief warns against: if capacity == domain, every key
//! eventually fits and, once warmed, `get` stops missing — a 100% hit rate
//! measures nothing about eviction, which is the entire reason an LRU exists.
//! So `workload.size` is read as the **key domain**, and the cache's capacity
//! is derived as a fixed fraction of it ([`capacity_for`]).
//!
//! Caveat, stated rather than hidden: the access pattern is the same uniform
//! xorshift draw every module here uses. Under i.i.d. uniform access an LRU
//! has no locality to exploit, so its steady-state hit rate converges to
//! roughly `capacity / domain` regardless of how good the eviction policy is
//! — a real workload's skewed (Zipfian, recency-clustered) access would show
//! a materially *higher* hit rate at the same ratio. Read this benchmark as
//! "cost per operation under continuous churn", not as a hit-rate prediction.
//! Building a second, skewed generator to chase a more flattering hit rate
//! would be the mirror image of the rigging above, and is not done here.
//!
//! Op mix: 50% `set` (mutating, no checksum contribution), 25% `get`
//! (mutating — it splays the hit entry to the front — *and* a read,
//! contributing the value or 0 on a miss; the same "mutates and reads" shape
//! as `static-disjoint-set`'s `find` with path compression), 25% `has` (pure
//! read, contributing the boolean). `IK = K = V = u32` with the identity
//! `to_index`, since only strings and numbers ever reach this family's index
//! upstream and a bare integer is the simplest faithful instance.

use mnemonist_core::structures::lru_cache::LruCache;

use crate::workload::Workload;

/// Capacity is 20% of the key domain. Not 100% (a hit rate of 100% once
/// warmed measures nothing about eviction) and not a tiny fixed number like
/// `6` (measures nothing about a real cache's steady state either) — a fifth
/// of the domain keeps both the hit and the eviction paths genuinely
/// exercised for the whole run. See the module docs for the honest limit of
/// what "hit rate" means under this benchmark's uniform access pattern.
const CAPACITY_FRACTION: u32 = 5;

fn capacity_for(domain: u32) -> usize {
    (domain / CAPACITY_FRACTION).max(1) as usize
}

/// One measured pass: fresh cache, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut cache: LruCache<u32, u32, u32> = LruCache::new(capacity_for(workload.size))
        .expect("benchmark capacities are well inside the pointer limit");

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let key = workload.a[i];

            match workload.kind[i] {
                0 | 1 => cache.set(key, key, key, |k| *k),
                2 => checksum += cache.get(&key).map(|v| u64::from(*v)).unwrap_or(0),
                _ => checksum += u64::from(cache.has(&key)),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&cache);

    (batches, checksum)
}

/// `--structure`: preallocate a cache at the same derived capacity and touch
/// it. `LruCache::new` allocates its index and both parallel arrays up front
/// regardless of occupancy, so — like `bit-set` and unlike `heap`/`trie`/
/// `vector` below — this isolates a real preallocation rather than standing
/// in for one.
pub fn build_structure(size: u32) {
    let cache: LruCache<u32, u32, u32> = LruCache::new(capacity_for(size))
        .expect("benchmark capacities are well inside the pointer limit");

    std::hint::black_box(&cache);
    std::hint::black_box(cache.has(&0));
}

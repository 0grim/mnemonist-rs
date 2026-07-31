//! Workload generation and the timed loop.
//!
//! Two rules from DESIGN.md 5.1–5.2 are structural here rather than incidental:
//!
//! * **The op array is materialised before the timed region.** Generation is
//!   never measured.
//! * **Timing is batched at K = 1000 ops.** `Instant::now()` costs ~20–30 ns
//!   and a `find` on a compressed forest is single-digit ns, so per-op timing
//!   would measure the clock. Batching drops timer cost to ~0.03% of a sample
//!   *and* — the reason it is the right call rather than merely a cheaper one —
//!   puts each V8 GC pause inside exactly one batch, which is what makes
//!   batch-level p99 the metric that shows tail behaviour.

use mnemonist_core::structures::static_disjoint_set::StaticDisjointSet;

use crate::xorshift::XorShift32;

/// Op kinds, as drawn from the PRNG. Mirrored in `bench/node/run.js`.
const UNION_A: u8 = 0;
const UNION_B: u8 = 1;
const FIND: u8 = 2;

/// A materialised op sequence: parallel arrays rather than a `Vec<enum>`, so
/// the layout matches the typed arrays the JS side uses and neither runtime is
/// handed a friendlier representation.
pub struct Workload {
    pub size: u32,
    pub kind: Vec<u8>,
    pub a: Vec<u32>,
    pub b: Vec<u32>,
}

impl Workload {
    pub fn len(&self) -> usize {
        self.kind.len()
    }
}

/// Draw exactly three PRNG values per op — kind, then both operands — whether
/// or not the op uses the second one.
///
/// A conditional third draw would desynchronise the two implementations at the
/// first `find`, which is the subtle way a "matched" PRNG stops matching.
pub fn generate(size: u32, ops: usize, seed: u32) -> Workload {
    let mut rng = XorShift32::new(seed);

    let mut kind = Vec::with_capacity(ops);
    let mut a = Vec::with_capacity(ops);
    let mut b = Vec::with_capacity(ops);

    for _ in 0..ops {
        kind.push((rng.next() % 4) as u8);
        a.push(rng.below(size));
        b.push(rng.below(size));
    }

    Workload { size, kind, a, b }
}

/// One measured pass: a fresh set, then the whole workload in batches of `k`.
///
/// Returns per-batch nanoseconds and a checksum. The checksum is not
/// bookkeeping — `bench/run.sh` refuses to record a result unless the Rust and
/// Node checksums are identical, which proves both sides executed the same
/// sequence and got the same answers, not merely the same op *count*.
pub fn run_once(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    // Construction is outside the timed region but inside the run, so each
    // pass starts from the same fully disjoint forest. Reusing one set across
    // passes would make pass 2 onwards measure an already-merged structure.
    let mut set = StaticDisjointSet::new(workload.size as usize)
        .expect("benchmark sizes are well inside the pointer limit");

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            let (x, y) = (workload.a[i] as usize, workload.b[i] as usize);

            match workload.kind[i] {
                // union mutates, so it cannot be optimised away and needs no
                // contribution to the checksum. The JS side adds nothing here
                // either.
                UNION_A | UNION_B => {
                    set.union(x, y);
                }
                FIND => checksum += set.find(x) as u64,
                _ => checksum += u64::from(set.connected(x, y)),
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    // Keep the set alive past the loop so nothing above can be reordered out.
    std::hint::black_box(&set);

    (batches, checksum)
}

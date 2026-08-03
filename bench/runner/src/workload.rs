//! Workload generation and the timed loop.
//!
//! Two rules from `bench/methodology.md`–5.2 are structural here rather than incidental:
//!
//! * **The op array is materialised before the timed region.** Generation is
//!   never measured.
//! * **Timing is batched at K = 1000 ops** for the op-stream workloads. `Instant::now()` costs ~20–30 ns
//!   and a `find` on a compressed forest is single-digit ns, so per-op timing
//!   would measure the clock. Batching drops timer cost to ~0.03% of a sample
//!   *and* — the reason it is the right call rather than merely a cheaper one —
//!   puts each V8 GC pause inside exactly one batch, which is what makes
//!   batch-level p99 the metric that shows tail behaviour.

use crate::xorshift::XorShift32;

/// Op kinds, as drawn from the PRNG. Mirrored in `bench/node/run.js`.
///
/// One `kind % 4` stream serves every module; each names the four values for
/// its own alphabet so the 50/25/25 shape is identical and the two modules'
/// mixed workloads remain comparable to each other as well as to upstream.
pub const UNION_A: u8 = 0;
pub const UNION_B: u8 = 1;
pub const FIND: u8 = 2;

/// `sparse-set`'s names for the same four values: 50% `add`, 25% `has`, and
/// the remaining quarter `delete`.
pub const ADD_A: u8 = 0;
pub const ADD_B: u8 = 1;
pub const HAS: u8 = 2;

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

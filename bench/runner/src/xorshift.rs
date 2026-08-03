//! xorshift32, the matched PRNG.
//!
//! `bench/methodology.md`: both sides must execute the *identical* op sequence, or the
//! benchmark is partly a comparison of two PRNGs. The mechanism specified there
//! is a matched generator rather than a serialised workload file — a 1e6-op
//! `workload.jsonl` would be ~30 MB and would drag JSON parsing into the
//! measurement, while ten lines of xorshift32 have zero I/O and are provably
//! identical.
//!
//! "Provably" is not rhetorical here: `bench-runner --dump-prng N` and
//! `node bench/node/run.js --dump-prng N` print the same stream, and
//! `bench/run.sh` diffs the first 1000 values before it will benchmark
//! anything.
//!
//! The JS twin lives in `bench/node/run.js`. Keep them in step: the `>>> 0` on
//! that side is what makes JS's 64-bit-float bitwise semantics agree with a
//! Rust `u32`, and `wrapping_*` is not needed here because `u32` shifts already
//! truncate.

/// Matches `x ^= x << 13; x ^= x >>> 17; x ^= x << 5;` exactly.
pub struct XorShift32(u32);

impl XorShift32 {
    /// # Panics
    ///
    /// Panics on a zero seed, which is xorshift's fixed point and would yield
    /// an infinite run of zeroes — silently turning the workload into one op
    /// repeated a million times.
    pub fn new(seed: u32) -> Self {
        assert!(seed != 0, "xorshift32 cannot be seeded with zero");

        Self(seed)
    }

    pub fn next(&mut self) -> u32 {
        let mut x = self.0;

        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;

        x
    }

    /// Uniform-enough index into `0..bound`.
    ///
    /// Plain modulo, with the bias that implies. Deliberate: the JS side must
    /// reproduce it exactly, and rejection sampling would consume a
    /// data-dependent number of PRNG values, which is far harder to keep in
    /// step than a bias neither side benefits from.
    pub fn below(&mut self, bound: u32) -> u32 {
        self.next() % bound
    }
}

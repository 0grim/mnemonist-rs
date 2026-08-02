//! The `bit-vector` mixed workload — a growable bit set, `bit-set` with
//! `push`/`pop`/`grow` added on top of the same word-array backing (see
//! `mnemonist_core::structures::bits`, shared by both modules).
//!
//! Op mix: 50% `push` (mutating growth) / 25% `get` at a uniformly random
//! *existing* index (pure read, modulo the current length) / 25% `pop`
//! (mutating and a read) — `vector`/`hashed-array-tree`'s shape, not
//! `bit-set`'s: this module grows under `push` rather than being
//! preallocated to a fixed domain, so there is no `size`-as-capacity
//! parameter to set here.
//!
//! `rank`/`select` are excluded for the reason recorded in `bit_set.rs`:
//! neither has an index behind it, so a single call is O(i / 32) words, not
//! O(1), and a uniform-weighted mix would put a domain-scaling cost next to
//! three genuinely O(1) ops. `push`/`pop`/`get` are all O(1) here (word/bit
//! arithmetic on the last or a directly-addressed word), so the trap that
//! caught `rank` does not apply to this op set.
//!
//! `workload.size` only bounds the magnitude of the parity used to decide
//! `push`'s bit, exactly as `vector`'s bounds pushed magnitude: a fresh
//! vector starts at length 0 and grows one word at a time under `push` alone.

use mnemonist_core::structures::bit_vector::BitVector;

use crate::workload::Workload;

/// One measured pass: fresh vector, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut vector = BitVector::new(0);

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            match workload.kind[i] {
                0 | 1 => {
                    vector
                        .push(workload.a[i] % 2 == 1)
                        .expect("the default growth policy never refuses at these sizes");
                }
                2 => {
                    let len = vector.length();

                    if len > 0 {
                        let index = (workload.a[i] as usize) % len;
                        checksum += u64::from(vector.get(index as i64).unwrap_or(0));
                    }
                }
                _ => {
                    checksum += u64::from(vector.pop().unwrap_or(0));
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&vector);

    (batches, checksum)
}

/// `--structure`: like `vector`/`hashed-array-tree`, no capacity distinct
/// from the pushed length, so "size" means "pushed `size` bits".
pub fn build_structure(size: u32) {
    let mut vector = BitVector::new(0);

    for i in 0..size {
        vector
            .push(i % 2 == 1)
            .expect("the default growth policy never refuses at these sizes");
    }

    std::hint::black_box(&vector);
}

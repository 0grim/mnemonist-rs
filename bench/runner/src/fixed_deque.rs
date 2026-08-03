//! The `fixed-deque` mixed workload — a double-ended queue of fixed capacity
//! laid out as a ring. Second of the three fixed-capacity modules; see
//! `fixed_stack.rs` for the shape and the guard both share.
//!
//! Op mix: 50% `push` / 25% `peek_last` (pure read) / 25% `pop` — back-end
//! operations only, mirroring `fixed-stack`'s exact shape so the batch stays
//! one repeatable pattern rather than a bespoke mix per module. `unshift`/
//! `shift` exercise the same ring arithmetic from the other end and are not
//! separately measured here; `get`, which `FixedDeque` also offers, is
//! bounded by the *capacity* rather than the size (BUG-CIRCULAR-BUFFER-1) and reading it
//! safely within `[0, size)` would need the same clamping `vector`'s `get`
//! does, which is more machinery than this batched pass needs for one more
//! pure-read data point.
//!
//! `workload.size` is the capacity directly. As with `fixed-stack`, `push` is
//! guarded by `size < capacity` so the timed loop never calls the fallible
//! path — an unguarded push into a full deque would benchmark V8's `Error`
//! construction (stack-trace capture), not the ring. See `fixed_stack.rs`'s
//! module docs for the full account; the mechanism is identical here.

use mnemonist_core::structures::backing::Backing;
use mnemonist_core::structures::fixed_deque::FixedDeque;

use crate::workload::Workload;

/// One measured pass: fresh deque at `workload.size` capacity, then the whole
/// workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut deque: FixedDeque<f64> = FixedDeque::new(Backing::Filled(0.0), workload.size as usize)
        .expect("benchmark sizes are inside the limit");
    let capacity = deque.capacity();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            match workload.kind[i] {
                0 | 1 => {
                    if deque.size() < capacity {
                        let _ = deque.push(f64::from(workload.a[i]));
                    }
                }
                2 => {
                    checksum += deque.peek_last().map(|v| v as u64).unwrap_or(0);
                }
                _ => {
                    checksum += deque.pop().map(|v| v as u64).unwrap_or(0);
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&deque);

    (batches, checksum)
}

/// `--structure`: preallocate a `size`-capacity deque and touch it.
pub fn build_structure(size: u32) {
    let deque: FixedDeque<f64> =
        FixedDeque::new(Backing::Filled(0.0), size as usize).expect("size is a valid capacity");

    std::hint::black_box(&deque);
}

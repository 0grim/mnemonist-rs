//! The `linked-list` mixed workload — `u32` items over a shared
//! `LinkedList<u32>`.
//!
//! # The load-bearing parameter: an op that actually chases pointers
//!
//! `push` (tail) and `shift` (head) are both O(1) — they only ever touch the
//! two ends the list already holds pointers to, exactly like a plain
//! array-backed deque would. A workload of only `push`/`shift` would never
//! exercise the one thing a *linked* list does differently from a
//! contiguous one: walking from node to node by following `next`, which is
//! cache-unfriendly in a way indexing into an array never is. So a third op,
//! **`walk`**, opens a fresh cursor at the head and steps it forward
//! [`WALK_STEPS`] times — upstream's own lazy generator (`values()`), the
//! same primitive `forEach`/`entries`/`Symbol.iterator` all share — genuinely
//! following `WALK_STEPS` pointers rather than answering in O(1) the way
//! `first()`/`last()` would.
//!
//! Op mix: 50% `push` (mutating, no checksum contribution), 25% `shift`
//! (mutating and a read, contributing the removed value — `sparse-set`'s own
//! shape for its front-removal op), 25% `walk` (a read that chases
//! [`WALK_STEPS`] pointers, contributing whatever item it lands on).
//!
//! # The arena never frees a shifted node's slot
//!
//! `LinkedList::shift` only advances `head`; the node's own arena slot stays
//! allocated (see that module's own docs on why — a live cursor may still
//! reference it). So `push`-heavy traffic over 1e6 ops grows the arena to
//! roughly the number of `push` calls regardless of how many are later
//! `shift`ed away, which is real and expected: `structure_rss_delta_mb`
//! reports exactly this, not a leak.

use mnemonist_core::structures::linked_list::LinkedList;

use crate::workload::Workload;

/// How many pointers `walk` follows per call — enough to be a genuine
/// multi-hop traversal rather than "peek at the head", small enough that
/// its cost stays a fraction of a batch rather than dominating it.
const WALK_STEPS: usize = 20;

/// One measured pass: fresh list, then the whole workload in batches of `k`.
pub fn run_mixed(workload: &Workload, k: usize) -> (Vec<u64>, u64) {
    let mut list: LinkedList<u32> = LinkedList::new();

    let ops = workload.len();
    let mut batches = Vec::with_capacity(ops.div_ceil(k));
    let mut checksum: u64 = 0;

    for start in (0..ops).step_by(k) {
        let end = (start + k).min(ops);
        let clock = std::time::Instant::now();

        for i in start..end {
            match workload.kind[i] {
                0 | 1 => {
                    list.push(workload.a[i]);
                }
                2 => {
                    if let Some(value) = list.shift() {
                        checksum += u64::from(value);
                    }
                }
                _ => {
                    let mut cursor = list.values();
                    let mut last = None;

                    for _ in 0..WALK_STEPS {
                        match cursor.step(&list) {
                            Some(value) => last = Some(*value),
                            None => break,
                        }
                    }

                    if let Some(value) = last {
                        checksum += u64::from(value);
                    }
                }
            }
        }

        batches.push(clock.elapsed().as_nanos() as u64);
    }

    std::hint::black_box(&list);

    (batches, checksum)
}

/// `--structure`: a `LinkedList` has no capacity distinct from occupied
/// size — "size" means "prefilled with `size` pushes".
pub fn build_structure(size: u32) {
    let mut list: LinkedList<u32> = LinkedList::new();

    for i in 0..size {
        list.push(i);
    }

    std::hint::black_box(&list);
}

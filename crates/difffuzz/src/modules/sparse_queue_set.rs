//! [`ModuleSpec`] for `sparse-queue-set`.
//!
//! # What this grammar reaches
//!
//! The ring. `sparse-set` and `sparse-map` both have a `dense` array that only
//! ever grows forwards from slot 0; here it wraps, and `start` moves. Three
//! consequences the generator is built around:
//!
//! * **Rotation is state.** Two queues holding the same members in the same
//!   order can sit at different `start`s, and every membership test, every
//!   enqueue position and every walk depends on it. `start` is therefore in the
//!   observed state, alongside `size` and `capacity`.
//! * **`dequeue` is the only op that rotates**, so it is weighted heavily
//!   enough that a 200-op program goes round the ring many times rather than
//!   filling it once. At `capacity` 8 with the weights below, a program wraps
//!   on the order of ten times.
//! * **The interesting capacity is exact.** BUG-SPARSE-QUEUE-SET-1 — `dequeue`'s absence sentinel
//!   truncating because it is written into an array sized for indices
//!   `0..capacity-1` — happens at `capacity` **256** and **65536** and nowhere
//!   else. A uniform range over `0..=400` would hit 256 about once in 400
//!   programs, so 256 is drawn explicitly alongside the range, with 255 as its
//!   control. 65536 is left to a native test; see [`BOUNDARY_CAPACITIES`] for
//!   the measurement behind that.
//!
//! The sentinel itself needs no special op to observe: `sparse` is in the
//! observed state, so every `dequeue` compares the value written into it.
//!
//! # Observable state
//!
//! `size`, `capacity`, `start`, `dense`, `sparse`. All five are public
//! properties upstream. `start` is the one this module adds over its siblings,
//! and it is load-bearing: a port that got the wrap right in `values()` but
//! wrong in `dequeue()` would still agree on every member — for a while.
//!
//! # Deliberately excluded
//!
//! Nothing. Every out-of-range member is generated, and reproducing what
//! upstream does with it is the port's job — here that means BUG-SPARSE-QUEUE-SET-2, an
//! out-of-range `enqueue` evicting a **live** member and pushing `size` past
//! `capacity`, which is a strictly nastier corruption than `SparseSet.add`'s
//! because the slot it overwrites belonged to someone.

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::sparse_queue_set::SparseQueueSet;
use mnemonist_core::utils::typed_arrays::{PointerVec, PointerWidth};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{
    for_each, for_each_args, for_each_index, for_each_strategy, ModuleSpec, Op, FOR_EACH_MANY,
};

/// Upper end of the ordinary capacity range.
///
/// Straddles 256, where `getPointerArray` switches both arrays to 16-bit.
const MAX_CAPACITY: u32 = 400;

/// The capacity where BUG-SPARSE-QUEUE-SET-1's sentinel does not fit its own array, and its
/// control.
///
/// Drawn explicitly because it is a point, not a range: a uniform draw over
/// `0..=400` would reach 256 about once in 400 programs. 255 rides along as the
/// control — one below the boundary, where the sentinel fits and the queue
/// behaves — so a port that "fixed" BUG-SPARSE-QUEUE-SET-1 fails on 256 while a port that broke
/// the *ordinary* case fails on 255.
///
/// **65536, the second boundary, is deliberately NOT here.** It is the same
/// defect one width up, and it is covered by
/// `the_sentinel_truncates_at_the_second_boundary_too` in the core's tests
/// instead. Including it costs about 95% of this campaign's throughput —
/// measured, 880 op/s against 15,000 — because the observable state is two
/// backing arrays and they get serialised, sent and compared after *every*
/// operation. A 60-second campaign that executes 5% of the programs is a worse
/// check than a native test plus a fast campaign.
const BOUNDARY_CAPACITIES: [u32; 2] = [255, 256];

/// How far past `capacity` a generated member may reach.
const OVERSHOOT: u32 = 64;

pub struct SparseQueueSetSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/sparse-queue-set.txt"
);

/// The queue plus the one cursor a program can have open over it.
pub struct Instance {
    queue: SparseQueueSet,
    cursor: Option<CursorState<SparseQueueSet>>,
}

impl ModuleSpec for SparseQueueSetSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "sparse-queue-set"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "capacity", "start", "dense", "sparse"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        prop_oneof![
            // 0 included: `new SparseQueueSet(0)` is legal and makes every
            // index computation NaN (BUG-SPARSE-QUEUE-SET-3).
            4 => 0u32..=MAX_CAPACITY,
            // Small rings, where a 200-op program wraps many times over.
            3 => 1u32..=8,
            // The two width boundaries and their controls.
            2 => prop::sample::select(BOUNDARY_CAPACITIES.as_slice()),
        ]
        .prop_map(|capacity| vec![json!(capacity)])
        .boxed()
    }

    fn op_strategy(&self, ctor: &[Value]) -> BoxedStrategy<Op> {
        let capacity = ctor[0].as_u64().expect("ctor arg 0 is the capacity") as u32;
        let member = 0u32..(capacity + OVERSHOOT);

        prop_oneof![
            4 => member.clone().prop_map(|m| Op::new("enqueue", vec![json!(m)])),
            // Heavy, because it is the only op that moves `start`, and the ring
            // is the thing this module has that its siblings do not.
            3 => Just(Op::new("dequeue", vec![])),
            3 => member.prop_map(|m| Op::new("has", vec![json!(m)])),
            1 => Just(Op::new("clear", vec![])),
            2 => Just(Op::new("$iter", vec![json!("values")])),
            3 => Just(Op::new("$next", vec![])),
            1 => Just(Op::new("$spread", vec![])),
            // The point of contrast with `sparse-set` and `sparse-map`: this
            // module's `forEach` FREEZES `c`, `l` and `i` before the loop, so
            // a callback that dequeues does not shorten it and a callback that
            // enqueues does not lengthen it. Generating the same shape of
            // program against both is the only way that difference is checked
            // rather than assumed. See `crate::spec::ForEach`.
            2 => for_each_strategy(MUTATIONS),
        ]
        .boxed()
    }

    fn program_len(&self) -> std::ops::Range<usize> {
        1..200
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let capacity = args[0].as_u64().expect("ctor arg 0 is the capacity") as usize;

        Instance {
            queue: SparseQueueSet::new(capacity)
                .expect("generated capacities are inside the pointer limit"),
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            // Upstream returns `this` for chaining; the oracle encodes that as
            // `{"$self": true}`.
            "enqueue" => {
                instance.queue.enqueue(member(op));
                json!({"$self": true})
            }
            // `undefined` on an empty queue, and on any queue of capacity 0,
            // where `dense[start]` is `undefined` too.
            "dequeue" => match instance.queue.dequeue() {
                Some(member) => json!(member),
                None => json!({"$undefined": true}),
            },
            "has" => json!(instance.queue.has(member(op))),
            "clear" => {
                instance.queue.clear();
                json!({"$undefined": true})
            }
            "$iter" => {
                instance.cursor = Some(CursorState::open(&instance.queue));
                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some(cursor) => step_value(cursor.step(&instance.queue)),
            },
            // A *fresh* cursor every time, which is what the collection-level
            // `Symbol.iterator` does upstream.
            // Upstream's own loop, frozen bounds and all:
            //
            // ```js
            // var c = this.capacity, l = this.size, i = this.start, j = 0;
            // while (j < l) {
            //   callback.call(scope, this.dense[i], j, this);
            //   i++; j++;
            //   if (i === c) i = 0;
            // }
            // ```
            //
            // `dense[i]` is a live read; everything else is captured. At
            // `capacity === 0` the wrap check never fires and `i` runs off the
            // end, so every argument is `undefined` -- BUG-SPARSE-QUEUE-SET-3's shape, now
            // reachable through `forEach` and not only through the cursor.
            "$forEach" => {
                let spec = for_each(op);
                let capacity = instance.queue.capacity();
                let length = instance.queue.size();
                let mut ring = instance.queue.start();
                let mut ordinal = 0usize;
                let mut seen = Vec::new();
                let mut fired = 0usize;

                while ordinal < length {
                    let member = slot(instance.queue.dense().try_get(ring));
                    let received = vec![member, json!(ordinal)];

                    seen.push(Value::Array(received.clone()));

                    if fired < spec.limit {
                        if let Some(args) = for_each_args(&spec, &received) {
                            fired += 1;

                            match spec.method.expect("for_each_args returned Some") {
                                "enqueue" => {
                                    instance.queue.enqueue(for_each_index(&spec, args[0]));
                                }
                                "dequeue" => {
                                    instance.queue.dequeue();
                                }
                                "clear" => instance.queue.clear(),
                                other => {
                                    panic!("`{other}` is not a $forEach mutation for this module")
                                }
                            }
                        }
                    }

                    ring += 1;
                    ordinal += 1;

                    if ring == capacity {
                        ring = 0;
                    }
                }

                json!({ "seen": seen })
            }
            "$spread" => {
                let mut cursor = CursorState::open(&instance.queue);
                let mut items = Vec::new();

                loop {
                    match cursor.step(&instance.queue) {
                        Step::Item(item) => items.push(json!(item)),
                        Step::Gap => items.push(json!({"$undefined": true})),
                        Step::Done => break,
                    }
                }

                Value::Array(items)
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.queue.size(),
            "capacity": instance.queue.capacity(),
            "start": instance.queue.start(),
            "dense": typed_array(instance.queue.dense()),
            "sparse": typed_array(instance.queue.sparse()),
        })
    }
}

/// What the callback may do to the queue, and how often.
///
/// All three are safe uncapped: the loop bound is captured before the first
/// step, so nothing the callback does can extend the walk.
const MUTATIONS: &[(&str, &str, u64)] = &[
    ("dequeue", "none", FOR_EACH_MANY),
    ("enqueue", "arg0+1", FOR_EACH_MANY),
    ("clear", "none", FOR_EACH_MANY),
];

/// A `dense` slot as the oracle encodes it.
fn slot(value: Option<u32>) -> Value {
    match value {
        Some(item) => json!(item),
        None => json!({"$undefined": true}),
    }
}

fn member(op: &Op) -> usize {
    op.args[0]
        .as_u64()
        .expect("generated members are non-negative integers") as usize
}

/// A step, in the shape `fuzz/oracle.js` normalises both sides to.
fn step_value(step: Step<u32>) -> Value {
    match step {
        Step::Item(item) => json!({"done": false, "value": item}),
        Step::Gap => json!({"done": false, "value": {"$undefined": true}}),
        Step::Done => json!({"done": true, "value": {"$undefined": true}}),
    }
}

/// Encode a backing array exactly as the oracle encodes a JS typed array.
fn typed_array(values: &PointerVec) -> Value {
    let name = match values.width() {
        PointerWidth::U8 => "Uint8Array",
        PointerWidth::U16 => "Uint16Array",
        PointerWidth::U32 => "Uint32Array",
    };

    json!({
        "$typed": name,
        "values": (0..values.len()).map(|i| values.get(i)).collect::<Vec<u32>>(),
    })
}

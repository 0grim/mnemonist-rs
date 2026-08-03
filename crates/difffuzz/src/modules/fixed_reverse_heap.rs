//! [`ModuleSpec`] for `fixed-reverse-heap`.
//!
//! Shares `heap`'s comparator factories (see [`crate::modules::heap`]) — the
//! mutating ones are the point here too, and this structure reaches them
//! through a *different* pair of algorithms: its own `size`-bounded `siftUp`
//! and its backwards-filling `consume`.
//!
//! # What is different from `heap`'s grammar
//!
//! * **The capacity is generated**, `0` included. A capacity of `0` is accepted
//!   upstream because the guard is `&&` where `||` was meant (BUG-FIXED-REVERSE-HEAP-1), and the
//!   heap then discards every push in silence. A grammar that only generated
//!   sensible capacities would never have visited that branch.
//! * **`clearer` is excluded.** `FixedReverseHeap.prototype.clear` sets `size`
//!   and does not touch `items`, so it is not the rebinding case `heap` uses it
//!   for; `clear` is an ordinary op in the alphabet instead, which reaches the
//!   stale-`peek()` bug (BUG-FIXED-REVERSE-HEAP-2) directly.
//! * **`Array` only.** Upstream's `ArrayClass` may be any typed array, and the
//!   element narrowing that comes with one (`push(300)` keeping `44`) is a
//!   JavaScript store semantic that `mnemonist-core`'s `VecStore` does not
//!   have. Fuzzing it would compare the port against a behaviour the port
//!   deliberately leaves to the bridge. It is covered instead by
//!   `test/fixed-reverse-heap.js`, which uses `Uint8Array` in five of its seven
//!   cases, and by `tests/boundary/heap.js`, which asserts the narrowing
//!   through the real bridge. Recorded as a gap in
//!   `docs/modules/fixed-reverse-heap.md`.
//!
//! # Observable state
//!
//! `size`, `capacity` and `items`. `items` is `capacity` slots long from
//! construction and keeps its contents through a `clear()`, so observing it is
//! what makes BUG-FIXED-REVERSE-HEAP-2 visible without waiting for a `peek`.

use mnemonist_core::structures::fixed_reverse_heap::FixedReverseHeap;
use mnemonist_core::structures::heap::{Store, VecStore};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::modules::heap::{
    factory, factory_name, number, slot, slots, thrown, FuzzComparator, Kind,
};
use crate::spec::{ModuleSpec, Op};

const VALUES: std::ops::Range<i64> = 0..24;

/// Capacities the generator draws from. `0` is deliberate — see the module
/// docs.
const CAPACITIES: std::ops::Range<i64> = 0..5;

pub struct FixedReverseHeapSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/fixed-reverse-heap.txt"
);

pub struct Instance {
    heap: FixedReverseHeap<VecStore<f64>, FuzzComparator>,
}

impl ModuleSpec for FixedReverseHeapSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "fixed-reverse-heap"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "capacity", "items"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // `new FixedReverseHeap(ArrayClass, comparator, capacity)`. The
        // comparator-omitted two-argument form is upstream's other signature;
        // both are generated, because `arguments.length === 2` is what selects
        // between them and the bridge cannot see arity.
        let comparators = prop_oneof![
            4 => Just("ascending"),
            2 => Just("descending"),
            2 => Just("pushy"),
            2 => Just("popper"),
            1 => Just("boom"),
        ];

        (comparators, CAPACITIES, proptest::bool::weighted(0.3))
            .prop_map(|(name, capacity, omit_comparator)| {
                let class = json!({"$global": "Array"});

                if omit_comparator {
                    // `new FixedReverseHeap(Array, capacity)` — two arguments,
                    // and upstream shuffles them itself.
                    return vec![class, json!(capacity)];
                }

                vec![class, factory(name), json!(capacity)]
            })
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            7 => VALUES.prop_map(|value| Op::new("push", vec![json!(value)])),
            2 => Just(Op::new("peek", vec![])),
            2 => Just(Op::new("clear", vec![])),
            2 => Just(Op::new("toArray", vec![])),
            1 => Just(Op::new("consume", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        // `if (arguments.length === 2) { capacity = comparator; comparator = null; }`
        let (kind, capacity) = match args.len() {
            2 => (Kind::Ascending, number(&args[1])),
            _ => (Kind::from_factory(factory_name(&args[1])), number(&args[2])),
        };
        let capacity = capacity as usize;

        // `this.items = new ArrayClass(capacity)` — `capacity` holes.
        let items = VecStore::<f64>::new();

        for _ in 0..capacity {
            items.push(None).expect("VecStore is infallible");
        }

        let comparator = FuzzComparator::new(kind);

        // No `Weak` to a heap here: this structure never rebinds `items`, so
        // the array the comparator mutates is the one it was handed. That
        // asymmetry with `heap` is upstream's, not the harness's.
        comparator.attach_items(items.clone());

        Instance {
            heap: FixedReverseHeap::new(items, comparator, capacity),
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        let heap = &instance.heap;

        match op.name {
            "push" => thrown(heap.push(Some(number(&op.args[0]))).map(|size| json!(size))),
            "peek" => thrown(heap.peek().map(slot)),
            "clear" => {
                heap.clear();
                json!({"$undefined": true})
            }
            "consume" => thrown(heap.consume().map(|items| slots(&items))),
            "toArray" => thrown(heap.to_array().map(|items| slots(&items))),
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.heap.size(),
            "capacity": instance.heap.capacity(),
            "items": slots(&instance.heap.items()),
        })
    }
}

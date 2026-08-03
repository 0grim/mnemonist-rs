//! [`ModuleSpec`] for `fixed-deque`.
//!
//! # What this grammar exists to reach
//!
//! The ring, from both ends, past every wrap. `push` and `unshift` are both
//! weighted, capacities run 1..=8, and programs are up to 200 ops, so a
//! generated program wraps `start` around many times and spends a large share
//! of its length at the capacity boundary where both inserts throw.
//!
//! Three behaviours it is aimed at specifically:
//!
//! * **`#.get` is bounded by the capacity, not by the size** (NOTES BUG-CIRCULAR-BUFFER-1), so
//!   generated indices run past the size *and* past the capacity — the first
//!   returns debris, the second is the one guard that fires.
//! * **`start` is observable**, and the upstream test file asserts on it
//!   directly, so it is in the observation set alongside `items`.
//! * **`forEach` freezes `start` as well as the size**, which `$forEach` with a
//!   mutating callback is the only op that can see: a `shift` inside the
//!   callback moves the deque's start but not the walk's.
//!
//! See `crate::modules::fixed_stack` for the encoding of a hole, the reason
//! both backing classes are generated, and why the `Uint8Array` element store
//! is modelled in this file rather than in core.

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::backing::Backing;
use mnemonist_core::structures::fixed_deque::FixedDeque;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::modules::fixed_stack::{optional, slots, step_value, store, Hole};
use crate::spec::{for_each, for_each_args, for_each_strategy, ModuleSpec, Op, FOR_EACH_MANY};

/// Largest value pushed. Above 255 so a `Uint8Array` deque truncates.
const MAX_VALUE: u32 = 320;

/// Largest capacity generated.
const MAX_CAPACITY: u32 = 8;

/// Largest index a generated `get` asks for — past `MAX_CAPACITY`, so both
/// halves of BUG-CIRCULAR-BUFFER-1's guard are exercised.
const MAX_INDEX: u32 = 11;

/// Mutations `$forEach`'s callback may perform, and how often. All three are
/// nullary and non-throwing, so firing on every step is always well formed.
const MUTATIONS: &[(&str, &str, u64)] = &[
    ("pop", "none", FOR_EACH_MANY),
    ("shift", "none", FOR_EACH_MANY),
    ("clear", "none", FOR_EACH_MANY),
];

pub struct FixedDequeSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/fixed-deque.txt"
);

pub struct Instance {
    deque: FixedDeque<Value>,
    typed: bool,
    cursor: Option<CursorState<FixedDeque<Value>>>,
}

impl ModuleSpec for FixedDequeSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "fixed-deque"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "capacity", "start", "items", "toArray"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        (
            prop::sample::select(&["Array", "Uint8Array"][..]),
            1u32..=MAX_CAPACITY,
        )
            .prop_map(|(class, capacity)| vec![json!({ "$global": class }), json!(capacity)])
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            5 => (0u32..MAX_VALUE).prop_map(|value| Op::new("push", vec![json!(value)])),
            3 => (0u32..MAX_VALUE).prop_map(|value| Op::new("unshift", vec![json!(value)])),
            2 => Just(Op::new("pop", vec![])),
            2 => Just(Op::new("shift", vec![])),
            1 => Just(Op::new("peekFirst", vec![])),
            1 => Just(Op::new("peekLast", vec![])),
            // Past both the size and the capacity: BUG-CIRCULAR-BUFFER-1's two clauses.
            2 => (0u32..=MAX_INDEX).prop_map(|index| Op::new("get", vec![json!(index)])),
            1 => Just(Op::new("clear", vec![])),
            2 => Just(Op::new("$iter", vec![json!("values")])),
            4 => Just(Op::new("$next", vec![])),
            1 => Just(Op::new("$spread", vec![])),
            // The mutating walk, plus a plain walk (no method) as the control
            // case — see `crate::spec::for_each_strategy`.
            3 => for_each_strategy(MUTATIONS),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let typed = args[0]["$global"].as_str() == Some("Uint8Array");
        let capacity = args[1]
            .as_u64()
            .expect("generated capacities are positive integers") as usize;
        let backing = if typed {
            Backing::Filled(json!(0))
        } else {
            Backing::Holes
        };

        Instance {
            deque: FixedDeque::new(backing, capacity).expect("capacity is at least one"),
            typed,
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "push" => match instance.deque.push(store(instance.typed, &op.args[0])) {
                Ok(size) => json!(size),
                Err(error) => json!({ "$throw": error.to_string() }),
            },
            "unshift" => match instance.deque.unshift(store(instance.typed, &op.args[0])) {
                Ok(size) => json!(size),
                Err(error) => json!({ "$throw": error.to_string() }),
            },
            "pop" => optional(instance.deque.pop()),
            "shift" => optional(instance.deque.shift()),
            "peekFirst" => optional(instance.deque.peek_first()),
            "peekLast" => optional(instance.deque.peek_last()),
            "get" => optional(instance.deque.get(index(&op.args[0]))),
            "clear" => {
                instance.deque.clear();
                json!({"$undefined": true})
            }
            "$iter" => {
                instance.cursor = Some(CursorState::open(&instance.deque));
                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some(cursor) => step_value(cursor.step(&instance.deque)),
            },
            "$spread" => {
                let mut cursor = CursorState::open(&instance.deque);
                let mut items = Vec::new();

                loop {
                    match cursor.step(&instance.deque) {
                        Step::Item(item) => items.push(item),
                        Step::Gap => items.push(json!({"$undefined": true})),
                        Step::Done => break,
                    }
                }

                Value::Array(items)
            }
            // `capacity`, `size` and `start` are frozen together at entry and
            // `this.items` is read live, which is exactly [`CursorState`] — so
            // this drives one rather than reimplementing the loop, and a
            // `shift` inside the callback therefore moves the deque's `start`
            // without moving the walk's.
            //
            // ```js
            // var c = this.capacity, l = this.size, i = this.start, j = 0;
            // while (j < l) {
            //   callback.call(scope, this.items[i], j, this);
            //   i++; j++;
            //   if (i === c) i = 0;
            // }
            // ```
            "$forEach" => {
                let spec = for_each(op);
                let mut cursor = CursorState::open(&instance.deque);
                let mut seen = Vec::new();
                let mut fired = 0usize;
                let mut position = 0usize;

                loop {
                    let value = match cursor.step(&instance.deque) {
                        Step::Item(item) => item,
                        Step::Gap => json!({"$undefined": true}),
                        Step::Done => break,
                    };
                    let received = vec![value, json!(position)];

                    seen.push(Value::Array(received.clone()));

                    if fired < spec.limit {
                        if let Some(_args) = for_each_args(&spec, &received) {
                            fired += 1;

                            match spec.method.expect("for_each_args returned Some") {
                                "pop" => {
                                    instance.deque.pop();
                                }
                                "shift" => {
                                    instance.deque.shift();
                                }
                                "clear" => instance.deque.clear(),
                                other => {
                                    panic!("`{other}` is not a $forEach mutation for this module")
                                }
                            }
                        }
                    }

                    position += 1;
                }

                json!({ "seen": seen })
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.deque.size(),
            "capacity": instance.deque.capacity(),
            "start": instance.deque.start(),
            "items": slots(instance.typed, instance.deque.items(), Hole::Skipped),
            // `toArray`'s fast path is `items.slice(start, offset)`, which
            // preserves a hole; see `docs/modules/fixed-deque.md`.
            "toArray": slots(instance.typed, &instance.deque.to_array(), Hole::Skipped),
        })
    }
}

fn index(value: &Value) -> usize {
    value
        .as_u64()
        .expect("generated indices are non-negative integers") as usize
}

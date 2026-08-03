//! [`ModuleSpec`] for `circular-buffer`.
//!
//! The `fixed-deque` grammar with the two ops that differ replaced, which is
//! what the module is: upstream copies `FixedDeque.prototype` key by key and
//! then overwrites `push` and `unshift`. The helpers are imported from
//! `crate::modules::fixed_stack` and the shape from
//! `crate::modules::fixed_deque` for the same reason the ports share code —
//! two copies of a wrap is two places to get it wrong.
//!
//! # What this grammar reaches that the deque's cannot
//!
//! `push` and `unshift` never throw here, so a program never stops growing:
//! the interesting states are all *past* the capacity, where every insert
//! overwrites and `start` walks. With capacities of 1..=8 and 200-op programs,
//! a generated program wraps the ring tens of times.
//!
//! That makes one thing routine here that most other modules cannot reach at
//! all: **an insert that overwrites a slot an open cursor has not yet
//! reached.** Elements are read live while the geometry is frozen (DIV-PROJ-10), so
//! `$next` can yield a value that was not in the buffer when the walk started.
//! The `$iter`/`$next`/`push` interleaving in this alphabet produces it
//! constantly.
//!
//! The other is the return value of an overwriting insert: the size
//! *unchanged*, which is the only externally visible signal that an element was
//! dropped and which nothing upstream asserts.

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::backing::Backing;
use mnemonist_core::structures::circular_buffer::CircularBuffer;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::modules::fixed_stack::{optional, slots, step_value, store, Hole};
use crate::spec::{for_each, for_each_args, for_each_strategy, ModuleSpec, Op, FOR_EACH_MANY};

/// Largest value pushed. Above 255 so a `Uint8Array` buffer truncates.
const MAX_VALUE: u32 = 320;

/// Largest capacity generated. Small, so the ring wraps constantly.
const MAX_CAPACITY: u32 = 8;

/// Largest index a generated `get` asks for — past `MAX_CAPACITY`.
const MAX_INDEX: u32 = 11;

/// Mutations `$forEach`'s callback may perform, and how often. All three are
/// nullary and non-throwing, so firing on every step is always well formed.
const MUTATIONS: &[(&str, &str, u64)] = &[
    ("pop", "none", FOR_EACH_MANY),
    ("shift", "none", FOR_EACH_MANY),
    ("clear", "none", FOR_EACH_MANY),
];

pub struct CircularBufferSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/circular-buffer.txt"
);

pub struct Instance {
    buffer: CircularBuffer<Value>,
    typed: bool,
    cursor: Option<CursorState<CircularBuffer<Value>>>,
}

impl ModuleSpec for CircularBufferSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "circular-buffer"
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
            buffer: CircularBuffer::new(backing, capacity).expect("capacity is at least one"),
            typed,
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            // Neither insert can fail here, and both return the size UNCHANGED
            // when they overwrite. That return value is compared like any
            // other, which is what pins it.
            "push" => json!(instance.buffer.push(store(instance.typed, &op.args[0]))),
            "unshift" => json!(instance.buffer.unshift(store(instance.typed, &op.args[0]))),
            "pop" => optional(instance.buffer.pop()),
            "shift" => optional(instance.buffer.shift()),
            "peekFirst" => optional(instance.buffer.peek_first()),
            "peekLast" => optional(instance.buffer.peek_last()),
            "get" => optional(instance.buffer.get(index(&op.args[0]))),
            "clear" => {
                instance.buffer.clear();
                json!({"$undefined": true})
            }
            "$iter" => {
                instance.cursor = Some(CursorState::open(&instance.buffer));
                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some(cursor) => step_value(cursor.step(&instance.buffer)),
            },
            "$spread" => {
                let mut cursor = CursorState::open(&instance.buffer);
                let mut items = Vec::new();

                loop {
                    match cursor.step(&instance.buffer) {
                        Step::Item(item) => items.push(item),
                        Step::Gap => items.push(json!({"$undefined": true})),
                        Step::Done => break,
                    }
                }

                Value::Array(items)
            }
            // The pasted `FixedDeque` walk (see the module docs): `capacity`,
            // `size` and `start` are frozen together at entry via
            // [`CursorState`], `this.items` is read live, and a `shift` inside
            // the callback moves the buffer's `start` without moving the
            // walk's.
            "$forEach" => {
                let spec = for_each(op);
                let mut cursor = CursorState::open(&instance.buffer);
                let mut seen = Vec::new();
                let mut fired = 0usize;
                let mut position = 0usize;

                loop {
                    let value = match cursor.step(&instance.buffer) {
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
                                    instance.buffer.pop();
                                }
                                "shift" => {
                                    instance.buffer.shift();
                                }
                                "clear" => instance.buffer.clear(),
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
            "size": instance.buffer.size(),
            "capacity": instance.buffer.capacity(),
            "start": instance.buffer.start(),
            "items": slots(instance.typed, instance.buffer.items(), Hole::Skipped),
            "toArray": slots(instance.typed, &instance.buffer.to_array(), Hole::Skipped),
        })
    }
}

fn index(value: &Value) -> usize {
    value
        .as_u64()
        .expect("generated indices are non-negative integers") as usize
}

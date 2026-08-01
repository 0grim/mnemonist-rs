//! [`ModuleSpec`] for `stack`.
//!
//! # Grammar, and what it deliberately includes
//!
//! `sparse-set` was the first grammar to interleave iteration with mutation
//! (D-21). This is the first where **the mutation can rebind the backing array
//! out from under an open cursor**, which is the behaviour `clear()` has and
//! `pop()` does not:
//!
//! * `clear()` installs a *new* array, so a cursor opened beforehand keeps
//!   walking the old one and is completely unaffected;
//! * `pop()` shortens the *same* array, so a cursor opened beforehand reads
//!   past its new end and yields `undefined` — DESIGN.md §3.7's shrink window.
//!
//! Two mutations, one shortening a walk and the other not, and a port that
//! modelled `items` as a `Vec<T>` would have produced identical (wrong) answers
//! for both. That is the pair this grammar exists to compare, and it is why
//! `clear` carries real weight rather than being a token op.
//!
//! # Observable state
//!
//! `size`, `toArray()` **and `items`**. `items` is a public property upstream,
//! and observing it directly is what makes the rebinding checkable without
//! waiting for a cursor to notice. `size` and `items.length` are separate
//! quantities upstream (D-19); comparing both is how a port that silently
//! unified them would be caught.
//!
//! # Deliberately excluded: nothing
//!
//! Every method `stack.js` exposes is either in the alphabet or in the
//! observation set. `inspect` is the one exception, and it is not ported — a
//! Node display convenience with no upstream assertion.

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::stack::Stack;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Range the generator draws pushed values from.
///
/// Small enough that duplicates are frequent — a stack is not a set, and a port
/// that deduplicated would only be caught by repeats.
const VALUES: std::ops::Range<i64> = 0..48;

pub struct StackSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/stack.txt"
);

/// The stack plus the one cursor a program can have open over it.
pub struct Instance {
    stack: Stack<Value>,
    cursor: Option<CursorState<Stack<Value>>>,
}

impl ModuleSpec for StackSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "stack"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "items", "toArray"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // `new Stack()` takes no arguments; the whole state is built by the ops.
        Just(Vec::new()).boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            // Weighted towards `push`: it is the only op that grows the stack,
            // so a read-heavy mix would spend most of a program on an empty one.
            6 => VALUES.prop_map(|value| Op::new("push", vec![json!(value)])),
            3 => Just(Op::new("pop", vec![])),
            2 => Just(Op::new("peek", vec![])),
            // Heavier than it looks worth: `clear` is half of the pair this
            // grammar exists to compare, and it is only interesting when a
            // cursor is already open.
            2 => Just(Op::new("clear", vec![])),
            2 => Just(Op::new("$iter", vec![json!("values")])),
            4 => Just(Op::new("$next", vec![])),
            1 => Just(Op::new("$spread", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {
        Instance {
            stack: Stack::new(),
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            // `return ++this.size` — the new size, not the instance.
            "push" => json!(instance.stack.push(op.args[0].clone())),
            "pop" => optional(instance.stack.pop()),
            "peek" => optional(instance.stack.peek()),
            "clear" => {
                instance.stack.clear();
                json!({"$undefined": true})
            }
            "$iter" => {
                instance.cursor = Some(CursorState::open(&instance.stack));
                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some(cursor) => step_value(cursor.step(&instance.stack)),
            },
            // A *fresh* cursor every time — the collection-level
            // `Symbol.iterator` is a factory (D-07), and reusing the stored one
            // here would turn it into the identity and still pass every
            // non-interleaved program.
            "$spread" => {
                let mut cursor = CursorState::open(&instance.stack);
                let mut items = Vec::new();

                loop {
                    match cursor.step(&instance.stack) {
                        Step::Item(item) => items.push(item),
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
            "size": instance.stack.size(),
            "items": instance.stack.items(),
            "toArray": instance.stack.to_vec(),
        })
    }
}

/// `undefined` is a value JSON has no word for; the oracle spells it this way.
fn optional(value: Option<Value>) -> Value {
    value.unwrap_or_else(|| json!({"$undefined": true}))
}

/// A step, in the shape `fuzz/oracle.js` normalises both sides to.
fn step_value(step: Step<Value>) -> Value {
    match step {
        Step::Item(item) => json!({"done": false, "value": item}),
        Step::Gap => json!({"done": false, "value": {"$undefined": true}}),
        Step::Done => json!({"done": true, "value": {"$undefined": true}}),
    }
}

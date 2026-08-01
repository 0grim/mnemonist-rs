//! [`ModuleSpec`] for `queue`.
//!
//! # Grammar, and what it deliberately includes
//!
//! `stack`'s grammar exists to compare two mutations that treat the backing
//! array differently. This one adds a third case that neither previous module
//! had: **the cursor's end is live**.
//!
//! `Queue.prototype.values` re-reads `items.length` on every step where
//! `Stack.prototype.values` freezes it, and obliterator's `Iterator` has no
//! `done` flag, so a walk that has already reported `{done: true}` **resumes**
//! when the queue grows. The `$next`-after-exhaustion case is therefore not a
//! degenerate corner here, it is a behaviour, and the generator reaches it
//! constantly because `$next` outweighs `$iter` four to one.
//!
//! The compaction adds the other half: `++offset * 2 >= items.length` installs
//! a *new* array, so a cursor opened beforehand detaches onto the old one and
//! goes on yielding elements the queue has already handed out.
//!
//! # Observable state
//!
//! `size`, `offset`, `items` **and `toArray()`**. `offset` and `items` are both
//! public properties upstream, and they are what makes the compaction visible
//! at all: `toArray()` alone cannot tell a compacted queue from an uncompacted
//! one holding the same elements. That is precisely the distinction a port
//! could get wrong for a whole program without anything noticing.
//!
//! # Deliberately excluded: nothing.

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::queue::Queue;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{for_each, for_each_args, for_each_strategy, ModuleSpec, Op, FOR_EACH_MANY};

/// Range the generator draws enqueued values from.
const VALUES: std::ops::Range<i64> = 0..48;

pub struct QueueSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/queue.txt"
);

/// The queue plus the one cursor a program can have open over it.
pub struct Instance {
    queue: Queue<Value>,
    cursor: Option<CursorState<Queue<Value>>>,
}

impl ModuleSpec for QueueSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "queue"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "offset", "items", "toArray"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        Just(Vec::new()).boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            6 => VALUES.prop_map(|value| Op::new("enqueue", vec![json!(value)])),
            // Heavy, because the compaction only fires on a dequeue and the
            // schedule (`++offset * 2 >= items.length`) needs several in a row
            // to be interesting.
            4 => Just(Op::new("dequeue", vec![])),
            2 => Just(Op::new("peek", vec![])),
            1 => Just(Op::new("clear", vec![])),
            2 => Just(Op::new("$iter", vec![json!("values")])),
            4 => Just(Op::new("$next", vec![])),
            1 => Just(Op::new("$spread", vec![])),
            2 => for_each_strategy(MUTATIONS),
        ]
        .boxed()
    }

    fn construct(&self, _args: &[Value]) -> Self::Instance {
        Instance {
            queue: Queue::new(),
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "enqueue" => json!(instance.queue.enqueue(op.args[0].clone())),
            "dequeue" => optional(instance.queue.dequeue()),
            "peek" => optional(instance.queue.peek()),
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
            // Upstream's own loop, which freezes the start and the bound but
            // re-reads `this.items` every step:
            //
            // ```js
            // for (var i = this.offset, j = 0, l = this.items.length; i < l; i++, j++)
            //   callback.call(scope, this.items[i], j, this);
            // ```
            //
            // The interesting program is a callback that dequeues far enough
            // to COMPACT: `items` is rebound to a shorter array while `i` and
            // `l` still refer to the old one, so the remaining reads run off
            // the end and yield `undefined`. That is the program the boundary
            // spec pins by hand and this op generates by the thousand.
            "$forEach" => {
                let spec = for_each(op);
                let start = instance.queue.offset();
                let frozen = instance.queue.items_len();
                let mut seen = Vec::new();
                let mut fired = 0usize;

                for (ordinal, index) in (start..frozen).enumerate() {
                    // Live: `this.items` may have been rebound by the previous
                    // step's callback.
                    let value = instance
                        .queue
                        .slot(index)
                        .unwrap_or_else(|| json!({"$undefined": true}));
                    let received = vec![value, json!(ordinal)];

                    seen.push(Value::Array(received.clone()));

                    if fired < spec.limit {
                        if let Some(args) = for_each_args(&spec, &received) {
                            fired += 1;

                            match spec.method.expect("for_each_args returned Some") {
                                "enqueue" => {
                                    instance.queue.enqueue(args[0].clone());
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
                }

                json!({ "seen": seen })
            }
            "$spread" => {
                let mut cursor = CursorState::open(&instance.queue);
                let mut items = Vec::new();

                loop {
                    match cursor.step(&instance.queue) {
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
            "size": instance.queue.size(),
            "offset": instance.queue.offset(),
            "items": instance.queue.items(),
            "toArray": instance.queue.to_vec(),
        })
    }
}

/// What the callback may do to the queue, and how often.
///
/// All three are safe uncapped: `l` is captured before the first step. The
/// `dequeue` row is the one that matters -- enough of them compact, and the
/// compaction is what rebinds `items` underneath the frozen indices.
const MUTATIONS: &[(&str, &str, u64)] = &[
    ("dequeue", "none", FOR_EACH_MANY),
    ("enqueue", "arg0", FOR_EACH_MANY),
    ("clear", "none", FOR_EACH_MANY),
];

fn optional(value: Option<Value>) -> Value {
    value.unwrap_or_else(|| json!({"$undefined": true}))
}

fn step_value(step: Step<Value>) -> Value {
    match step {
        Step::Item(item) => json!({"done": false, "value": item}),
        Step::Gap => json!({"done": false, "value": {"$undefined": true}}),
        Step::Done => json!({"done": true, "value": {"$undefined": true}}),
    }
}

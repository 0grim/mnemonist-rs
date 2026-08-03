//! [`ModuleSpec`] for `fixed-stack`.
//!
//! # The op this grammar adds: `$forEach` with a mutating callback
//!
//! Every grammar before this one drives iteration through `$iter`/`$next`,
//! which is enough for a *cursor*. It is not enough for `forEach`, and the
//! difference is not cosmetic: upstream's `forEach` freezes only its loop bound
//! and re-reads `this.items` on every step, so a callback that mutates the
//! stack is visible to the reads after it. Nothing in the alphabet could
//! express that, and PORTBUG-1 — the port's own worst bug — was reachable only
//! through a mutating `forEach` and survived 2.94 M operations because no
//! grammar had one.
//!
//! `$forEach` takes `[method, rule, limit]` (`crate::spec::ForEach`): the
//! method the callback calls back into (or `null` for a plain walk), how to
//! build its arguments from the callback's own — always `"none"` here, since
//! both mutations are nullary — and how many leading steps may fire it. Both
//! sides run their own `forEach` to completion, recording each callback's
//! first two arguments `(value, index)` per call. The mutations are the
//! nullary ones that cannot throw — `pop` and `clear` — so a generated
//! program is always well formed.
//!
//! It is also the only op that can see **BUG-FIXED-STACK-1**: `forEach`'s bound is
//! `this.items.length` and every other method's is `this.size`, so an
//! under-full stack hands the callback its unused slots first. A port that
//! walked `size` would pass every other op in this alphabet.
//!
//! # Observable state
//!
//! `size`, `capacity`, `items` **and** `toArray()`. `items` is a public
//! property upstream, and observing it is what makes the debris that `pop` and
//! `clear` leave behind checkable directly rather than only through a later
//! `forEach`.
//!
//! ## The one encoding subtlety
//!
//! A `new Array(n)` slot that was never written is a **hole**. `fuzz/oracle.js`
//! used to encode an array with `value.map(encode)`, which SKIPS holes and
//! leaves them holes for `JSON.stringify` to render as `null` — and this side
//! matched that with `Value::Null`. The oracle now walks arrays by index
//! instead (`heap` needed it: a comparator that shrinks the array mid-sift
//! makes the sift write an explicit `undefined` past the array's old end, and
//! that is not distinguishable from a hole through any API a stack, deque or
//! buffer exposes — `items[i]` reads `undefined` either way). Both now encode
//! as `{"$undefined": true}`, so the Rust side follows: a `None` slot of an
//! `Array`-backed stack is `{"$undefined": true}`, never `null`.
//!
//! A `Uint8Array`-backed stack has no holes — it is zero filled — so its
//! `items` goes out as the oracle's `{"$typed": …}` envelope instead.
//!
//! # What the ctor covers, and one thing the Rust side has to model
//!
//! Both `Array` and `Uint8Array`, because [`Backing`] is the whole reason the
//! two are not interchangeable: an oversized store grows the first and is
//! dropped by the second. Capacities run 1..=8, so a 200-op program spends most
//! of its time at the capacity boundary where `push` throws.
//!
//! Values run to 320, past a `Uint8Array` element's range. Core does **not**
//! model element coercion — that is a JS-value semantic living at the bridge
//! (`crate::array_class`) — so this spec applies the `% 256` itself. It is
//! therefore a *model* of the bridge's coercion rather than the bridge's own
//! code, and the real coercion is covered by `test/fixed-stack.js` and by the
//! differential probes recorded in `docs/modules/fixed-stack.md`.

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::backing::Backing;
use mnemonist_core::structures::fixed_stack::FixedStack;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{for_each, for_each_args, for_each_strategy, ModuleSpec, Op, FOR_EACH_MANY};

/// Largest value pushed. Above 255 so a `Uint8Array` stack truncates.
const MAX_VALUE: u32 = 320;

/// Largest capacity generated. Small on purpose: `push` must hit the ceiling
/// often, and `forEach`'s capacity-vs-size bound only differs below it.
const MAX_CAPACITY: u32 = 8;

/// Mutations `$forEach`'s callback may perform, and how often. Both are
/// nullary and non-throwing, so a limit-many firing is always well formed —
/// neither shrinks the frozen bound below zero, `pop` on an empty stack is a
/// no-op, and `clear` idempotently re-empties.
const MUTATIONS: &[(&str, &str, u64)] = &[
    ("pop", "none", FOR_EACH_MANY),
    ("clear", "none", FOR_EACH_MANY),
];

pub struct FixedStackSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/fixed-stack.txt"
);

/// The stack, the class it was built with, and the one cursor a program can
/// have open over it.
pub struct Instance {
    stack: FixedStack<Value>,
    /// Whether the backing class truncates element stores, which core does not
    /// model and this spec therefore has to.
    typed: bool,
    cursor: Option<CursorState<FixedStack<Value>>>,
}

impl ModuleSpec for FixedStackSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "fixed-stack"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "capacity", "items", "toArray"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        (
            // `{"$global": …}` is resolved to the real constructor by
            // fuzz/oracle.js; JSON has no way to carry one directly.
            prop::sample::select(&["Array", "Uint8Array"][..]),
            1u32..=MAX_CAPACITY,
        )
            .prop_map(|(class, capacity)| vec![json!({ "$global": class }), json!(capacity)])
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            // Weighted towards `push`: it is the only op that grows the stack,
            // and everything else is more interesting on a non-empty one.
            6 => (0u32..MAX_VALUE).prop_map(|value| Op::new("push", vec![json!(value)])),
            3 => Just(Op::new("pop", vec![])),
            2 => Just(Op::new("peek", vec![])),
            2 => Just(Op::new("clear", vec![])),
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
            stack: FixedStack::new(backing, capacity).expect("capacity is at least one"),
            typed,
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "push" => {
                let value = store(instance.typed, &op.args[0]);

                match instance.stack.push(value) {
                    Ok(size) => json!(size),
                    Err(error) => json!({ "$throw": error.to_string() }),
                }
            }
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
            // `Symbol.iterator` is a factory (DIV-STACK-2).
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
            // Upstream's own loop, whose bound is frozen (BUG-FIXED-STACK-1):
            //
            // ```js
            // for (var i = 0, l = this.items.length; i < l; i++)
            //   callback.call(scope, this.items[l - i - 1], i, this);
            // ```
            "$forEach" => {
                let spec = for_each(op);
                let bound = instance.stack.items_len();
                let mut seen = Vec::new();
                let mut fired = 0usize;

                for index in 0..bound {
                    let value = instance
                        .stack
                        .lifo_slot(bound, index)
                        .unwrap_or_else(|| json!({"$undefined": true}));
                    let received = vec![value, json!(index)];

                    seen.push(Value::Array(received.clone()));

                    if fired < spec.limit {
                        if let Some(_args) = for_each_args(&spec, &received) {
                            fired += 1;

                            match spec.method.expect("for_each_args returned Some") {
                                "pop" => {
                                    instance.stack.pop();
                                }
                                "clear" => instance.stack.clear(),
                                other => {
                                    panic!("`{other}` is not a $forEach mutation for this module")
                                }
                            }
                        }
                    }
                }

                json!({ "seen": seen })
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.stack.size(),
            "capacity": instance.stack.capacity(),
            "items": slots(instance.typed, instance.stack.items(), Hole::Skipped),
            // `toArray` writes every index explicitly, so a missing slot is an
            // own `undefined` property rather than a hole.
            "toArray": slots(instance.typed, &instance.stack.to_array(), Hole::Written),
        })
    }
}

/// Whether a missing slot was left as a hole or written as `undefined`.
///
/// The two are still genuinely different mutations on the JS side — a
/// `new Array(n)` slot nothing has assigned to, versus a slot an explicit
/// `array[i] = undefined` touched (upstream's `FixedStack.prototype.toArray`
/// writes every index explicitly; `FixedDeque.prototype.toArray`'s fast path
/// is `items.slice(...)`, which preserves a hole as a hole). But
/// `fuzz/oracle.js` no longer tells them apart: it walks arrays by index
/// rather than by `.map()` (which used to skip holes), so both read back
/// through the same `{"$undefined": true}` envelope. The variant is kept at
/// each call site as documentation of which one is in play; it no longer
/// changes what gets encoded.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hole {
    /// A `new Array(n)` slot nothing has assigned.
    Skipped,
    /// A slot an explicit assignment set to `undefined`.
    Written,
}

/// One array of slots, encoded exactly as `fuzz/oracle.js` encodes the JS value
/// it corresponds to.
pub(crate) fn slots(typed: bool, slots: &[Option<Value>], _hole: Hole) -> Value {
    if typed {
        // Zero filled, so a missing slot can only be a read past the end, which
        // a fresh typed array renders as its zero.
        return json!({
            "$typed": "Uint8Array",
            "values": slots
                .iter()
                .map(|slot| slot.clone().unwrap_or_else(|| json!(0)))
                .collect::<Vec<Value>>(),
        });
    }

    // Both provenances above now encode identically: `fuzz/oracle.js` walks
    // arrays by index, so a hole and an explicit `undefined` are the same
    // `{"$undefined": true}` on the wire. See the `Hole` doc comment.
    Value::Array(
        slots
            .iter()
            .map(|slot| slot.clone().unwrap_or_else(|| json!({"$undefined": true})))
            .collect(),
    )
}

/// The element store a `Uint8Array` performs and a plain `Array` does not.
///
/// Modelled here rather than in core deliberately; see the module docs.
pub(crate) fn store(typed: bool, value: &Value) -> Value {
    let raw = value
        .as_u64()
        .expect("generated values are non-negative integers");

    if typed {
        return json!(raw % 256);
    }

    json!(raw)
}

/// `undefined` is a value JSON has no word for; the oracle spells it this way.
pub(crate) fn optional(value: Option<Value>) -> Value {
    value.unwrap_or_else(|| json!({"$undefined": true}))
}

/// A step, in the shape `fuzz/oracle.js` normalises both sides to.
pub(crate) fn step_value(step: Step<Value>) -> Value {
    match step {
        Step::Item(item) => json!({"done": false, "value": item}),
        Step::Gap => json!({"done": false, "value": {"$undefined": true}}),
        Step::Done => json!({"done": true, "value": {"$undefined": true}}),
    }
}

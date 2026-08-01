//! [`ModuleSpec`] for `bit-vector`.
//!
//! Shares the step and word encoders with
//! [`crate::modules::bit_set`], because upstream shares the methods they
//! encode.
//!
//! # What this grammar adds over `bit-set`'s
//!
//! The whole capacity machinery, and it is the interaction between `push`,
//! `pop` and `resize` that matters rather than any of them alone:
//!
//! * `push`/`pop` are weighted up because upstream's three defects there only
//!   appear once a `pop` has released a slot that a later `push` walks back
//!   over — `push(0)` does not clear it, `push(1)` counts it again, and `pop`
//!   decremented neither `size` nor the bit.
//! * `resize`/`reallocate`/`grow` all move `length` and `capacity` apart, which
//!   is what makes `set(length)` land in the capacity region and what makes a
//!   length of 0 over a non-empty array iterate 32 bits.
//! * `set` is the only op that throws, and it does so on `length < index`,
//!   which the generator reaches constantly because indices overshoot `length`
//!   by 64.
//!
//! # Deliberately excluded: custom growth policies
//!
//! Upstream's policy is a JS function, and a generated program is JSON. The
//! default policy is therefore the only one fuzzed, and the two throws in
//! `applyPolicy` are consequently **unreachable** from this grammar — the
//! default policy is strictly increasing, so it never returns a value less than
//! or equal to the current capacity. Both are covered by native tests in
//! `mnemonist-core` instead. Stated here rather than left implicit, because a
//! silently narrowed grammar reads as "we covered everything" when it did not.

use mnemonist_core::structures::bit_vector::BitVector;
use mnemonist_core::structures::bits::{bits_in_word, BitEntries, BitWalk};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::modules::bit_set::{step_entry, step_value, typed_array};
use crate::spec::{
    for_each, for_each_args, for_each_index, for_each_strategy, ModuleSpec, Op, FOR_EACH_MANY,
};

/// Largest initial length the generator builds.
const MAX_LENGTH: u32 = 200;

/// How far past `length` a generated index may reach.
const OVERSHOOT: u32 = 64;

/// Largest capacity a generated `grow`/`resize`/`reallocate` asks for.
const MAX_EXTENT: u32 = 512;

pub struct BitVectorSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/bit-vector.txt"
);

pub struct Instance {
    vector: BitVector,
    cursor: Option<Cursor>,
}

enum Cursor {
    Values(BitWalk),
    Entries(BitEntries),
}

impl ModuleSpec for BitVectorSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "bit-vector"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "length", "capacity", "array", "toJSON"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        (0u32..=MAX_LENGTH)
            .prop_map(|length| vec![json!(length)])
            .boxed()
    }

    fn op_strategy(&self, ctor: &[Value]) -> BoxedStrategy<Op> {
        let length = ctor[0].as_u64().expect("ctor arg 0 is the length") as u32;
        let index = 0u32..(length + OVERSHOOT);
        let extent = 0u32..MAX_EXTENT;

        prop_oneof![
            3 => index.clone().prop_map(|i| Op::new("set", vec![json!(i)])),
            2 => index.clone().prop_map(|i| Op::new("set", vec![json!(i), json!(0)])),
            3 => index.clone().prop_map(|i| Op::new("reset", vec![json!(i)])),
            2 => index.clone().prop_map(|i| Op::new("flip", vec![json!(i)])),
            2 => index.clone().prop_map(|i| Op::new("get", vec![json!(i)])),
            1 => index.clone().prop_map(|i| Op::new("test", vec![json!(i)])),
            2 => index.clone().prop_map(|i| Op::new("rank", vec![json!(i)])),
            2 => index.prop_map(|r| Op::new("select", vec![json!(r)])),
            // The push/pop pair carries this module's own defects, so it gets
            // the weight. A 1 and a 0 push are separate ops because only the
            // former touches `size` and only the latter leaves a stale bit.
            3 => Just(Op::new("push", vec![json!(1)])),
            3 => Just(Op::new("push", vec![json!(0)])),
            3 => Just(Op::new("pop", vec![])),
            2 => extent.clone().prop_map(|l| Op::new("resize", vec![json!(l)])),
            2 => extent.clone().prop_map(|c| Op::new("reallocate", vec![json!(c)])),
            1 => extent.prop_map(|c| Op::new("grow", vec![json!(c)])),
            1 => Just(Op::new("grow", vec![])),
            1 => Just(Op::new("$iter", vec![json!("values")])),
            1 => Just(Op::new("$iter", vec![json!("entries")])),
            3 => Just(Op::new("$next", vec![])),
            1 => Just(Op::new("$spread", vec![])),
            2 => for_each_strategy(MUTATIONS),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        Instance {
            vector: BitVector::new(args[0].as_u64().expect("ctor arg 0 is the length") as usize),
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            "set" => {
                let value = op.args.get(1).is_none();

                match instance.vector.set(index(op), value) {
                    Ok(()) => json!({"$self": true}),
                    // The only throw this grammar reaches. Compared by its full
                    // message, which is upstream's verbatim.
                    Err(error) => json!({"$throw": error.to_string()}),
                }
            }
            "reset" => {
                instance.vector.reset(index(op));
                json!({"$self": true})
            }
            "flip" => {
                instance.vector.flip(index(op));
                json!({"$self": true})
            }
            "get" => match instance.vector.get(index(op)) {
                Some(bit) => json!(bit),
                None => json!({"$undefined": true}),
            },
            "test" => json!(instance.vector.test(index(op))),
            "rank" => json!(instance.vector.rank(index(op))),
            "select" => match instance.vector.select(index(op)) {
                Some(position) => json!(position),
                None => json!({"$undefined": true}),
            },
            "push" => {
                let value = op.args[0].as_u64() != Some(0);

                match instance.vector.push(value) {
                    Ok(length) => json!(length),
                    Err(error) => json!({"$throw": error.to_string()}),
                }
            }
            "pop" => match instance.vector.pop() {
                Some(bit) => json!(bit),
                None => json!({"$undefined": true}),
            },
            "resize" => {
                instance.vector.resize(extent(op));
                json!({"$self": true})
            }
            "reallocate" => {
                instance.vector.reallocate(extent(op));
                json!({"$self": true})
            }
            "grow" => match instance.vector.grow(op.args.first().map(|_| extent(op))) {
                Ok(()) => json!({"$self": true}),
                Err(error) => json!({"$throw": error.to_string()}),
            },
            "$iter" => {
                instance.cursor = Some(match op.args[0].as_str() {
                    Some("entries") => Cursor::Entries(instance.vector.entries()),
                    _ => Cursor::Values(instance.vector.values()),
                });
                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some(Cursor::Values(walk)) => step_value(walk.step()),
                Some(Cursor::Entries(entries)) => step_entry(entries.0.step_entry()),
            },
            // Upstream's own loop, which snapshots each word and freezes both
            // bounds:
            //
            // ```js
            // var length = this.length, byte, bit, b = 32;
            // for (var i = 0, l = this.array.length; i < l; i++) {
            //   byte = this.array[i];
            //   if (i === l - 1) b = length % 32 || 32;
            //   for (var j = 0; j < b; j++) {
            //     bit = (byte >> j) & 1;
            //     callback.call(scope, bit, i * 32 + j);
            //   }
            // }
            // ```
            //
            // Three things are captured -- `length`, `l`, and `byte` for the
            // duration of the inner loop -- and only `this.array[i]` is a live
            // read. So a callback that writes to the word being walked is
            // invisible for the rest of that word and visible in the next one,
            // and a callback that pushes does not extend the walk.
            "$forEach" => {
                let spec = for_each(op);
                let word_count = instance.vector.words().word_count();
                let length = instance.vector.length();
                let mut seen = Vec::new();
                let mut fired = 0usize;

                for index in 0..word_count {
                    // Live, exactly as `this.array[i]` is -- and re-read here
                    // rather than lifted, because the callback may have
                    // rewritten it.
                    let word = instance.vector.words().word(index).unwrap_or(0);
                    let bits = bits_in_word(index, word_count, length);

                    for offset in 0..bits {
                        let bit = ((word as i32) >> offset) & 1;
                        let position = index * 32 + offset;
                        let received = vec![json!(bit), json!(position)];

                        seen.push(Value::Array(received.clone()));

                        if fired < spec.limit {
                            if let Some(args) = for_each_args(&spec, &received) {
                                fired += 1;
                                match spec.method.expect("for_each_args returned Some") {
                                    "set" => {
                                        // `set` past the capacity throws
                                        // upstream, and the oracle reports it
                                        // alongside the steps already taken.
                                        if let Err(error) = instance
                                            .vector
                                            .set(for_each_index(&spec, args[0]) as i64, true)
                                        {
                                            return json!({
                                                "seen": seen,
                                                "$throw": error.to_string(),
                                            });
                                        }
                                    }
                                    "reset" => {
                                        instance.vector.reset(for_each_index(&spec, args[0]) as i64)
                                    }
                                    "flip" => {
                                        instance.vector.flip(for_each_index(&spec, args[0]) as i64)
                                    }
                                    "push" => {
                                        if let Err(error) = instance.vector.push(true) {
                                            return json!({
                                                "seen": seen,
                                                "$throw": error.to_string(),
                                            });
                                        }
                                    }
                                    "pop" => {
                                        instance.vector.pop();
                                    }
                                    other => panic!(
                                        "`{other}` is not a $forEach mutation for this module"
                                    ),
                                }
                            }
                        }
                    }
                }

                json!({ "seen": seen })
            }
            "$spread" => Value::Array(instance.vector.values().map(|bit| json!(bit)).collect()),
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.vector.size(),
            "length": instance.vector.length(),
            "capacity": instance.vector.capacity(),
            "array": typed_array(&instance.vector.words().to_vec()),
            "toJSON": instance.vector.to_json(),
        })
    }
}

/// What the callback may do to the vector, and how often.
///
/// `push` grows the vector, but the outer bound is `this.array.length` read
/// once, so the walk cannot be extended and an uncapped push still terminates.
/// It is capped at four anyway: a push per bit over a 400-bit vector is
/// hundreds of reallocations per case, and the throughput matters more than
/// the extra depth.
const MUTATIONS: &[(&str, &str, u64)] = &[
    ("set", "arg1", FOR_EACH_MANY),
    ("reset", "arg1", FOR_EACH_MANY),
    ("flip", "arg1", FOR_EACH_MANY),
    ("push", "none", 4),
    ("pop", "none", FOR_EACH_MANY),
];

fn index(op: &Op) -> i64 {
    op.args[0].as_i64().expect("generated indices are integers")
}

fn extent(op: &Op) -> usize {
    op.args[0]
        .as_u64()
        .expect("generated extents are non-negative integers") as usize
}

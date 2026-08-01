//! [`ModuleSpec`] for `bit-set`.
//!
//! # Grammar, and what it is aimed at
//!
//! Three things in this module are only reachable by a generator that is
//! deliberately careless about ranges and deliberately fond of `reset`:
//!
//! * **B-13.** `reset` on an already-clear bit decrements `size` whenever bit
//!   31 of the word is set. So `reset` carries weight 3, and indices reach past
//!   `length`, which is how bit 31 of a word gets set in the first place on a
//!   set whose length is not a multiple of 32.
//! * **B-14.** `select` loses 32 positions per skipped all-zero word, so
//!   lengths run to 400 — thirteen words — and stay sparse enough that empty
//!   words are common.
//! * **B-19.** An index in `length .. 32 * ceil(length / 32)` is accepted and
//!   then invisible to `rank`, `select` and iteration. `MAX_INDEX` overshoots
//!   `length` by 64 precisely to generate those.
//!
//! # Observable state
//!
//! `size`, `length`, **`array`** and `toJSON()`. `array` is the point: `size`
//! alone would agree in plenty of programs where the words had diverged, and
//! comparing the backing store word for word after every operation is what
//! makes the signed/unsigned comparison in `reset` checkable directly rather
//! than only through its eventual effect on `rank`.
//!
//! # Cursor lifecycle
//!
//! `$iter` alternates between `values` and `entries` — the two walks share an
//! implementation here and are separate closures upstream, so comparing only
//! one would leave the other unchecked. `clear()` is in the alphabet *because*
//! it interacts with an open cursor: it replaces `this.array`, and a cursor
//! opened beforehand must keep reading the pre-clear words.

use mnemonist_core::cursor::Step;
use mnemonist_core::structures::bit_set::BitSet;
use mnemonist_core::structures::bits::{BitEntries, BitWalk};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Largest set the generator builds. Thirteen words, so empty words between
/// set bits are routine and B-14 is reachable.
const MAX_LENGTH: u32 = 400;

/// How far past `length` a generated index may reach.
///
/// 64 covers both the "inside the last word but past `length`" band that B-19
/// lives in and the fully out-of-range band beyond it.
const OVERSHOOT: u32 = 64;

pub struct BitSetSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/bit-set.txt"
);

/// The set plus the one cursor a program can have open over it.
pub struct Instance {
    set: BitSet,
    cursor: Option<Cursor>,
}

/// `values()` and `entries()` are separate closures upstream and yield
/// different shapes, so the stored cursor has to remember which it is.
enum Cursor {
    Values(BitWalk),
    Entries(BitEntries),
}

impl ModuleSpec for BitSetSpec {
    type Instance = Instance;

    fn module(&self) -> &'static str {
        "bit-set"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["size", "length", "array", "toJSON"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        // 0 included: `new BitSet(0)` allocates no words at all, which is the
        // degenerate end of every guard in the module.
        (0u32..=MAX_LENGTH)
            .prop_map(|length| vec![json!(length)])
            .boxed()
    }

    fn op_strategy(&self, ctor: &[Value]) -> BoxedStrategy<Op> {
        let length = ctor[0].as_u64().expect("ctor arg 0 is the length") as u32;
        let index = 0u32..(length + OVERSHOOT);
        let rank = 0u32..(length + OVERSHOOT);

        prop_oneof![
            4 => index.clone().prop_map(|i| Op::new("set", vec![json!(i)])),
            2 => index.clone().prop_map(|i| Op::new("set", vec![json!(i), json!(0)])),
            // Weighted up: `reset` is where B-13 lives, and it only misbehaves
            // on a bit that is ALREADY clear -- which a low weight would make
            // rare rather than routine.
            3 => index.clone().prop_map(|i| Op::new("reset", vec![json!(i)])),
            2 => index.clone().prop_map(|i| Op::new("flip", vec![json!(i)])),
            2 => index.clone().prop_map(|i| Op::new("get", vec![json!(i)])),
            1 => index.clone().prop_map(|i| Op::new("test", vec![json!(i)])),
            2 => index.prop_map(|i| Op::new("rank", vec![json!(i)])),
            2 => rank.prop_map(|r| Op::new("select", vec![json!(r)])),
            1 => Just(Op::new("clear", vec![])),
            1 => Just(Op::new("$iter", vec![json!("values")])),
            1 => Just(Op::new("$iter", vec![json!("entries")])),
            3 => Just(Op::new("$next", vec![])),
            1 => Just(Op::new("$spread", vec![])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        Instance {
            set: BitSet::new(args[0].as_u64().expect("ctor arg 0 is the length") as usize),
            cursor: None,
        }
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            // `set`, `reset` and `flip` all return `this` upstream.
            "set" => {
                // `value === 0 || value === false`; the generator only ever
                // emits a literal 0 for the clearing form.
                let value = op.args.get(1).is_none();
                instance.set.set_to(index(op), value);
                json!({"$self": true})
            }
            "reset" => {
                instance.set.reset(index(op));
                json!({"$self": true})
            }
            "flip" => {
                instance.set.flip(index(op));
                json!({"$self": true})
            }
            "get" => json!(instance.set.get(index(op))),
            "test" => json!(instance.set.test(index(op))),
            "rank" => json!(instance.set.rank(index(op))),
            "select" => match instance.set.select(index(op)) {
                Some(position) => json!(position),
                // Upstream falls out of the loop with no `return`.
                None => json!({"$undefined": true}),
            },
            "clear" => {
                instance.set.clear();
                json!({"$undefined": true})
            }
            "$iter" => {
                instance.cursor = Some(match op.args[0].as_str() {
                    Some("entries") => Cursor::Entries(instance.set.entries()),
                    _ => Cursor::Values(instance.set.values()),
                });
                json!({"$iterator": true})
            }
            "$next" => match instance.cursor.as_mut() {
                None => json!({"$noIterator": true}),
                Some(Cursor::Values(walk)) => step_value(walk.step()),
                Some(Cursor::Entries(entries)) => step_entry(entries.0.step_entry()),
            },
            // `Array.from(set)` goes through the COLLECTION's Symbol.iterator,
            // which upstream aliases to `values` -- so a fresh cursor each time.
            "$spread" => Value::Array(instance.set.values().map(|bit| json!(bit)).collect()),
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "size": instance.set.size(),
            "length": instance.set.length(),
            "array": typed_array(&instance.set.to_json()),
            "toJSON": instance.set.to_json(),
        })
    }
}

fn index(op: &Op) -> i64 {
    op.args[0].as_i64().expect("generated indices are integers")
}

/// A `values()` step, in the shape `fuzz/oracle.js` normalises to.
pub fn step_value(step: Step<u32>) -> Value {
    match step {
        Step::Item(bit) => json!({"done": false, "value": bit}),
        // Unreachable for a bit walk -- the frozen array is kept alive by the
        // cursor -- but encoded rather than panicked on, so that if it ever
        // does happen it is reported as a divergence instead of a crash.
        Step::Gap => json!({"done": false, "value": {"$undefined": true}}),
        Step::Done => json!({"done": true, "value": {"$undefined": true}}),
    }
}

/// An `entries()` step: `[index, bit]`.
pub fn step_entry(step: Step<(usize, u32)>) -> Value {
    match step {
        Step::Item((index, bit)) => json!({"done": false, "value": [index, bit]}),
        Step::Gap => json!({"done": false, "value": {"$undefined": true}}),
        Step::Done => json!({"done": true, "value": {"$undefined": true}}),
    }
}

/// Encode a word array exactly as the oracle encodes a `Uint32Array`.
pub fn typed_array(words: &[u32]) -> Value {
    json!({ "$typed": "Uint32Array", "values": words })
}

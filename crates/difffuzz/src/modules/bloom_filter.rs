//! [`ModuleSpec`] for `bloom-filter`.
//!
//! # Grammar
//!
//! Ops are `add`, `test`, `clear` and `toJSON`, which is **every** method
//! upstream defines apart from the static `from` and the unported `inspect`.
//! `data`, `capacity`, `errorRate` and `hashFunctions` are in the observable
//! state instead of the op alphabet, so they are compared after every step
//! rather than only when the generator picks them — and `data` is the whole
//! point of the module, so a divergence in a single bit shows up on the
//! operation that caused it rather than several ops later.
//!
//! `clear` is in the alphabet rather than the observations because it is the
//! one method that *mutates*: upstream's `clear` re-derives `hashFunctions` and
//! reallocates `data` from `capacity` and `errorRate`, so calling it mid-program
//! is a real state transition and putting it in `observations` would silently
//! wipe the filter before every comparison.
//!
//! # Items include non-strings, on purpose
//!
//! `add` and `test` take a string, a number or a boolean. The non-strings are
//! not noise: upstream's `stringToByteArray` reads `.length`, gets `undefined`,
//! and produces an **empty** `Uint16Array`, so every one of them hashes as the
//! empty sequence and they all collide with each other and with `''`. That is
//! B-98, and it is only fuzzable if the grammar can express a non-string item.
//! `null` and `undefined` are excluded: upstream throws a `TypeError` from the
//! property read, and the oracle compares thrown messages verbatim, which would
//! turn an engine-wording difference into a false divergence.
//!
//! The string alphabet includes `U+0000` — equal to the value `murmurhash3`'s
//! tail treats as absent — and three characters above `U+00FF`, where the
//! hash's habit of reading 16-bit elements as if they were bytes makes distinct
//! inputs overlap.
//!
//! # What the grammar deliberately excludes
//!
//! **`errorRate >= 1`.** `Math.log` of it is non-negative, `bits` goes
//! negative, and for a large enough capacity `new Uint8Array(-59)` throws from
//! the *constructor* — which reaches the oracle's `init` rather than an op, and
//! an `init` failure is apparatus failure by protocol, aborting the campaign
//! instead of reporting anything. That is B-99, it is documented in
//! `docs/modules/bloom-filter.md` and pinned by a native test, so fuzzing it
//! would only re-report a known decision (DESIGN.md §3.7).
//!
//! Every `errorRate` below 1 is safe: `ln(x) < 0` makes `bits` positive, so the
//! allocation length is never negative. The **zero-hash-function** region — the
//! one where `test` returns `true` for everything, B-97 — is *not* excluded and
//! is reached routinely, because an `errorRate` near 1 gets there without
//! throwing.

use mnemonist_core::structures::bloom_filter::BloomFilter;
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Symbols the string generator draws from.
const ALPHABET: &[char] = &[
    'a', 'b', 'c', 'A', '\u{0}', '\u{1}', '\u{100}', '\u{141}', '\u{201}',
];

/// Largest item the generator builds.
const MAX_ITEM: usize = 20;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/bloom-filter.txt"
);

pub struct BloomFilterSpec;

/// One `add`/`test` argument: a string, a number or a boolean.
fn item_strategy() -> BoxedStrategy<Value> {
    prop_oneof![
        // Weighted towards strings: that is what a caller passes, and the
        // non-strings all collapse onto one hash (B-98) so more of them buys
        // nothing.
        8 => proptest::collection::vec(proptest::sample::select(ALPHABET), 0..=MAX_ITEM)
            .prop_map(|chars| Value::String(chars.into_iter().collect())),
        1 => (-1000i64..1000).prop_map(|n| json!(n)),
        1 => any::<bool>().prop_map(|b| json!(b)),
    ]
    .boxed()
}

/// The UTF-16 code units upstream's `stringToByteArray` would produce.
///
/// A non-string has no `length`, so `new Uint16Array(undefined)` is empty and
/// the loop never runs. B-98, reproduced on this side of the comparison too —
/// getting it "right" here would report a divergence that is upstream's
/// behaviour, not the port's.
fn item_units(value: &Value) -> Vec<u16> {
    match value {
        Value::String(text) => text.encode_utf16().collect(),
        _ => Vec::new(),
    }
}

impl ModuleSpec for BloomFilterSpec {
    type Instance = BloomFilter;

    fn module(&self) -> &'static str {
        "bloom-filter"
    }

    fn observations(&self) -> &'static [&'static str] {
        // The first four are properties; `toJSON` is a nullary method.
        &["capacity", "errorRate", "hashFunctions", "data", "toJSON"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        prop_oneof![
            // A bare capacity, the shape upstream's own suite uses.
            2 => (1u32..=300).prop_map(|capacity| vec![json!(capacity)]),
            // ...and the options object, with an errorRate that spans from
            // "tight" to "so loose that hashFunctions truncates to zero".
            // Kept strictly below 1: see the module docs.
            2 => (1u32..=300, 1u32..=99)
                .prop_map(|(capacity, rate)| {
                    vec![json!({"capacity": capacity, "errorRate": rate as f64 / 100.0})]
                }),
            // A fractional capacity, which upstream's "positive integer"
            // message forbids and its `> 0` check allows.
            1 => (1u32..=600).prop_map(|half| vec![json!(half as f64 / 2.0)]),
        ]
        .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            4 => item_strategy().prop_map(|item| Op::new("add", vec![item])),
            4 => item_strategy().prop_map(|item| Op::new("test", vec![item])),
            // Rare: it throws the filter away, so a frequent `clear` would
            // spend the program on empty filters.
            1 => Just(Op::new("clear", Vec::new())),
            1 => Just(Op::new("toJSON", Vec::new())),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let (capacity, error_rate) = match &args[0] {
            Value::Object(options) => (
                options["capacity"]
                    .as_f64()
                    .expect("the generator always supplies a numeric capacity"),
                options.get("errorRate").and_then(Value::as_f64),
            ),
            other => (
                other
                    .as_f64()
                    .expect("the generator always supplies a numeric capacity"),
                None,
            ),
        };

        BloomFilter::new(capacity, error_rate)
            .expect("the generator stays inside the range that upstream accepts")
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            // Upstream returns `this` for chaining; the oracle encodes that as
            // `{"$self": true}`.
            "add" => {
                instance.add(&item_units(&op.args[0]));
                json!({"$self": true})
            }
            "test" => json!(instance.test(&item_units(&op.args[0]))),
            // A bare `return;` upstream, so `undefined`.
            "clear" => {
                instance
                    .clear()
                    .expect("the generator never reaches a negative length; see the module docs");
                json!({"$undefined": true})
            }
            "toJSON" => typed_data(instance),
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "capacity": js_number(instance.capacity()),
            "errorRate": js_number(instance.error_rate()),
            "hashFunctions": instance.hash_functions(),
            "data": typed_data(instance),
            "toJSON": typed_data(instance),
        })
    }
}

/// A `f64` as JSON the way `JSON.stringify` renders a JavaScript number.
///
/// JavaScript has one number type; JSON, and `serde_json::Value`, do not.
/// `JSON.stringify(6)` is `6`, which the Rust side parses as an integer, while
/// `json!(6.0_f64)` is `6.0` — and `Value::Number(6) != Value::Number(6.0)`.
///
/// Found by the fuzzer on its very first run, before a single real operation:
/// `capacity: port 6.0, upstream 6`. Worth recording rather than quietly
/// fixing, because it is the failure mode DESIGN.md warns about for every
/// module spec — an encoding mismatch is a *false* divergence, and a spec that
/// produced one on a rare value instead of on every value would have looked
/// like a port defect.
fn js_number(value: f64) -> Value {
    // 2^53, past which a `f64` no longer represents consecutive integers and
    // `JSON.stringify` starts emitting exponent notation anyway.
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// `data` as `fuzz/oracle.js` encodes a `Uint8Array`.
///
/// The `$typed` tag carries the constructor name, so a port that handed back a
/// plain array — or the wrong width — is a divergence rather than a silent
/// match on the values alone.
fn typed_data(instance: &BloomFilter) -> Value {
    json!({
        "$typed": "Uint8Array",
        "values": instance.data(),
    })
}

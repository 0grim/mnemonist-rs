//! [`ModuleSpec`] for `hashed-array-tree`.
//!
//! # The first grammar in which an op is allowed to throw
//!
//! `spec::CheckFailure` carries a note, written before any such module existed,
//! that a thrown exception arrives as apparatus failure and would abort the
//! campaign rather than being reported — and that the fix is to encode it as
//! `{"$throw": "<message>"}` on both sides. This is that module: `set` and
//! `get` both throw, one with upstream's own message and one with V8's, and
//! both are generated deliberately rather than avoided.
//!
//! Two consequences worth stating:
//!
//! * The comparison includes the **exact message text**, so a port that throws
//!   the right kind of error with the wrong wording is a divergence. That also
//!   ties the campaign to Node 24.18.1, since
//!   `Cannot set properties of undefined (setting '0')` is V8's phrasing.
//! * State is compared *after* a throw as well, which is what pins the claim
//!   that upstream's throws happen before any mutation.
//!
//! # Grammar
//!
//! Block sizes are tiny (1, 2, 4, 8) on purpose. Upstream's own test uses the
//! 1024-element default and pushes twice, so it never leaves the first block —
//! which is exactly why its `pop` bug survives. At `blockSize: 2` a 200-op
//! program spends nearly all of its time across block boundaries, where `pop`'s
//! wrong-block read and the `index == length` admission both bite.
//!
//! Indices run to `64` regardless of the tree's length, so roughly every third
//! `set` is out of bounds, one in many lands exactly on `length`, and the
//! `length == capacity` case that raises the `TypeError` is reached by
//! `resize`/`push` driving the two together.
//!
//! # Observable state
//!
//! `length`, `capacity`, `blockSize`, `offsetMask`, `blockMask` and **`blocks`**.
//! The blocks are the point: comparing them block for block after every op is
//! what makes the truncating stores, the `set(length)` write and the fact that
//! a shrinking `resize` deallocates nothing checkable directly, rather than
//! only through their eventual effect on `get`.

use mnemonist_core::structures::hashed_array_tree::{HashedArrayTree, Options};
use mnemonist_core::utils::typed_arrays::{PointerVec, PointerWidth};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::spec::{ModuleSpec, Op};

/// Largest index a generated `set`/`get` reaches.
///
/// Well past the lengths the generator builds, so the `length < index` throw is
/// common rather than rare.
const MAX_INDEX: u32 = 64;

/// Largest `capacity`/`length` a generated `grow`/`resize` asks for.
const MAX_EXTENT: u32 = 96;

/// Largest value pushed or stored.
///
/// Above 255 so a `Uint8Array` tree truncates, which is the only way the
/// element-width story gets checked.
const MAX_VALUE: u32 = 320;

pub struct HashedArrayTreeSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/hashed-array-tree.txt"
);

impl ModuleSpec for HashedArrayTreeSpec {
    type Instance = HashedArrayTree;

    fn module(&self) -> &'static str {
        "hashed-array-tree"
    }

    fn observations(&self) -> &'static [&'static str] {
        &[
            "length",
            "capacity",
            "blockSize",
            "offsetMask",
            "blockMask",
            "blocks",
        ]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        (
            // `{"$global": …}` is resolved to the real constructor by
            // fuzz/oracle.js; JSON has no way to carry one directly.
            prop::sample::select(&["Uint8Array", "Uint16Array", "Uint32Array"][..]),
            prop::sample::select(&[1u32, 2, 4, 8][..]),
            0u32..24,
            0u32..24,
        )
            .prop_map(|(class, block_size, initial_length, initial_capacity)| {
                vec![
                    json!({ "$global": class }),
                    json!({
                        "blockSize": block_size,
                        "initialLength": initial_length,
                        "initialCapacity": initial_capacity,
                    }),
                ]
            })
            .boxed()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        prop_oneof![
            // Weighted towards growth: everything interesting needs more than
            // one block, and only `push`/`grow`/`resize` get there.
            5 => (0u32..MAX_VALUE).prop_map(|v| Op::new("push", vec![json!(v)])),
            3 => Just(Op::new("pop", vec![])),
            3 => (0u32..MAX_INDEX, 0u32..MAX_VALUE)
                    .prop_map(|(i, v)| Op::new("set", vec![json!(i), json!(v)])),
            3 => (0u32..MAX_INDEX).prop_map(|i| Op::new("get", vec![json!(i)])),
            1 => (0u32..MAX_EXTENT).prop_map(|c| Op::new("grow", vec![json!(c)])),
            1 => Just(Op::new("grow", vec![])),
            2 => (0u32..MAX_EXTENT).prop_map(|l| Op::new("resize", vec![json!(l)])),
        ]
        .boxed()
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        let class = match args[0]["$global"].as_str() {
            Some("Uint8Array") => PointerWidth::U8,
            Some("Uint16Array") => PointerWidth::U16,
            Some("Uint32Array") => PointerWidth::U32,
            other => panic!("ctor arg 0 is a supported array class, got {other:?}"),
        };
        let options = &args[1];

        HashedArrayTree::new(
            class,
            Options {
                // The generator only emits powers of two, so upstream's
                // `options.blockSize || DEFAULT_BLOCK_SIZE` fallback is never
                // reached and does not have to be reproduced here.
                block_size: number(&options["blockSize"]),
                initial_length: number(&options["initialLength"]),
                initial_capacity: number(&options["initialCapacity"]),
            },
        )
        .expect("generated block sizes are powers of two within range")
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        match op.name {
            // `set`, `grow` and `resize` all return `this` upstream, which the
            // oracle encodes as `{"$self": true}`.
            "set" => match instance.set(number(&op.args[0]), number(&op.args[1]) as u32) {
                Ok(()) => json!({"$self": true}),
                Err(error) => thrown(&error),
            },
            "get" => match instance.get(number(&op.args[0])) {
                Ok(Some(value)) => json!(value),
                Ok(None) => json!({"$undefined": true}),
                Err(error) => thrown(&error),
            },
            "push" => json!(instance.push(number(&op.args[0]) as u32)),
            "pop" => match instance.pop() {
                Some(value) => json!(value),
                None => json!({"$undefined": true}),
            },
            "grow" => {
                instance.grow(op.args.first().map(number));
                json!({"$self": true})
            }
            "resize" => {
                instance.resize(number(&op.args[0]));
                json!({"$self": true})
            }
            other => panic!("op `{other}` is not in this module's alphabet"),
        }
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        json!({
            "length": instance.length(),
            "capacity": instance.capacity(),
            "blockSize": instance.block_size(),
            "offsetMask": instance.offset_mask(),
            "blockMask": instance.block_mask(),
            "blocks": instance.blocks().iter().map(typed_array).collect::<Vec<Value>>(),
        })
    }
}

fn number(value: &Value) -> usize {
    value
        .as_u64()
        .expect("generated arguments are non-negative integers") as usize
}

/// A thrown error, in the shape `fuzz/oracle.js` now reports one.
///
/// The message is compared in full, not just the fact of the throw: upstream's
/// own message embeds the array class, and V8's embeds the block offset, so a
/// port that raises at the right moment with the wrong text is still wrong.
fn thrown(error: &mnemonist_core::structures::hashed_array_tree::Error) -> Value {
    json!({ "$throw": error.to_string() })
}

/// Encode one block exactly as the oracle encodes a JS typed array.
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

//! [`ModuleSpec`]s for the `lru-cache` family: `lru-cache`, `lru-map`,
//! `lru-cache-with-delete`, `lru-map-with-delete`.
//!
//! # Four module keys, one grammar
//!
//! `test/lru-cache.js` requires all four upstream files and runs the exact
//! same `makeTests` suite against each of them (DESIGN.md §1.1 makes this one
//! unit), so this file mirrors that shape: one generic engine
//! ([`Instance`], parameterised on the index key `IK` exactly the way
//! `mnemonist_core::structures::lru_cache::LruCache<IK, K, V>` itself is),
//! and four thin [`ModuleSpec`] structs that each pick an `IK`, a `to_index`
//! function and whether `delete`/`remove` are in the alphabet. The oracle
//! addresses each by `require`-ing the matching `bench/upstream/<key>.js`, so
//! the four keys below are exactly the four upstream filenames.
//!
//! # Where the entropy is: small capacities, high op counts, `get`-heavy
//!
//! **The whole point of fuzzing an LRU is eviction order.** A capacity large
//! relative to the program length proves only that a map stores things — so
//! [`MAX_CAPACITY`] is small (6) and [`ModuleSpec::program_len`] is widened to
//! `1..300`, which keeps a generated program cycling the ring many times over
//! at every capacity in range. `get` is weighted the heaviest of any op (8),
//! because in an LRU a **read mutates recency** — `peek`/`has` are the
//! non-mutating controls, weighted low specifically so the campaign is not
//! dominated by them.
//!
//! # Keys are drawn from a small, deliberately mixed pool
//!
//! [`KEY_POOL`] mirrors [`crate::modules::default_map`]'s reasoning — few
//! enough keys that collisions, overwrites and evict-then-reinsert happen
//! constantly — but the mix is chosen for what is unique to *this* family:
//! `Int(0)` and `Str("0")` are the same key for `lru-cache`/`-with-delete`
//! (upstream's own `ToPropertyKey` coerces both to the property string `"0"`)
//! and two *different* keys for `lru-map`/`-with-delete` (a `Map`'s
//! SameValueZero never conflates a number with a string). One pool run against
//! all four specs therefore exercises the one thing this family's index
//! design is actually about, without a single line of `IK`-specific grammar.
//!
//! # The `-with-delete` pair: where the holes live
//!
//! `delete`/`remove` are only generated for the two `-with-delete` specs, and
//! at higher weight than `clear` for exactly the reason CLAUDE.md names them:
//! interleaved with `set`/`setpop`-driven eviction, they are what exercises
//! the freelist (`holes`) reuse path in
//! `mnemonist_core::structures::lru_cache::LruCache::insert_new`. `$iter` /
//! `$next` are generated frequently and independently of `delete`/`remove`,
//! so a single program routinely opens a walk and then deletes a key the walk
//! has not yet visited — see "Bugs this found" in `docs/modules/lru-cache.md`
//! for the port defect that exact interleaving found before this grammar
//! existed at all, by reading rather than fuzzing. This grammar is what keeps
//! it a regression rather than a one-off.
//!
//! # `items`, and the one observation deliberately left out
//!
//! The object-backed pair's `items` is a plain object (`{propertyKeyString:
//! pointer}`), and JSON object equality here is a **set** comparison — no
//! `preserve_order` feature on `serde_json`, so field order never matters —
//! so the full index (every live key's pointer, not just its count) is
//! compared after **every** op. That is a strong invariant: it pins the
//! pointer-allocation algorithm itself, not only the keys/values a walk
//! surfaces.
//!
//! The `Map`-backed pair's `items` is a real upstream `Map`, and the oracle's
//! `encode` renders any `Map` as `{"$map": [...]}` — an **ordered** list,
//! because two Maps can hold the same entries in different orders and that is
//! observable through iteration. `mnemonist_core`'s index is a plain
//! `std::collections::HashMap`, whose iteration order is unrelated to
//! insertion order and would drift from the oracle's on every single op —
//! not because of any port defect, but because nothing on the Rust side is
//! obliged to track insertion order for a structure that only ever *looks up*
//! by key. So `lru-map`/`lru-map-with-delete` omit `items` from
//! `observations()` entirely and rely on `size` — which is exactly the
//! judgement call `crate::mnemonist_napi::lru_map`'s own bridge already made,
//! for the same reason (see that module's doc comment). Comparing the full
//! map here would manufacture a divergence out of an implementation detail,
//! not find one.

use std::hash::Hash;

use mnemonist_core::cursor::{CursorState, Step};
use mnemonist_core::structures::lru_cache::{LruCache as CoreLru, Projected, Projection, SetPop};
use proptest::prelude::*;
use proptest::strategy::Union;
use serde_json::{json, Map as JsonMap, Value};

use crate::spec::{for_each, for_each_args, for_each_strategy, ModuleSpec, Op, FOR_EACH_MANY};

/// Largest capacity generated. Small on purpose — see the module docs.
const MAX_CAPACITY: u32 = 6;

/// Keys a generated program draws from. See the module docs for why `Int(0)`
/// and `Str("0")` are both here.
const KEY_POOL: usize = 10;

fn key_at(index: usize) -> Value {
    match index {
        0 => json!("a"),
        1 => json!("b"),
        2 => json!("0"),
        3 => json!(0),
        4 => json!(1),
        5 => json!(-1),
        6 => json!(true),
        7 => json!(false),
        8 => Value::Null,
        _ => json!({"$undefined": true}),
    }
}

/// A key, mirroring `mnemonist_napi::js_key::JsKey`'s primitive shapes —
/// `Undefined`/`Null`/`Bool`/`Number`/`String` — restricted to small integers
/// so [`property_key_of`] never has to reproduce `Number::toString`'s
/// scientific-notation cases (already covered, for real floats, by
/// `mnemonist_napi::lru_cache::js_number_to_string`'s own unit tests). The
/// restriction costs nothing here: the whole point of this pool is
/// collisions, and integers already deliver every one this family cares
/// about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FuzzKey {
    Undefined,
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
}

impl FuzzKey {
    fn from_json(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Bool(*value),
            Value::Number(number) => Self::Int(
                number
                    .as_i64()
                    .expect("generated key numbers are small integers"),
            ),
            Value::String(text) => Self::Str(text.clone()),
            Value::Object(fields) if fields.contains_key("$undefined") => Self::Undefined,
            other => panic!("`{other}` is not a key this grammar generates"),
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Self::Undefined => json!({"$undefined": true}),
            Self::Null => Value::Null,
            Self::Bool(value) => json!(value),
            Self::Int(value) => json!(value),
            Self::Str(text) => json!(text),
        }
    }

    /// JS `Boolean(value)`. Mirrors
    /// `mnemonist_napi::lru_cache::is_js_truthy` exactly, for BUG-LRU-CACHE-1 — see
    /// the `"setpop"` arm of [`apply_generic`].
    fn is_js_truthy(&self) -> bool {
        match self {
            Self::Undefined | Self::Null => false,
            Self::Bool(value) => *value,
            Self::Int(value) => *value != 0,
            Self::Str(text) => !text.is_empty(),
        }
    }
}

/// `ToPropertyKey`, restricted to what [`FuzzKey`] can express — the
/// object-backed pair's index key. Mirrors
/// `mnemonist_napi::lru_cache::property_key_of` exactly, for the primitive
/// shapes both share.
fn property_key_of(key: &FuzzKey) -> String {
    match key {
        FuzzKey::Undefined => "undefined".to_string(),
        FuzzKey::Null => "null".to_string(),
        FuzzKey::Bool(true) => "true".to_string(),
        FuzzKey::Bool(false) => "false".to_string(),
        FuzzKey::Int(value) => value.to_string(),
        FuzzKey::Str(text) => text.clone(),
    }
}

/// The `Map`-backed pair's index key: the raw [`FuzzKey`] itself, unmodified
/// — SameValueZero, not a property-string coercion.
fn identity_index(key: &FuzzKey) -> FuzzKey {
    key.clone()
}

/// A value alphabet mixed enough to tell every JSON scalar shape apart, with
/// the `undefined` marker weighted in rather than rare: `remove`'s "custom
/// missing indicator" behaviour and `setpop`'s three-way return both depend
/// on it appearing often.
fn value_strategy() -> BoxedStrategy<Value> {
    prop_oneof![
        3 => Just(json!({"$undefined": true})),
        2 => Just(Value::Null),
        3 => (-4i64..4).prop_map(|n| json!(n)),
        2 => Just(json!("v")),
        2 => Just(json!("w")),
        1 => Just(json!(true)),
    ]
    .boxed()
}

/// What the `$forEach` callback may do, and how often. `set` writes back the
/// SAME `[key, value]` pair it was just handed (rule `"arg1,arg0"`, matching
/// `crate::modules::default_map`) — harmless to capacity, but it re-promotes
/// that key to the head, which is exactly the re-entrancy shape worth having:
/// upstream's own `forEach` freezes `size`/`head` at entry and reads
/// `forward`/`keys`/`values` live (bug-for-bug the same frozen-bound shape as
/// `circular-buffer`, not the live bound `default-map`'s `Map.prototype.forEach`
/// has), so a promotion mid-walk can relink the very pointer the walk is
/// about to visit next.
const MUTATIONS_PLAIN: &[(&str, &str, u64)] = &[
    ("clear", "none", FOR_EACH_MANY),
    ("set", "arg1,arg0", FOR_EACH_MANY),
];

/// The `-with-delete` pair's mutation table: [`MUTATIONS_PLAIN`] plus
/// `delete`, at a high per-step budget so it fires on nearly every callback
/// invocation rather than only once — this is the interleaving that found the
/// pointer-reuse defect documented in `docs/modules/lru-cache.md`.
const MUTATIONS_WITH_DELETE: &[(&str, &str, u64)] = &[
    ("clear", "none", FOR_EACH_MANY),
    ("set", "arg1,arg0", FOR_EACH_MANY),
    ("delete", "arg1", FOR_EACH_MANY),
];

/// The shared engine behind all four specs. `IK` is exactly
/// `mnemonist_core::structures::lru_cache::LruCache`'s own index-key
/// parameter; `to_index` is a plain `fn`, not a closure, because it never
/// captures anything — the object-backed pair's [`property_key_of`] and the
/// `Map`-backed pair's [`identity_index`] both serve as `index_of` (turning a
/// generated key into an `IK` for lookup) AND as `to_index` (re-deriving an
/// `IK` from the stored `K` on eviction), because this grammar never narrows
/// a key the way an `ArrayClass` (`Uint8Array` `Keys`) would — see the module
/// docs on what that means for the eviction-index-disagreement bug
/// `mnemonist_core::structures::lru_cache`'s own docs describe: this grammar
/// cannot reach it, by construction, because `index_of` and `to_index` are
/// literally the same function here.
/// One open walk, and which projection it was opened as — mirrors
/// `crate::modules::default_map::Instance`.
type OpenCursor<IK> = (CursorState<CoreLru<IK, FuzzKey, Value>>, Projection);

pub struct Instance<IK: Hash + Eq + Clone> {
    cache: CoreLru<IK, FuzzKey, Value>,
    to_index: fn(&FuzzKey) -> IK,
    /// The one cursor a program can have open, if any.
    cursor: Option<OpenCursor<IK>>,
}

fn construct_generic<IK: Hash + Eq + Clone>(
    args: &[Value],
    to_index: fn(&FuzzKey) -> IK,
) -> Instance<IK> {
    let capacity = args[0]
        .as_u64()
        .expect("generated capacities are positive integers") as usize;

    Instance {
        cache: CoreLru::new(capacity).expect("generated capacities are always in range"),
        to_index,
        cursor: None,
    }
}

fn key_index<IK: Hash + Eq + Clone>(instance: &Instance<IK>, arg: &Value) -> IK {
    (instance.to_index)(&FuzzKey::from_json(arg))
}

/// `get`/`peek`'s return: a present-but-`undefined` value and a missing key
/// are the same wire value, exactly as they are indistinguishable through
/// upstream's own return value.
fn get_result(value: Option<&Value>) -> Value {
    value.cloned().unwrap_or(json!({"$undefined": true}))
}

/// One `keys()`/`values()`/`entries()` step, rendered as the oracle's real JS
/// iterator would.
fn projected_json(projection: Projection, item: Projected<FuzzKey, Value>) -> Value {
    match projection {
        Projection::Keys => item.key().expect("a Keys walk yields Key").to_json(),
        Projection::Values => item.value().expect("a Values walk yields Value"),
        Projection::Entries => {
            let (key, value) = item.entry().expect("an Entries walk yields Entry");

            json!([key.to_json(), value])
        }
    }
}

fn apply_generic<IK: Hash + Eq + Clone>(instance: &mut Instance<IK>, op: &Op) -> Value {
    match op.name {
        "get" => {
            let index_key = key_index(instance, &op.args[0]);

            get_result(instance.cache.get(&index_key))
        }
        "peek" => {
            let index_key = key_index(instance, &op.args[0]);

            get_result(instance.cache.peek(&index_key))
        }
        "has" => {
            let index_key = key_index(instance, &op.args[0]);

            json!(instance.cache.has(&index_key))
        }
        "set" => {
            let raw = FuzzKey::from_json(&op.args[0]);
            let index_key = (instance.to_index)(&raw);

            instance
                .cache
                .set(index_key, raw, op.args[1].clone(), instance.to_index);

            json!({"$undefined": true})
        }
        "setpop" => {
            let raw = FuzzKey::from_json(&op.args[0]);
            let index_key = (instance.to_index)(&raw);

            match instance
                .cache
                .set_pop(index_key, raw, op.args[1].clone(), instance.to_index)
            {
                SetPop::None => Value::Null,
                SetPop::Overwritten { key, value } => {
                    json!({"evicted": false, "key": key.to_json(), "value": value})
                }
                // BUG-LRU-CACHE-1: upstream's `if (oldKey) {...} else { return null; }`
                // -- a JS-falsy evicted key silently suppresses the eviction
                // report. Mirrors `mnemonist_napi::lru_cache::is_js_truthy`;
                // see docs/modules/lru-cache.md.
                SetPop::Evicted { key, value } if key.is_js_truthy() => {
                    json!({"evicted": true, "key": key.to_json(), "value": value})
                }
                SetPop::Evicted { .. } => Value::Null,
            }
        }
        "clear" => {
            instance.cache.clear();

            json!({"$undefined": true})
        }
        "delete" => {
            let index_key = key_index(instance, &op.args[0]);

            json!(instance.cache.delete(&index_key))
        }
        "remove" => {
            let index_key = key_index(instance, &op.args[0]);

            match instance.cache.remove(&index_key) {
                Some(value) => value,
                // Upstream's default parameter: the caller's `missing`
                // argument, echoed back exactly as received.
                None => op.args[1].clone(),
            }
        }
        "$iter" => {
            let projection = match op.args[0].as_str().expect("$iter names a projection") {
                "keys" => Projection::Keys,
                "values" => Projection::Values,
                "entries" => Projection::Entries,
                other => panic!("`{other}` is not an iterator this module has"),
            };
            let frozen = instance.cache.frozen(projection);

            instance.cursor = Some((
                CursorState::open_projected(&instance.cache, frozen),
                projection,
            ));

            json!({"$iterator": true})
        }
        "$next" => match instance.cursor.as_mut() {
            None => json!({"$noIterator": true}),
            Some((cursor, projection)) => {
                let projection = *projection;

                match cursor.step(&instance.cache) {
                    Step::Item(item) => {
                        json!({"done": false, "value": projected_json(projection, item)})
                    }
                    // Structurally unreachable: `LruCache::slot` always
                    // returns `Some` (see that module's docs). Handled rather
                    // than `unreachable!()`'d so a future regression reports
                    // as a divergence instead of a panic.
                    Step::Gap => json!({"done": false, "value": {"$undefined": true}}),
                    Step::Done => json!({"done": true, "value": {"$undefined": true}}),
                }
            }
        },
        // `Array.from(instance)` — the collection's own `Symbol.iterator`,
        // aliased to `entries()` upstream.
        "$spread" => {
            let frozen = instance.cache.frozen(Projection::Entries);
            let mut cursor = CursorState::open_projected(&instance.cache, frozen);
            let mut items = Vec::new();

            loop {
                match cursor.step(&instance.cache) {
                    Step::Item(item) => items.push(projected_json(Projection::Entries, item)),
                    Step::Gap => items.push(json!({"$undefined": true})),
                    Step::Done => break,
                }
            }

            Value::Array(items)
        }
        // Upstream's `forEach`: `l = this.size`, `pointer = this.head` frozen
        // at entry, `forward`/`keys`/`values` read live, and the pointer
        // advances AFTER the callback — not before, unlike the three lazy
        // iterators. That is why this is `ForEachWalk`, not `CursorState`;
        // see the core module's docs and "Bugs this found" in
        // `docs/modules/lru-cache.md`. The callback receives
        // `(value, key, this)`, so a mutation's rule selects out of
        // `[value, key]`.
        "$forEach" => {
            let spec = for_each(op);
            let mut walk = instance.cache.for_each_walk();
            let mut seen = Vec::new();
            let mut fired = 0usize;

            while let Some((key, value)) = walk.current(&instance.cache) {
                let received = vec![value, key.to_json()];

                seen.push(Value::Array(received.clone()));

                if fired < spec.limit {
                    if let Some(args) = for_each_args(&spec, &received) {
                        fired += 1;

                        match spec.method.expect("for_each_args returned Some") {
                            "clear" => instance.cache.clear(),
                            "set" => {
                                let raw = FuzzKey::from_json(args[0]);
                                let index_key = (instance.to_index)(&raw);

                                instance.cache.set(
                                    index_key,
                                    raw,
                                    args[1].clone(),
                                    instance.to_index,
                                );
                            }
                            "delete" => {
                                let raw = FuzzKey::from_json(args[0]);
                                let index_key = (instance.to_index)(&raw);

                                instance.cache.delete(&index_key);
                            }
                            other => {
                                panic!("`{other}` is not a $forEach mutation for this module")
                            }
                        }
                    }
                }

                // Read `forward` NOW, live, after whatever the mutation above
                // just did -- exactly where upstream's own
                // `pointer = forward[pointer]` sits, one statement below the
                // callback call. See `ForEachWalk`'s docs.
                walk.advance(&instance.cache);
            }

            json!({ "seen": seen })
        }
        other => panic!("op `{other}` is not in this module's alphabet"),
    }
}

/// Shared `capacity`/`size`/`head`/`tail`, common to all four observations.
fn base_observations<IK: Hash + Eq + Clone>(instance: &Instance<IK>) -> JsonMap<String, Value> {
    let mut state = JsonMap::new();

    state.insert("capacity".into(), json!(instance.cache.capacity() as u64));
    state.insert("size".into(), json!(instance.cache.len() as u64));
    state.insert("head".into(), json!(instance.cache.head() as u64));
    state.insert("tail".into(), json!(instance.cache.tail() as u64));

    state
}

/// The object-backed pair's `observe`: `items` compared in full. See the
/// module docs for why that is safe here and not for the `Map`-backed pair.
fn observe_property_backed(instance: &Instance<String>) -> Value {
    let mut state = base_observations(instance);
    let mut items = JsonMap::new();

    for (key, pointer) in instance.cache.index_entries() {
        items.insert(key.clone(), json!(pointer as u64));
    }

    state.insert("items".into(), Value::Object(items));

    Value::Object(state)
}

/// The `Map`-backed pair's `observe`: `items` deliberately omitted. See the
/// module docs.
fn observe_map_backed(instance: &Instance<FuzzKey>) -> Value {
    Value::Object(base_observations(instance))
}

fn ctor_strategy() -> BoxedStrategy<Vec<Value>> {
    (1u32..=MAX_CAPACITY)
        .prop_map(|capacity| vec![json!(capacity)])
        .boxed()
}

fn op_strategy(with_delete: bool) -> BoxedStrategy<Op> {
    let key = || (0..KEY_POOL).prop_map(key_at).boxed();

    let mut variants: Vec<(u32, BoxedStrategy<Op>)> = vec![
        // `get` is weighted heaviest of any op: it is the mutating read, and
        // an LRU's whole point is that a read changes recency. See the
        // module docs.
        (8, key().prop_map(|k| Op::new("get", vec![k])).boxed()),
        (2, key().prop_map(|k| Op::new("peek", vec![k])).boxed()),
        (2, key().prop_map(|k| Op::new("has", vec![k])).boxed()),
        (
            6,
            (key(), value_strategy())
                .prop_map(|(k, v)| Op::new("set", vec![k, v]))
                .boxed(),
        ),
        (
            3,
            (key(), value_strategy())
                .prop_map(|(k, v)| Op::new("setpop", vec![k, v]))
                .boxed(),
        ),
        (1, Just(Op::new("clear", vec![])).boxed()),
        (
            2,
            prop_oneof![
                Just(Op::new("$iter", vec![json!("keys")])),
                Just(Op::new("$iter", vec![json!("values")])),
                Just(Op::new("$iter", vec![json!("entries")])),
            ]
            .boxed(),
        ),
        (4, Just(Op::new("$next", vec![])).boxed()),
        (2, Just(Op::new("$spread", vec![])).boxed()),
    ];

    if with_delete {
        // Weighted high and independent of `$iter`/`$next`, so a generated
        // program routinely opens a walk and deletes a not-yet-visited key
        // from underneath it — see the module docs.
        variants.push((4, key().prop_map(|k| Op::new("delete", vec![k])).boxed()));
        variants.push((
            3,
            (key(), value_strategy())
                .prop_map(|(k, missing)| Op::new("remove", vec![k, missing]))
                .boxed(),
        ));
        variants.push((2, for_each_strategy(MUTATIONS_WITH_DELETE)));
    } else {
        variants.push((2, for_each_strategy(MUTATIONS_PLAIN)));
    }

    Union::new_weighted(variants).boxed()
}

/// `lru-cache`: the object-backed base class.
pub struct LruCacheSpec;

/// Path proptest writes a minimised failing seed to.
pub const REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/lru-cache.txt"
);

impl ModuleSpec for LruCacheSpec {
    type Instance = Instance<String>;

    fn module(&self) -> &'static str {
        "lru-cache"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["capacity", "size", "head", "tail", "items"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        ctor_strategy()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        op_strategy(false)
    }

    fn program_len(&self) -> std::ops::Range<usize> {
        1..300
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        construct_generic(args, property_key_of)
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        apply_generic(instance, op)
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        observe_property_backed(instance)
    }
}

/// `lru-cache-with-delete`.
pub struct LruCacheWithDeleteSpec;

pub const WITH_DELETE_REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/lru-cache-with-delete.txt"
);

impl ModuleSpec for LruCacheWithDeleteSpec {
    type Instance = Instance<String>;

    fn module(&self) -> &'static str {
        "lru-cache-with-delete"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["capacity", "size", "head", "tail", "items"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        ctor_strategy()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        op_strategy(true)
    }

    fn program_len(&self) -> std::ops::Range<usize> {
        1..300
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        construct_generic(args, property_key_of)
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        apply_generic(instance, op)
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        observe_property_backed(instance)
    }
}

/// `lru-map`: the `Map`-backed base class.
pub struct LruMapSpec;

pub const MAP_REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/lru-map.txt"
);

impl ModuleSpec for LruMapSpec {
    type Instance = Instance<FuzzKey>;

    fn module(&self) -> &'static str {
        "lru-map"
    }

    fn observations(&self) -> &'static [&'static str] {
        // `items` deliberately omitted -- see the module docs.
        &["capacity", "size", "head", "tail"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        ctor_strategy()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        op_strategy(false)
    }

    fn program_len(&self) -> std::ops::Range<usize> {
        1..300
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        construct_generic(args, identity_index)
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        apply_generic(instance, op)
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        observe_map_backed(instance)
    }
}

/// `lru-map-with-delete`.
pub struct LruMapWithDeleteSpec;

pub const MAP_WITH_DELETE_REGRESSIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/lru-map-with-delete.txt"
);

impl ModuleSpec for LruMapWithDeleteSpec {
    type Instance = Instance<FuzzKey>;

    fn module(&self) -> &'static str {
        "lru-map-with-delete"
    }

    fn observations(&self) -> &'static [&'static str] {
        &["capacity", "size", "head", "tail"]
    }

    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>> {
        ctor_strategy()
    }

    fn op_strategy(&self, _ctor: &[Value]) -> BoxedStrategy<Op> {
        op_strategy(true)
    }

    fn program_len(&self) -> std::ops::Range<usize> {
        1..300
    }

    fn construct(&self, args: &[Value]) -> Self::Instance {
        construct_generic(args, identity_index)
    }

    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value {
        apply_generic(instance, op)
    }

    fn observe(&self, instance: &mut Self::Instance) -> Value {
        observe_map_backed(instance)
    }
}

/// A self-check on the GRAMMAR, not a differential test — no oracle, no
/// `node`, nothing compared against upstream. DESIGN.md's own warning about
/// this family is that a campaign whose capacity is large relative to its op
/// count proves only that a map stores things, and "the weights look right by
/// inspection" is exactly the kind of confident-but-unverified claim
/// CLAUDE.md's NOTES.md keeps a table of. So this runs a representative batch
/// of generated programs purely against `mnemonist-core` and asserts a floor
/// on how often `set`/`setpop` actually evict and how often `delete` actually
/// removes something, printing the real counts under `--nocapture` for the
/// numbers a report can cite.
#[cfg(test)]
mod grammar_self_check {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRunner};

    use super::*;

    /// `samples` generated `(ctor, ops)` pairs, run start to finish. Returns
    /// `(total ops applied, evictions, successful deletes)`.
    ///
    /// An eviction is detected the same way the algorithm decides one: before
    /// a `set`/`setpop` call, the key is not already present AND the cache is
    /// already at capacity. Both are necessary and sufficient for
    /// `LruCache::insert_new`'s "cache is full" branch to run.
    fn sample(with_delete: bool, samples: usize) -> (u64, u64, u64) {
        let mut runner = TestRunner::new(Config::default());
        let mut total_ops = 0u64;
        let mut evictions = 0u64;
        let mut successful_deletes = 0u64;

        for _ in 0..samples {
            let ctor = ctor_strategy()
                .new_tree(&mut runner)
                .expect("ctor_strategy never rejects")
                .current();
            let ops = proptest::collection::vec(op_strategy(with_delete), 1..300)
                .new_tree(&mut runner)
                .expect("op_strategy never rejects")
                .current();
            let mut instance = construct_generic(&ctor, property_key_of);

            for op in &ops {
                total_ops += 1;

                if op.name == "set" || op.name == "setpop" {
                    let index_key = key_index(&instance, &op.args[0]);
                    let will_evict = instance.cache.len() == instance.cache.capacity()
                        && !instance.cache.has(&index_key);

                    if will_evict {
                        evictions += 1;
                    }
                }

                let result = apply_generic(&mut instance, op);

                if op.name == "delete" && result == Value::Bool(true) {
                    successful_deletes += 1;
                }
            }
        }

        (total_ops, evictions, successful_deletes)
    }

    #[test]
    fn the_grammar_evicts_constantly_rather_than_only_storing() {
        let (ops, evictions, _) = sample(false, 400);

        eprintln!(
            "lru-cache grammar (no delete): {ops} ops, {evictions} evictions \
             ({:.1}% of ops)",
            100.0 * evictions as f64 / ops as f64
        );

        assert!(
            evictions * 20 > ops,
            "eviction should fire on a healthy fraction of ops, not a token \
             few: {evictions} evictions over {ops} ops"
        );
    }

    #[test]
    fn the_with_delete_grammar_deletes_and_evicts_constantly() {
        let (ops, evictions, deletes) = sample(true, 400);

        eprintln!(
            "lru-cache-with-delete grammar: {ops} ops, {evictions} evictions \
             ({:.1}%), {deletes} successful deletes ({:.1}%)",
            100.0 * evictions as f64 / ops as f64,
            100.0 * deletes as f64 / ops as f64
        );

        assert!(
            evictions * 20 > ops,
            "eviction should fire on a healthy fraction of ops: {evictions} \
             over {ops}"
        );
        assert!(
            deletes * 100 > ops,
            "delete should succeed on a healthy fraction of ops, which is what \
             actually exercises the freelist: {deletes} over {ops}"
        );
    }
}

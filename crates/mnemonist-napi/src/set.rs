//! JS bridge for [`mnemonist_core::structures::set`].
//!
//! Fourteen free functions over **native JavaScript `Set`s**. Like
//! [`crate::sort`] there is no instance and no `#[napi]` class; unlike it, the
//! values crossing the boundary are objects with behaviour, so this is where
//! the unit's one real capability lives.
//!
//! # The capability: `Set` at the boundary, not storage in core
//!
//! `set.js` holds no state — it reads sets in and hands a set back. So core
//! takes and returns [`OrderedSet<JsKey>`], an ordinary insertion-ordered set,
//! and everything JavaScript-specific is the three helpers at the bottom of
//! this file: [`read`] drains a `Set` through its own `values()` iterator,
//! [`build`] constructs one with the real `new Set(array)`, and [`replay`]
//! calls the caller's own `add`/`delete`.
//!
//! Member equality is [`JsKey`]'s, which is SameValueZero by construction —
//! the same rule a `Set` uses, and the same one `Map` uses, which is why the
//! type is shared with `crate::default_map` rather than re-derived. Its stated
//! limit applies unchanged: **object members are rejected loudly**, because no
//! identity hash for a JS object is reachable from Rust. `test/set.js` uses
//! numbers and single characters.
//!
//! # Why the mutating four replay calls instead of rebuilding
//!
//! `add`, `subtract`, `intersect` and `disjunct` mutate the caller's set and
//! return `undefined`. The obvious bridge is: read A, compute the answer,
//! `A.clear()`, re-add. It passes every assertion in `test/set.js` and it is
//! wrong in a way nothing there can see — an iterator already open over `A`
//! would observe every member being removed and re-inserted.
//!
//! So core returns the [`SetOp`] trace it applied, in upstream's own order,
//! and [`replay`] makes exactly those `add`/`delete` calls on the real object.
//! What the caller's set experiences is then what upstream's experiences,
//! call for call.
//!
//! # Variadicity, and why it goes through an array
//!
//! `intersection` and `union` are variadic upstream and napi has no variadic
//! parameter. They take a `Vec` here and `tests/bridge/set.js` spreads into
//! it — arity glue in the shim, which is DESIGN.md §2.3's Problem 2 again and
//! the same role `crate::statics` plays for `X.of`. The arity check itself
//! stays in core, so the message and the threshold are upstream's.
//!
//! # One upstream shortcut the bridge cannot supply, and why it does not matter
//!
//! `intersection` skips `set.has(item)` when `set === smallestSet` — object
//! identity. Two arguments that are the same JS `Set` become two separate
//! [`OrderedSet`]s here, so the shortcut never fires. It is unobservable:
//! when the identity holds, the check it skips would have been `smallest.has`
//! on a member drawn from `smallest`, which is `true` by construction. Same
//! for `isSubset`'s `A === B` and `intersectionSize`'s, both of which return
//! exactly what the loop would have computed. Core still implements all three
//! with pointer equality, so a Rust caller that *does* pass one reference twice
//! gets upstream's code path.

use mnemonist_core::structures::set::{self as core_set, OrderedSet, SetOp};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::js_key::JsKey;

/// `set.js#intersection` — variadic, so the shim spreads into this array.
#[napi(js_name = "setIntersection")]
pub fn set_intersection<'env>(env: Env, sets: Vec<Unknown<'env>>) -> Result<Unknown<'env>> {
    let read: Vec<OrderedSet<JsKey>> = sets.iter().map(read).collect::<Result<_>>()?;
    let borrowed: Vec<&OrderedSet<JsKey>> = read.iter().collect();

    build(&env, throwing(core_set::intersection(&borrowed))?)
}

/// `set.js#union` — variadic, as above.
#[napi(js_name = "setUnion")]
pub fn set_union<'env>(env: Env, sets: Vec<Unknown<'env>>) -> Result<Unknown<'env>> {
    let read: Vec<OrderedSet<JsKey>> = sets.iter().map(read).collect::<Result<_>>()?;
    let borrowed: Vec<&OrderedSet<JsKey>> = read.iter().collect();

    build(&env, throwing(core_set::union(&borrowed))?)
}

/// `set.js#difference` — `A \ B`.
#[napi(js_name = "setDifference")]
pub fn set_difference<'env>(env: Env, a: Unknown<'env>, b: Unknown<'env>) -> Result<Unknown<'env>> {
    build(&env, core_set::difference(&read(&a)?, &read(&b)?))
}

/// `set.js#symmetricDifference`.
#[napi(js_name = "setSymmetricDifference")]
pub fn set_symmetric_difference<'env>(
    env: Env,
    a: Unknown<'env>,
    b: Unknown<'env>,
) -> Result<Unknown<'env>> {
    build(&env, core_set::symmetric_difference(&read(&a)?, &read(&b)?))
}

/// `set.js#isSubset`.
#[napi(js_name = "setIsSubset")]
pub fn set_is_subset(a: Unknown, b: Unknown) -> Result<bool> {
    Ok(core_set::is_subset(&read(&a)?, &read(&b)?))
}

/// `set.js#isSuperset`, which upstream defines as `isSubset(B, A)`.
#[napi(js_name = "setIsSuperset")]
pub fn set_is_superset(a: Unknown, b: Unknown) -> Result<bool> {
    Ok(core_set::is_superset(&read(&a)?, &read(&b)?))
}

/// `set.js#add` — mutates `A`, returns nothing.
#[napi(js_name = "setAdd")]
pub fn set_add(a: Unknown, b: Unknown) -> Result<()> {
    let mut left = read(&a)?;
    let ops = core_set::add(&mut left, &read(&b)?);

    replay(&a, &ops)
}

/// `set.js#subtract` — mutates `A`.
#[napi(js_name = "setSubtract")]
pub fn set_subtract(a: Unknown, b: Unknown) -> Result<()> {
    let mut left = read(&a)?;
    let ops = core_set::subtract(&mut left, &read(&b)?);

    replay(&a, &ops)
}

/// `set.js#intersect` — mutates `A`.
#[napi(js_name = "setIntersect")]
pub fn set_intersect(a: Unknown, b: Unknown) -> Result<()> {
    let mut left = read(&a)?;
    let ops = core_set::intersect(&mut left, &read(&b)?);

    replay(&a, &ops)
}

/// `set.js#disjunct` — mutates `A` into the symmetric difference.
#[napi(js_name = "setDisjunct")]
pub fn set_disjunct(a: Unknown, b: Unknown) -> Result<()> {
    let mut left = read(&a)?;
    let ops = core_set::disjunct(&mut left, &read(&b)?);

    replay(&a, &ops)
}

/// `set.js#intersectionSize`.
#[napi(js_name = "setIntersectionSize")]
pub fn set_intersection_size(a: Unknown, b: Unknown) -> Result<u32> {
    Ok(core_set::intersection_size(&read(&a)?, &read(&b)?) as u32)
}

/// `set.js#unionSize`.
#[napi(js_name = "setUnionSize")]
pub fn set_union_size(a: Unknown, b: Unknown) -> Result<u32> {
    Ok(core_set::union_size(&read(&a)?, &read(&b)?) as u32)
}

/// `set.js#jaccard`.
#[napi(js_name = "setJaccard")]
pub fn set_jaccard(a: Unknown, b: Unknown) -> Result<f64> {
    Ok(core_set::jaccard(&read(&a)?, &read(&b)?))
}

/// `set.js#overlap`.
#[napi(js_name = "setOverlap")]
pub fn set_overlap(a: Unknown, b: Unknown) -> Result<f64> {
    Ok(core_set::overlap(&read(&a)?, &read(&b)?))
}

/// Surface a core arity error as the `Error` upstream throws.
fn throwing<T>(outcome: std::result::Result<T, &'static str>) -> Result<T> {
    outcome.map_err(|message| Error::new(Status::GenericFailure, message.to_owned()))
}

/// Drain a JS `Set` into an [`OrderedSet`], preserving insertion order.
///
/// Goes through the object's own `values()` iterator rather than reading any
/// internal slot, which is what upstream does and what makes a `Set` subclass
/// with an overridden `values` behave here as it does there. `has` is *not*
/// consulted — see the divergence doc: upstream calls the caller's `has`, this
/// answers from the snapshot, and the two differ only for an object that lies
/// about its own membership.
fn read(value: &Unknown) -> Result<OrderedSet<JsKey>> {
    // SAFETY: `Object` is the widest object shape napi has and `cast` performs
    // the conversion `get_named_property` would perform anyway; a non-object
    // fails on the property read below, with a message naming `values`.
    let object = unsafe { value.cast::<Object>()? };

    let values: Function<'_, (), Object> = object.get_named_property("values").map_err(|_| {
        Error::new(
            Status::InvalidArg,
            "mnemonist-rs: expected a Set (an object with a `values` method) -- \
             see docs/modules/set.md."
                .to_owned(),
        )
    })?;
    let iterator = values.apply(object, ())?;
    let next: Function<'_, (), Object> = iterator.get_named_property("next")?;

    let mut members = OrderedSet::new();

    loop {
        let step: Object = next.apply(iterator, ())?;

        if step.get_named_property::<bool>("done")? {
            return Ok(members);
        }

        members.add(step.get_named_property_unchecked::<JsKey>("value")?);
    }
}

/// Construct a real JS `Set` from an [`OrderedSet`], in order.
///
/// `new Set(array)` against the realm's own `Set`, rather than an object that
/// merely behaves like one: `test/set.js` does `Array.from(result)`, and the
/// caller is entitled to a value that `instanceof Set`.
fn build<'env>(env: &Env, members: OrderedSet<JsKey>) -> Result<Unknown<'env>> {
    let global = env.get_global()?;
    let constructor: Function<'_, Array, Unknown> = global.get_named_property_unchecked("Set")?;

    constructor.new_instance(Array::from_vec(env, members.to_vec())?)
}

/// Apply a trace to the caller's own `Set`, call for call.
///
/// The `add` and `delete` handles are fetched once, before the first call, so
/// a trace cannot be diverted halfway by a member's own side effects. Upstream
/// resolves `A.add` per call and so could be diverted; there is nothing in the
/// original suite either way, and one lookup is the honest reading of
/// `A.add(x)` repeated in a loop.
fn replay(target: &Unknown, ops: &[SetOp<JsKey>]) -> Result<()> {
    if ops.is_empty() {
        return Ok(());
    }

    // SAFETY: as in `read` — this value has already been drained as a `Set`,
    // so it is an object.
    let object = unsafe { target.cast::<Object>()? };

    let add: Function<'_, JsKey, Unknown> = object.get_named_property_unchecked("add")?;
    let delete: Function<'_, JsKey, Unknown> = object.get_named_property_unchecked("delete")?;

    for op in ops {
        match op {
            SetOp::Add(member) => add.apply(object, member.clone())?,
            SetOp::Delete(member) => delete.apply(object, member.clone())?,
        };
    }

    Ok(())
}

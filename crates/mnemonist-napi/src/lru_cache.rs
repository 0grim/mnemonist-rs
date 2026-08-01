//! JS bridge for [`mnemonist_core::structures::lru_cache`], and the shared
//! machinery all four upstream files use.
//!
//! `lru-map.js`, `lru-cache-with-delete.js` and `lru-map-with-delete.js` all
//! `require('./lru-cache.js')` upstream and copy most of its prototype
//! verbatim, so this module is the natural home for everything the four
//! bridges share — `crate::lru_map`, `crate::lru_cache_with_delete` and
//! `crate::lru_map_with_delete` all `use` from here.
//!
//! # Modelling the two storage mechanisms
//!
//! [`mnemonist_core::structures::lru_cache::LruCache<IK, K, V>`] is generic
//! over the index key `IK` (`Hash + Eq`) and the raw stored key `K`. The two
//! families instantiate it differently:
//!
//! * **`lru-cache` / `lru-cache-with-delete`** — backed by a plain `{}`.
//!   `this.items[key]` string-coerces `key` for the lookup while
//!   `this.K[pointer]` keeps the raw value, and `test/lru-cache.js:65` asserts
//!   both halves independently (`Object.keys(cache.items).length` against
//!   `Array.from(cache.entries())`). So `IK = `[`PropertyKey`], a JS
//!   `ToPropertyKey`-shaped string, and `K = `[`crate::js_slot::JsSlot`], the
//!   raw value.
//! * **`lru-map` / `lru-map-with-delete`** — backed by a real `Map`,
//!   SameValueZero. `IK = `[`crate::js_key::JsKey`] directly, and `K` is the
//!   same [`JsKey`](crate::js_key::JsKey) converted back to a
//!   [`JsSlot`](crate::js_slot::JsSlot) — see [`stored_key_of`].
//!
//! Only strings and numbers reach either index in the original suite (the
//! same audit DESIGN.md §3.8 makes for the rest of T3), so [`JsKey`] — already
//! built for the `Map`-backed pair — is reused as the classification for
//! **both** families rather than building a second, wider key parser. Concrete
//! consequence: an object key is rejected for `lru-cache` too, where upstream
//! would actually coerce it via `ToPropertyKey`. No test reaches that path;
//! see `docs/modules/lru-cache.md`.
//!
//! # Values, and why this family does not need `Retained`/`Received`/`Loaned`
//!
//! `default-map`'s value triple exists because a plain `Option<T>` argument
//! collapses `null` and `undefined`, and because releasing a held reference
//! needs an `Env` that `Drop` does not have. [`JsSlot`](crate::js_slot::JsSlot)
//! — built for `Stack`/`Queue` — already solves both: it keeps `Null` and
//! `Undefined` as distinct variants, and its own `Drop` releases a held
//! `napi_ref` without needing an `Env`, because [`Handle`](crate::js_slot::Handle)
//! stores the raw `napi_env` itself. So every stored key and value in this
//! family is a `JsSlot`, taken as the method's `Unknown` argument rather than
//! through napi's typed `Option<T>` decoding.
//!
//! # The `Keys`/`Values` array classes
//!
//! `new Cache(Keys, Values, capacity)` accepts optional array-class
//! constructors (`Uint8Array`, `Float64Array`, …) that narrow what is stored
//! in `this.K`/`this.V` — reused verbatim from [`crate::array_class::ArrayClass`],
//! built for `FixedStack`/`FixedDeque`/`CircularBuffer`. Unlike those three,
//! upstream does **not** throw when the class is not a function: `typeof Keys
//! === 'function' ? new Keys(capacity) : new Array(capacity)` silently falls
//! back to a plain array, so a `None` here means exactly that fallback.
//!
//! # The one place a stored key and an index key can disagree
//!
//! Eviction removes the displaced entry from the index by re-deriving its
//! index key from the **stored** `K` (`to_index`, below) — exactly upstream's
//! `delete this.items[this.K[pointer]]`, not the key `set` was originally
//! called with. Reproduced bug-for-bug in `mnemonist-core`; see that crate's
//! `lru_cache` module docs and `docs/modules/lru-cache.md`.

use std::cell::RefCell;
use std::hash::Hash;
use std::rc::Rc;

use mnemonist_core::structures::lru_cache::{
    LruCache as CoreLru, NewError, Projected, Projection, SetPop,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::array_class::ArrayClass;
use crate::cursor::CellCursor;
use crate::js_key::JsKey;
use crate::js_slot::JsSlot;

// ---------------------------------------------------------------------------
// The object-backed index key: JS `ToPropertyKey`, restricted to the
// primitive shapes `JsKey` classifies.
// ---------------------------------------------------------------------------

/// The index key `lru-cache`/`lru-cache-with-delete` use: a property-key
/// string, the way a plain object's bracket access coerces one.
///
/// `Rc<str>` for the same reason [`crate::js_key::JsKey::String`] is: the
/// index holds one copy and every `keys()`/`entries()` step over the *index*
/// (not reached here, since the stored `K` is a `JsSlot`, not a `PropertyKey`)
/// would want another.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PropertyKey(Rc<str>);

impl PropertyKey {
    /// The property-key text itself — what `Object.keys(cache.items)` would
    /// list this entry under.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `ToPropertyKey`, restricted to what [`JsKey`] can classify.
///
/// Only `Undefined`/`Null`/`Bool`/`Number`/`String` ever reach a `JsKey` at
/// all — anything else is already rejected by [`JsKey::from_unknown`] before
/// this runs — so the five arms below are the whole of the port's
/// `ToPropertyKey`. An object key would upstream coerce through
/// `toString`/`valueOf`; none of the original suite's four `makeTests` blocks
/// ever supplies one. See the module docs.
pub fn property_key_of(key: &JsKey) -> PropertyKey {
    let text = match key {
        JsKey::Undefined => Rc::from("undefined"),
        JsKey::Null => Rc::from("null"),
        JsKey::Bool(true) => Rc::from("true"),
        JsKey::Bool(false) => Rc::from("false"),
        JsKey::Number(bits) => Rc::from(js_number_to_string(f64::from_bits(*bits)).as_str()),
        JsKey::String(text) => Rc::clone(text),
    };

    PropertyKey(text)
}

/// `ToPropertyKey` of whatever `lru-cache`'s eviction re-reads from `this.K`.
///
/// `stored` is always one of the shapes `JsKey` covers, because the only
/// route a value takes into `K` is [`coerce_key`], which starts from a
/// [`JsKey`]-classified raw argument. A `JsSlot::BigInt` or
/// `JsSlot::Referenced` reaching here would mean an object or symbol survived
/// [`JsKey::from_unknown`]'s rejection, which cannot happen.
pub fn property_key_of_stored(stored: &JsSlot) -> PropertyKey {
    let text = match stored {
        JsSlot::Undefined => Rc::from("undefined"),
        JsSlot::Null => Rc::from("null"),
        JsSlot::Boolean(true) => Rc::from("true"),
        JsSlot::Boolean(false) => Rc::from("false"),
        JsSlot::Number(value) => Rc::from(js_number_to_string(*value).as_str()),
        JsSlot::String(units) => Rc::from(String::from_utf16_lossy(units).as_str()),
        JsSlot::BigInt(_) | JsSlot::Referenced(_) => {
            unreachable!(
                "a stored lru-cache key is always JsKey-shaped -- see coerce_key and \
                 JsKey::from_unknown, which reject everything else before a key ever \
                 reaches storage"
            )
        }
    };

    PropertyKey(text)
}

/// `ECMA-262 Number::toString(10)` — the shortest decimal that round-trips,
/// formatted the way JavaScript places the point rather than the way Rust's
/// `Display` does (which never switches to exponential notation).
///
/// Digits come from Rust's own shortest-round-trip conversion (`{:e}`, which
/// shares its `flt2dec` machinery with `Display`), so the two languages agree
/// on *which* digits; this function only reproduces the spec's placement
/// rules (steps 6–20 of `Number::toString`) on top of them. Unconfirmed beyond
/// the boundary cases checked in this module's tests — full agreement with V8
/// across the entire `f64` range is not independently verified here.
pub fn js_number_to_string(value: f64) -> String {
    if value.is_nan() {
        return String::from("NaN");
    }

    if value == 0.0 {
        // Covers `-0`: `String(-0) === '0'`.
        return String::from("0");
    }

    if value < 0.0 {
        return format!("-{}", js_number_to_string(-value));
    }

    if value.is_infinite() {
        return String::from("Infinity");
    }

    // Rust's scientific form: "<digit>[.<digits>]e<exp>", already the
    // shortest round-tripping decimal.
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("scientific notation always has an exponent");
    let exponent: i32 = exponent.parse().expect("Rust's exponent is an integer");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };

    let k = digits.len() as i32;
    // `n` is the spec's: the value equals `digits (as an integer) * 10^(n-k)`.
    let n = exponent + 1;

    if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        let (whole, fraction) = digits.split_at(n as usize);
        format!("{whole}.{fraction}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let mantissa = if k == 1 {
            digits.to_string()
        } else {
            let (first, rest) = digits.split_at(1);
            format!("{first}.{rest}")
        };
        let sign = if n > 0 { "+" } else { "-" };

        format!("{mantissa}e{sign}{}", (n - 1).abs())
    }
}

// ---------------------------------------------------------------------------
// The `Map`-backed index key: `JsKey` directly, both as `IK` and as the
// source `K` is rebuilt from.
// ---------------------------------------------------------------------------

/// `lru-map`'s stored key, rebuilt from the `JsKey` an eviction re-reads.
///
/// The `Map`-backed pair's `IK` and the value an eviction re-derives from `K`
/// are the *same* `JsKey`, so there is no coercion gap here the way there is
/// for [`property_key_of_stored`] — `to_index` for this family is
/// [`JsKey::from_unknown`] applied to the stored slot's own shape, which
/// [`stored_key_of`] guarantees round-trips.
pub fn stored_key_of(key: &JsKey) -> JsSlot {
    match key {
        JsKey::Undefined => JsSlot::Undefined,
        JsKey::Null => JsSlot::Null,
        JsKey::Bool(value) => JsSlot::Boolean(*value),
        JsKey::Number(bits) => JsSlot::Number(f64::from_bits(*bits)),
        JsKey::String(text) => JsSlot::String(Rc::new(text.encode_utf16().collect())),
    }
}

/// The inverse of [`stored_key_of`], used as `to_index` for the `Map`-backed
/// pair's eviction. Total over the same domain [`property_key_of_stored`]
/// covers, for the same reason.
///
/// `JsSlot::Number(value).to_bits()` is used directly rather than re-running
/// SameValueZero normalisation: the only route a value takes into `K` is
/// [`stored_key_of`], which only ever starts from an already-normalised
/// `JsKey::Number`, so the bits are normalised already.
pub fn js_key_of_stored(stored: &JsSlot) -> JsKey {
    match stored {
        JsSlot::Undefined => JsKey::Undefined,
        JsSlot::Null => JsKey::Null,
        JsSlot::Boolean(value) => JsKey::Bool(*value),
        JsSlot::Number(value) => JsKey::Number(value.to_bits()),
        JsSlot::String(units) => JsKey::String(Rc::from(String::from_utf16_lossy(units).as_str())),
        JsSlot::BigInt(_) | JsSlot::Referenced(_) => unreachable!(
            "a stored lru-map key is always JsKey-shaped -- see coerce_key and \
             JsKey::from_unknown, which reject everything else before a key ever \
             reaches storage"
        ),
    }
}

// ---------------------------------------------------------------------------
// Constructor / `from` argument handling, shared by all four bridges.
// ---------------------------------------------------------------------------

/// `typeof capacity !== 'number' || capacity <= 0` -> `not_positive`;
/// `!isFinite(capacity) || Math.floor(capacity) !== capacity` -> `not_integer`.
///
/// The two upstream messages differ only by module name
/// (`mnemonist/lru-cache: ...` vs `mnemonist/lru-map: ...`); both are supplied
/// by the caller so this one function serves all four bridges.
pub fn validate_capacity(
    capacity: &Unknown,
    not_positive: &'static str,
    not_integer: &'static str,
) -> Result<usize> {
    if capacity.get_type()? != ValueType::Number {
        return Err(Error::new(Status::InvalidArg, not_positive));
    }

    // SAFETY: the type check above is exactly `typeof === 'number'`.
    let value = unsafe { capacity.cast::<f64>()? };

    validate_capacity_number(value, not_positive, not_integer)
}

/// The numeric half of [`validate_capacity`], shared with the `Cache.from`
/// path where the candidate capacity comes from `guessLength` rather than
/// from a JS argument and so is already an `f64`, never an `Unknown`.
fn validate_capacity_number(
    value: f64,
    not_positive: &'static str,
    not_integer: &'static str,
) -> Result<usize> {
    if value <= 0.0 {
        return Err(Error::new(Status::InvalidArg, not_positive));
    }

    if !value.is_finite() || value.floor() != value {
        return Err(Error::new(Status::InvalidArg, not_integer));
    }

    Ok(value as usize)
}

/// `NewError` rendered with the caller's module-specific wording. Both
/// messages are `&'static str`s already carrying the right prefix, so this is
/// only ever the `TooLarge` case in practice — `validate_capacity` runs first
/// and a capacity it accepted only fails here past `2^32`.
pub fn map_new_error(error: NewError, not_positive: &'static str) -> Error {
    match error {
        NewError::ZeroCapacity => Error::new(Status::InvalidArg, not_positive),
        NewError::TooLarge => Error::new(
            Status::GenericFailure,
            mnemonist_core::utils::typed_arrays::POINTER_ARRAY_TOO_LARGE,
        ),
    }
}

/// One resolved `(Keys, Values, capacity)` triple, however the constructor
/// received it.
pub struct Construction {
    pub keys_class: Option<ArrayClass>,
    pub values_class: Option<ArrayClass>,
    pub capacity: usize,
}

/// `typeof Keys === 'function' ? ArrayClass::probe(Keys) : None`. Silent
/// fallback, not a throw — see the module docs on why this differs from
/// `FixedStack`'s `ArrayClass`.
fn resolve_class(env: &Env, value: &Unknown) -> Result<Option<ArrayClass>> {
    if value.get_type()? != ValueType::Function {
        return Ok(None);
    }

    ArrayClass::probe(env, value, NOT_A_CONSTRUCTOR).map(Some)
}

const NOT_A_CONSTRUCTOR: &str = "Keys is not a constructor";

/// `function LRUCache(Keys, Values, capacity) { if (arguments.length < 2) { ... } }`
///
/// napi cannot see `arguments.length`; an omitted trailing argument and an
/// explicit `undefined` there collapse, the same divergence `FixedStack`'s
/// `ArrayClass` already accepts (D-61) — `values_arg` (the second formal
/// parameter, matching upstream's own "< 2") being `undefined` stands in for
/// "fewer than two arguments were passed", i.e. `new Cache(capacity)`.
pub fn resolve_construction(
    env: &Env,
    keys_arg: &Unknown,
    values_arg: &Unknown,
    capacity_arg: &Unknown,
    not_positive: &'static str,
    not_integer: &'static str,
) -> Result<Construction> {
    if values_arg.get_type()? == ValueType::Undefined {
        // `capacity = Keys; Keys = null; Values = null;`
        let capacity = validate_capacity(keys_arg, not_positive, not_integer)?;

        return Ok(Construction {
            keys_class: None,
            values_class: None,
            capacity,
        });
    }

    Ok(Construction {
        keys_class: resolve_class(env, keys_arg)?,
        values_class: resolve_class(env, values_arg)?,
        capacity: validate_capacity(capacity_arg, not_positive, not_integer)?,
    })
}

/// `Cache.from(iterable, Keys, Values, capacity)`'s own arity dance, plus the
/// `guessLength` fallback when only the iterable was supplied.
#[allow(clippy::too_many_arguments)]
pub fn resolve_from_construction(
    env: &Env,
    iterable: &Unknown,
    keys_arg: &Unknown,
    values_arg: &Unknown,
    capacity_arg: &Unknown,
    not_positive: &'static str,
    not_integer: &'static str,
    cannot_guess: &'static str,
) -> Result<Construction> {
    if keys_arg.get_type()? == ValueType::Undefined {
        // `capacity = iterables.guessLength(iterable); if (typeof capacity
        // !== 'number') throw ...` -- note this is the CANNOT-GUESS message,
        // not the capacity one: a guess that IS a number still has to pass
        // the ordinary capacity checks below, unlike a constructor call,
        // where an omitted capacity always hits `not_positive` instead.
        let length = crate::iterables::guess_length(env, iterable)?
            .ok_or_else(|| Error::new(Status::GenericFailure, cannot_guess))?;
        let capacity = validate_capacity_number(length, not_positive, not_integer)?;

        return Ok(Construction {
            keys_class: None,
            values_class: None,
            capacity,
        });
    }

    if values_arg.get_type()? == ValueType::Undefined {
        // `arguments.length === 2`: the second argument was really the
        // capacity.
        let capacity = validate_capacity(keys_arg, not_positive, not_integer)?;

        return Ok(Construction {
            keys_class: None,
            values_class: None,
            capacity,
        });
    }

    Ok(Construction {
        keys_class: resolve_class(env, keys_arg)?,
        values_class: resolve_class(env, values_arg)?,
        capacity: validate_capacity(capacity_arg, not_positive, not_integer)?,
    })
}

/// `typeof Keys === 'function' ? new Keys(capacity) : new Array(capacity)`,
/// then the one-element round trip that actually narrows -- or, with no
/// class, the raw value untouched.
pub fn coerce(env: &Env, class: &Option<ArrayClass>, value: &Unknown) -> Result<JsSlot> {
    match class {
        Some(class) => class.coerce(env, value),
        None => JsSlot::new(env, value),
    }
}

// ---------------------------------------------------------------------------
// Populating a fresh cache from an iterable -- `Cache.from`'s shared body.
// ---------------------------------------------------------------------------

/// `forEach(iterable, function (value, key) { cache.set(key, value); })`,
/// applied directly into a freshly constructed core cache.
///
/// Generic over `IK` so `lru-cache`/`lru-cache-with-delete` (`IK =
/// PropertyKey`) and `lru-map`/`lru-map-with-delete` (`IK = JsKey`) share one
/// body; `index_of`/`to_index` are the two family-specific derivations
/// described in the module docs.
pub fn populate_from<IK, IndexOf, ToIndex>(
    env: &Env,
    iterable: Unknown,
    keys_class: &Option<ArrayClass>,
    values_class: &Option<ArrayClass>,
    capacity: usize,
    index_of: IndexOf,
    to_index: ToIndex,
) -> Result<Rc<RefCell<CoreLru<IK, JsSlot, JsSlot>>>>
where
    IK: Hash + Eq + 'static,
    IndexOf: Fn(&JsKey) -> IK + Copy + 'static,
    ToIndex: Fn(&JsSlot) -> IK + Copy + 'static,
{
    let cache = Rc::new(RefCell::new(CoreLru::new(capacity).map_err(|error| {
        map_new_error(
            error,
            "mnemonist/lru-cache.from: capacity should be positive number.",
        )
    })?));

    // A JS collector function so branch 2 of `obliterator/forEach` hands a
    // host `.forEach` exactly the callback shape it expects -- the same
    // reasoning as `crate::foreach::collect`.
    //
    // `(*keys_class).clone()`, not `keys_class.clone()`: the receiver is
    // already `&Option<ArrayClass>`, and a bare `.clone()` there resolves to
    // the blanket `impl Clone for &T` -- cloning the *reference*, with the
    // same borrowed lifetime -- rather than the value, which is exactly the
    // borrow the closure below must not capture.
    let keys_class = (*keys_class).clone();
    let values_class = (*values_class).clone();
    // A second, cloned `Rc` for the closure -- not an attempt to later
    // recover sole ownership. `populate_from` returns the shared `Rc` itself
    // (the bridge's `inner` field is `Rc<RefCell<...>>` for exactly this
    // reason): a JS `Function`'s boxed closure is freed whenever V8 collects
    // it, which is not necessarily before this call returns, so trying to
    // unwrap the `Rc` back to a lone owner here would be relying on GC timing
    // that N-API makes no promise about.
    let population = Rc::clone(&cache);
    let collector: Function<'_, FnArgs<(Unknown, Unknown)>, ()> = env
        .create_function_from_closure("collect", move |context| {
            let value: Unknown = context.get(0)?;
            let key: Unknown = context.get(1)?;

            let raw_key = JsKey::from_unknown(&key)?;
            let index_key = index_of(&raw_key);
            let stored_key = coerce(context.env, &keys_class, &key)?;
            let stored_value = coerce(context.env, &values_class, &value)?;

            population
                .borrow_mut()
                .set(index_key, stored_key, stored_value, to_index);

            Ok(())
        })?;
    // SAFETY: `collector` is a JS function this call just created.
    let collector = unsafe { Unknown::from_raw_unchecked(env.raw(), collector.raw()) };

    crate::foreach::for_each(env, iterable, collector)?;

    Ok(cache)
}

// ---------------------------------------------------------------------------
// Shared cursor-stepping helpers, one body for the twelve iterator classes
// across the four bridges.
// ---------------------------------------------------------------------------

/// One `[key, value]` pair, as `entries()` yields it.
///
/// A dedicated type because napi has no tuple-to-array conversion and the
/// array must be built with the `env` only available at conversion time --
/// same reasoning as `crate::default_map::Pair`.
pub struct Pair(pub JsSlot, pub JsSlot);

impl ToNapiValue for Pair {
    unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> Result<napi::sys::napi_value> {
        let key = unsafe { ToNapiValue::to_napi_value(env, val.0)? };
        let value = unsafe { ToNapiValue::to_napi_value(env, val.1)? };

        let mut pair = std::ptr::null_mut();
        napi::check_status!(
            unsafe { napi::sys::napi_create_array_with_length(env, 2, &mut pair) },
            "mnemonist-rs: failed to build a [key, value] pair"
        )?;
        napi::check_status!(
            unsafe { napi::sys::napi_set_element(env, pair, 0, key) },
            "mnemonist-rs: failed to set a pair's key"
        )?;
        napi::check_status!(
            unsafe { napi::sys::napi_set_element(env, pair, 1, value) },
            "mnemonist-rs: failed to set a pair's value"
        )?;

        Ok(pair)
    }
}

/// One step of a `keys()` cursor, for any of the four bridges' owner types.
pub fn step_key<Owner: 'static, IK: Hash + Eq + 'static>(
    cursor: &mut CellCursor<Owner, CoreLru<IK, JsSlot, JsSlot>>,
) -> Option<JsSlot> {
    cursor.step().item().and_then(Projected::key)
}

/// One step of a `values()` cursor.
pub fn step_value<Owner: 'static, IK: Hash + Eq + 'static>(
    cursor: &mut CellCursor<Owner, CoreLru<IK, JsSlot, JsSlot>>,
) -> Option<JsSlot> {
    cursor.step().item().and_then(Projected::value)
}

/// One step of an `entries()` cursor.
pub fn step_entry<Owner: 'static, IK: Hash + Eq + 'static>(
    cursor: &mut CellCursor<Owner, CoreLru<IK, JsSlot, JsSlot>>,
) -> Option<Pair> {
    cursor
        .step()
        .item()
        .and_then(Projected::entry)
        .map(|(key, value)| Pair(key, value))
}

/// `forEach`'s shared body: open a [`ForEachWalk`] over `inner` now, then
/// re-borrow once for the read and once more for the advance, so a re-entrant
/// `set`/`delete` from inside the callback is never fighting an outstanding
/// borrow -- the same discipline `default_map`'s `forEach` uses, and for the
/// same reason (B-31).
///
/// `ForEachWalk`, not `CursorState`: advancing the walk's pointer has to
/// happen *after* `callback.apply` returns, reading `forward` as it stands
/// once the callback's own mutation (if any) has already run -- exactly
/// where upstream's own `pointer = forward[pointer]` sits in its loop body,
/// one statement below the callback call. `CursorState`'s `Sequence` impl
/// advances eagerly, which is right for `keys`/`values`/`entries` (upstream's
/// own lazy-iterator closures do the same) and was wrong here: this bridge
/// used to open an `Entries` walk the same way those three do, and it
/// reproduced their timing instead of `forEach`'s own. See
/// `mnemonist_core::structures::lru_cache`'s module docs and
/// `docs/modules/lru-cache.md`'s "Bugs this found".
pub fn for_each_entries<IK: Hash + Eq>(
    inner: &RefCell<CoreLru<IK, JsSlot, JsSlot>>,
    this: This,
    callback: Function<FnArgs<(JsSlot, JsSlot, Object)>, Unknown>,
    scope: Option<Unknown>,
) -> Result<()> {
    let mut walk = inner.borrow().for_each_walk();

    loop {
        let current = {
            let borrowed = inner.borrow();

            walk.current(&borrowed)
        };

        let Some((key, value)) = current else {
            break;
        };

        let arguments = FnArgs::from((value, key, this.object));

        match &scope {
            Some(scope) => callback.apply(*scope, arguments)?,
            None => callback.apply(this, arguments)?,
        };

        let borrowed = inner.borrow();

        walk.advance(&borrowed);
    }

    Ok(())
}

/// A `CellCursor` alias, so the four bridges' iterator structs stay short.
pub type Cursor<Owner, IK> = CellCursor<Owner, CoreLru<IK, JsSlot, JsSlot>>;

// ---------------------------------------------------------------------------
// LRUCache -- the object-backed pair's base class.
// ---------------------------------------------------------------------------

/// Verbatim from `lru-cache.js`.
const NOT_POSITIVE: &str = "mnemonist/lru-cache: capacity should be positive number.";
/// Verbatim from `lru-cache.js`.
const NOT_INTEGER: &str = "mnemonist/lru-cache: capacity should be a finite positive integer.";
/// Verbatim from `lru-cache.js`.
const CANNOT_GUESS: &str = "mnemonist/lru-cache.from: could not guess iterable length. \
     Please provide desired capacity as last argument.";

/// The core instantiation both `LRUCache` and `LRUCacheWithDelete` share.
pub type CacheCore = CoreLru<PropertyKey, JsSlot, JsSlot>;

/// `to_index` for the object-backed pair's eviction: re-derive the
/// property-string index key from the stored (possibly `Keys`-narrowed) key.
pub fn cache_to_index(stored: &JsSlot) -> PropertyKey {
    property_key_of_stored(stored)
}

/// `{evicted, key, value}` — upstream's `setpop` result object. `null` is
/// `Option::None`, mapped by napi the way it should be here (unlike a missing
/// map value, upstream's `setpop` really does `return null`).
///
/// A hand-written `ToNapiValue` rather than `#[napi(object)]`: the derive
/// would also require `FromNapiValue` on every field, and
/// [`JsSlot`](crate::js_slot::JsSlot) only ever travels *out* of Rust in this
/// family (raw `Unknown` arguments are decoded by hand into it — see
/// [`coerce`]), so it has no `FromNapiValue` impl to satisfy that with.
pub struct SetPopOutcome {
    pub evicted: bool,
    pub key: JsSlot,
    pub value: JsSlot,
}

impl ToNapiValue for SetPopOutcome {
    unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> Result<napi::sys::napi_value> {
        let env = Env::from_raw(env);
        let mut object = Object::new(&env)?;

        object.set("evicted", val.evicted)?;
        object.set("key", val.key)?;
        object.set("value", val.value)?;

        unsafe { ToNapiValue::to_napi_value(env.raw(), object) }
    }
}

/// An LRU cache backed by a plain object index (JS `{}`). See the module
/// docs for how `IK`/`K` are split.
#[napi(js_name = "LRUCache")]
pub struct JsLruCache {
    inner: Rc<RefCell<CacheCore>>,
    keys_class: Option<ArrayClass>,
    values_class: Option<ArrayClass>,
}

#[napi]
impl JsLruCache {
    /// `new LRUCache(Keys, Values, capacity)`.
    #[napi(constructor)]
    pub fn new(env: Env, keys: Unknown, values: Unknown, capacity: Unknown) -> Result<Self> {
        let construction =
            resolve_construction(&env, &keys, &values, &capacity, NOT_POSITIVE, NOT_INTEGER)?;
        let inner = CacheCore::new(construction.capacity)
            .map_err(|error| map_new_error(error, NOT_POSITIVE))?;

        Ok(Self {
            inner: Rc::new(RefCell::new(inner)),
            keys_class: construction.keys_class,
            values_class: construction.values_class,
        })
    }

    #[napi(getter)]
    pub fn capacity(&self) -> u32 {
        self.inner.borrow().capacity() as u32
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().len() as u32
    }

    /// Upstream's `head` property: the pointer of the most recently used
    /// entry.
    #[napi(getter)]
    pub fn head(&self) -> u32 {
        self.inner.borrow().head() as u32
    }

    /// Upstream's `tail` property: the pointer of the least recently used
    /// entry.
    #[napi(getter)]
    pub fn tail(&self) -> u32 {
        self.inner.borrow().tail() as u32
    }

    /// Upstream's `this.items`: a plain object, one property per live entry,
    /// keyed by the property-string index and valued by the internal pointer
    /// (as `this.items[key] = pointer` stores it upstream).
    /// `test/lru-cache.js:65` only inspects `Object.keys(...).length`, but the
    /// values are included anyway for fidelity.
    #[napi(getter)]
    pub fn items(&self, env: Env) -> Result<Object<'_>> {
        let inner = self.inner.borrow();
        let mut object = Object::new(&env)?;

        for (key, pointer) in inner.index_entries() {
            object.set(key.as_str(), pointer as u32)?;
        }

        Ok(object)
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    #[napi]
    pub fn has(&self, key: JsKey) -> bool {
        self.inner.borrow().has(&property_key_of(&key))
    }

    /// No splay, no promotion.
    #[napi]
    pub fn peek(&self, key: JsKey) -> Either<JsSlot, Undefined> {
        self.inner
            .borrow()
            .peek(&property_key_of(&key))
            .cloned()
            .into()
    }

    /// Splays the entry to the front on a hit.
    #[napi]
    pub fn get(&self, key: JsKey) -> Either<JsSlot, Undefined> {
        self.inner
            .borrow_mut()
            .get(&property_key_of(&key))
            .cloned()
            .into()
    }

    /// Upstream returns `undefined` (there is no `return this` at the end of
    /// `LRUCache.prototype.set`), unlike `DefaultMap`'s chainable `set`.
    #[napi]
    pub fn set(&self, env: Env, key: Unknown, value: Unknown) -> Result<()> {
        let raw_key = JsKey::from_unknown(&key)?;
        let index_key = property_key_of(&raw_key);
        let stored_key = coerce(&env, &self.keys_class, &key)?;
        let stored_value = coerce(&env, &self.values_class, &value)?;

        self.inner
            .borrow_mut()
            .set(index_key, stored_key, stored_value, cache_to_index);

        Ok(())
    }

    #[napi]
    pub fn setpop(&self, env: Env, key: Unknown, value: Unknown) -> Result<Option<SetPopOutcome>> {
        let raw_key = JsKey::from_unknown(&key)?;
        let index_key = property_key_of(&raw_key);
        let stored_key = coerce(&env, &self.keys_class, &key)?;
        let stored_value = coerce(&env, &self.values_class, &value)?;

        let outcome =
            self.inner
                .borrow_mut()
                .set_pop(index_key, stored_key, stored_value, cache_to_index);

        Ok(match outcome {
            SetPop::None => None,
            SetPop::Overwritten { key, value } => Some(SetPopOutcome {
                evicted: false,
                key,
                value,
            }),
            SetPop::Evicted { key, value } => Some(SetPopOutcome {
                evicted: true,
                key,
                value,
            }),
        })
    }

    /// `(value, key, this)`, newest-first — see [`for_each_entries`].
    #[allow(clippy::type_complexity)]
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(JsSlot, JsSlot, Object)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        for_each_entries(&self.inner, this, callback, scope)
    }

    #[napi]
    pub fn keys(&self, env: Env, this: Reference<JsLruCache>) -> Result<JsLruCacheKeys> {
        let frozen = self.inner.borrow().frozen(Projection::Keys);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruCacheKeys {
            cursor: CellCursor::open_projected(source, frozen),
        })
    }

    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsLruCache>) -> Result<JsLruCacheValues> {
        let frozen = self.inner.borrow().frozen(Projection::Values);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruCacheValues {
            cursor: CellCursor::open_projected(source, frozen),
        })
    }

    /// Also `Symbol.iterator`, aliased by `install_iterator_factories`.
    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsLruCache>) -> Result<JsLruCacheEntries> {
        let frozen = self.inner.borrow().frozen(Projection::Entries);
        let source = this.share_with(env, |cache| Ok(&*cache.inner))?;

        Ok(JsLruCacheEntries {
            cursor: CellCursor::open_projected(source, frozen),
        })
    }

    /// `LRUCache.from(iterable, Keys, Values, capacity)`.
    #[napi(factory)]
    pub fn from(
        env: Env,
        iterable: Unknown,
        keys: Unknown,
        values: Unknown,
        capacity: Unknown,
    ) -> Result<Self> {
        let construction = resolve_from_construction(
            &env,
            &iterable,
            &keys,
            &values,
            &capacity,
            NOT_POSITIVE,
            NOT_INTEGER,
            CANNOT_GUESS,
        )?;
        let inner = populate_from(
            &env,
            iterable,
            &construction.keys_class,
            &construction.values_class,
            construction.capacity,
            property_key_of,
            cache_to_index,
        )?;

        Ok(Self {
            inner,
            keys_class: construction.keys_class,
            values_class: construction.values_class,
        })
    }
}

impl Default for JsLruCache {
    /// Never actually reachable from JS (the constructor requires a
    /// capacity), but several bridges in this crate provide one for
    /// consistency; kept here for the same reason.
    fn default() -> Self {
        Self {
            inner: Rc::new(RefCell::new(
                CacheCore::new(1).expect("capacity 1 is always valid"),
            )),
            keys_class: None,
            values_class: None,
        }
    }
}

/// The cursor `LRUCache.prototype.keys()` hands out.
#[napi(iterator, js_name = "LRUCacheKeys")]
pub struct JsLruCacheKeys {
    cursor: Cursor<JsLruCache, PropertyKey>,
}

impl Generator for JsLruCacheKeys {
    type Yield = JsSlot;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<JsSlot> {
        step_key(&mut self.cursor)
    }

    /// Upstream's cursors have no `return` method, so a `break` out of a
    /// `for…of` leaves the walk exactly where it stopped.
    fn complete(&mut self, _value: Option<()>) -> Option<JsSlot> {
        None
    }
}

/// The cursor `LRUCache.prototype.values()` hands out.
#[napi(iterator, js_name = "LRUCacheValues")]
pub struct JsLruCacheValues {
    cursor: Cursor<JsLruCache, PropertyKey>,
}

impl Generator for JsLruCacheValues {
    type Yield = JsSlot;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<JsSlot> {
        step_value(&mut self.cursor)
    }

    fn complete(&mut self, _value: Option<()>) -> Option<JsSlot> {
        None
    }
}

/// The cursor `LRUCache.prototype.entries()` hands out, and
/// `LRUCache.prototype[Symbol.iterator]`.
#[napi(iterator, js_name = "LRUCacheEntries")]
pub struct JsLruCacheEntries {
    cursor: Cursor<JsLruCache, PropertyKey>,
}

impl Generator for JsLruCacheEntries {
    type Yield = Pair;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Pair> {
        step_entry(&mut self.cursor)
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Pair> {
        None
    }
}

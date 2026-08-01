//! `JsKey` — a JavaScript value usable as a `Map` key, with SameValueZero.
//!
//! [`mnemonist_core::map::OrderedMap`] is generic over `K: Hash + Eq + Clone`
//! and knows nothing about JavaScript. Everything specific to JavaScript keys
//! is here, which is the whole of the T3 split: core owns *ordering and
//! liveness*, the bridge owns *key equality*.
//!
//! # SameValueZero, and why `Hash`/`Eq` can just be derived
//!
//! `Map` compares keys with SameValueZero, which differs from `===` in exactly
//! two places:
//!
//! | | `===` | SameValueZero |
//! |---|---|---|
//! | `NaN` vs `NaN` | `false` | **`true`** |
//! | `+0` vs `-0` | `true` | `true` |
//!
//! Both are handled by *normalising at construction* rather than by a
//! hand-written `PartialEq`: [`JsKey::Number`] stores the bit pattern of an
//! already-normalised `f64`, where every `NaN` has been folded to one
//! canonical `NaN` and `-0.0` to `+0.0`. Derived `Hash` and `Eq` over that
//! representation are then SameValueZero by construction, with no way for the
//! two to disagree — which is the failure mode a hand-written `PartialEq`
//! paired with a derived `Hash` invites.
//!
//! Normalising `-0` also reproduces a detail of `Map.prototype.set` that is
//! easy to miss: the *stored* key becomes `+0`, so it comes back out of
//! `keys()` as `0`. Confirmed against Node 24.18.1 —
//! `m.set(-0, 1); Object.is([...m.keys()][0], -0)` is `false`.
//!
//! # Object keys are NOT supported, deliberately
//!
//! `Map` compares objects by **identity**, and there is no identity hash for a
//! JS object reachable from Rust. The two implementable designs both cost
//! something real:
//!
//! * **Tag each object** with a hidden monotonic id under a private `Symbol`.
//!   O(1), but it mutates the caller's object — visible to
//!   `Object.getOwnPropertySymbols` — and fails outright on a frozen or
//!   sealed one.
//! * **An association list** of `napi_ref` to id, probed linearly with
//!   `napi_strict_equals`. Does not touch the caller's object, but it is O(n)
//!   per object-keyed operation and every entry is a strong reference, so
//!   getting the release right is a memory-leak problem in its own right.
//!
//! **No upstream test in the entire T3 family uses an object as a key.**
//! Audited across `default-map`, `set`, `bi-map`, `fuzzy-map`,
//! `fuzzy-multi-map`, `multi-map`, `multi-set`, `lru-cache` (all four
//! variants) and `sparse-map`: every key that reaches a `Map` is a string or a
//! number. `fuzzy-map` and `fuzzy-multi-map` *accept* objects at the public
//! API, but hash them to a string before the `Map` ever sees one.
//!
//! So this port rejects a non-primitive key **loudly**, with a message that
//! names the limitation, rather than shipping machinery no test can reach or —
//! far worse — silently answering wrongly. See `docs/modules/default-map.md`.

use std::rc::Rc;

use napi::bindgen_prelude::*;
use napi::sys;

/// What a rejected key type is told.
///
/// Deliberately specific: the failure mode this guards against is a port that
/// quietly treats every object as the same key, and a caller who hits this
/// should learn immediately that it is a stated limit rather than a bug.
const UNSUPPORTED: &str =
    "mnemonist-rs: this port's Map supports undefined, null, boolean, number and string keys. \
     Object, symbol, function and bigint keys need identity comparison, which is not implemented \
     -- see docs/modules/default-map.md.";

/// A JavaScript value used as a `Map` key.
///
/// `Eq` and `Hash` are derived, and are SameValueZero because
/// [`JsKey::Number`] holds a normalised bit pattern. See the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum JsKey {
    Undefined,
    Null,
    Bool(bool),
    /// `f64::to_bits` of a **normalised** double: no `-0.0`, one `NaN`.
    Number(u64),
    /// `Rc<str>` rather than `String`: core's index holds a second copy of
    /// every key, and every `keys()` step hands one out, so the clone has to
    /// be a refcount bump rather than a copy of the text. `Rc<str>` hashes and
    /// compares as its `str`, so this is invisible to SameValueZero.
    String(Rc<str>),
}

impl JsKey {
    /// Fold the two values SameValueZero treats as one onto a single bit
    /// pattern each.
    ///
    /// `-0.0 == 0.0` is true in Rust, so the zero test catches both zeroes;
    /// `f64::NAN` is Rust's one canonical quiet NaN, so every incoming NaN
    /// payload collapses onto it.
    fn number(value: f64) -> Self {
        let normalised = if value == 0.0 {
            0.0
        } else if value.is_nan() {
            f64::NAN
        } else {
            value
        };

        Self::Number(normalised.to_bits())
    }

    /// Classify a JS value, rejecting the types this port cannot key on.
    pub fn from_unknown(value: &Unknown) -> Result<Self> {
        match value.get_type()? {
            ValueType::Undefined => Ok(Self::Undefined),
            ValueType::Null => Ok(Self::Null),
            // SAFETY (x3): `get_type` has just reported this exact type, which
            // is the precondition `Unknown::cast` documents.
            ValueType::Boolean => Ok(Self::Bool(unsafe { value.cast::<bool>()? })),
            ValueType::Number => Ok(Self::number(unsafe { value.cast::<f64>()? })),
            ValueType::String => Ok(Self::String(Rc::from(unsafe { value.cast::<String>()? }))),
            _ => Err(Error::new(Status::InvalidArg, UNSUPPORTED)),
        }
    }
}

impl TypeName for JsKey {
    fn type_name() -> &'static str {
        "JsKey"
    }

    fn value_type() -> ValueType {
        ValueType::Unknown
    }
}

impl ValidateNapiValue for JsKey {}

impl FromNapiValue for JsKey {
    unsafe fn from_napi_value(env: sys::napi_env, value: sys::napi_value) -> Result<Self> {
        let unknown = unsafe { Unknown::from_napi_value(env, value)? };

        Self::from_unknown(&unknown)
    }
}

impl ToNapiValue for &JsKey {
    /// Every arm delegates to napi's own conversion for the corresponding
    /// Rust type, so nothing here constructs a JS value by hand.
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        unsafe {
            match val {
                JsKey::Undefined => ToNapiValue::to_napi_value(env, ()),
                JsKey::Null => ToNapiValue::to_napi_value(env, Null),
                JsKey::Bool(value) => ToNapiValue::to_napi_value(env, *value),
                JsKey::Number(bits) => ToNapiValue::to_napi_value(env, f64::from_bits(*bits)),
                JsKey::String(value) => ToNapiValue::to_napi_value(env, &**value),
            }
        }
    }
}

impl ToNapiValue for JsKey {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        unsafe { ToNapiValue::to_napi_value(env, &val) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn hash_of(key: &JsKey) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);

        hasher.finish()
    }

    /// Both halves of every claim: equal keys must also hash equally, or the
    /// map silently grows two entries for one key.
    fn same_key(left: &JsKey, right: &JsKey) {
        assert_eq!(left, right, "{left:?} and {right:?} should be one key");
        assert_eq!(
            hash_of(left),
            hash_of(right),
            "{left:?} and {right:?} are equal but hash differently"
        );
    }

    fn different_keys(left: &JsKey, right: &JsKey) {
        assert_ne!(left, right, "{left:?} and {right:?} should be two keys");
    }

    #[test]
    fn nan_is_the_same_key_as_nan() {
        same_key(&JsKey::number(f64::NAN), &JsKey::number(f64::NAN));
    }

    /// A different NaN *payload* is still `NaN` to JavaScript, which has one
    /// observable NaN. Normalisation is what makes that true here too.
    #[test]
    fn every_nan_payload_folds_onto_one_key() {
        let exotic = f64::from_bits(0x7ff8_0000_dead_beef);
        assert!(exotic.is_nan());

        same_key(&JsKey::number(exotic), &JsKey::number(f64::NAN));
    }

    #[test]
    fn negative_zero_is_the_same_key_as_positive_zero() {
        same_key(&JsKey::number(-0.0), &JsKey::number(0.0));
    }

    /// …and the stored form is `+0`, so it comes back out of `keys()` as `0`,
    /// exactly as `Map` does.
    #[test]
    fn negative_zero_is_stored_as_positive_zero() {
        assert_eq!(JsKey::number(-0.0), JsKey::Number(0.0f64.to_bits()));
        assert!(!f64::from_bits(0.0f64.to_bits()).is_sign_negative());
    }

    #[test]
    fn ordinary_numbers_are_distinct_and_integral_forms_coincide() {
        different_keys(&JsKey::number(1.0), &JsKey::number(2.0));
        same_key(&JsKey::number(3.0), &JsKey::number(3.0));
        // `3` and `3.0` are one number in JavaScript.
        same_key(&JsKey::number(3f64), &JsKey::number(6.0 / 2.0));
    }

    #[test]
    fn infinities_are_keys_and_are_not_each_other() {
        same_key(&JsKey::number(f64::INFINITY), &JsKey::number(f64::INFINITY));
        different_keys(
            &JsKey::number(f64::INFINITY),
            &JsKey::number(f64::NEG_INFINITY),
        );
        different_keys(&JsKey::number(f64::INFINITY), &JsKey::number(f64::NAN));
    }

    /// The five primitive shapes are five different keys even when JavaScript
    /// would coerce between them: `Map` never coerces.
    #[test]
    fn the_primitive_shapes_do_not_collide() {
        let keys = [
            JsKey::Undefined,
            JsKey::Null,
            JsKey::Bool(false),
            JsKey::Bool(true),
            JsKey::number(0.0),
            JsKey::String(Rc::from("")),
            JsKey::String(Rc::from("0")),
            JsKey::String(Rc::from("false")),
        ];

        for (index, left) in keys.iter().enumerate() {
            for right in &keys[index + 1..] {
                different_keys(left, right);
            }
        }
    }

    #[test]
    fn strings_are_compared_by_content() {
        same_key(
            &JsKey::String(Rc::from("hello")),
            &JsKey::String(Rc::from(String::from("hello"))),
        );
        different_keys(
            &JsKey::String(Rc::from("hello")),
            &JsKey::String(Rc::from("Hello")),
        );
    }
}

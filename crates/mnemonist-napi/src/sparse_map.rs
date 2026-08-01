//! JS bridge for [`mnemonist_core::structures::sparse_map`].
//!
//! Thin translation only; every behavioural decision lives in the core crate.
//! Four adaptations, two of them larger than anything `sparse-set` needed.
//!
//! 1. **The overloaded constructor.** Upstream is
//!    `function SparseMap(Values, length)` with
//!    `if (arguments.length < 2) { length = Values; Values = Array; }`. napi's
//!    typed signature cannot see `arguments.length`, but it can see whether the
//!    second argument is present, which for this constructor is the same
//!    question. See [`JsSparseMap::new`] for the four cases and what each does.
//!
//! 2. **`Values` is a JS constructor, and Rust has no such runtime value.** It
//!    is resolved by *identity* against `globalThis` — `strict_equals` against
//!    `Array`, `Uint8Array`, `Uint16Array` and `Uint32Array` — rather than by
//!    reading `.name`, which any object can forge. The signed and floating
//!    typed arrays are **not** supported and say so; `mnemonist-core`'s
//!    `PointerVec` models the three unsigned widths and nothing else. Upstream
//!    accepts them, so this is a stated divergence, not an oversight.
//!
//! 3. **Values are JS numbers.** The core is generic over the value type, and
//!    the bridge instantiates it at `f64` — every JS number, and nothing else.
//!    Storing a string or an object is DESIGN.md 3.3's T3 tier (arbitrary JS
//!    values, which need a per-slot `Ref` and an `Env` to drop it), and this
//!    module does not reach for it. The upstream test file stores only numbers.
//!
//! 4. **`dense`, `sparse` and `vals` are not exposed.** As in `sparse-set`:
//!    they are public upstream and a JS caller can write *through* them, but
//!    napi can only hand out a copy, which would silently break write-through.
//!    They are exposed in Rust, and the differential fuzzer compares all three
//!    slot for slot after every op.

use mnemonist_core::cursor::Step;
use mnemonist_core::structures::sparse_map::{Projected, Projection, SparseMap as CoreMap};
use mnemonist_core::utils::typed_arrays::{PointerWidth, POINTER_ARRAY_TOO_LARGE};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::cursor::{yielded, BridgeCursor};

/// The value array constructors this bridge can resolve, and the width each
/// one maps to. `None` is `Array`, which is not a width at all.
const VALUE_CONSTRUCTORS: &[(&str, Option<PointerWidth>)] = &[
    ("Array", None),
    ("Uint8Array", Some(PointerWidth::U8)),
    ("Uint16Array", Some(PointerWidth::U16)),
    ("Uint32Array", Some(PointerWidth::U32)),
];

/// What upstream's `forEach` callback is invoked with: `(value, key)`, in that
/// order, and either of them possibly `undefined`.
///
/// A named alias rather than the type spelled inline, because spelled inline it
/// is four nested generics and clippy's `type_complexity` is right about it.
type ForEachArgs = FnArgs<(Either<f64, Undefined>, Either<u32, Undefined>)>;

/// A map from the members `0..length` to JS numbers.
#[napi(js_name = "SparseMap")]
pub struct JsSparseMap {
    inner: CoreMap<f64>,
}

#[napi]
impl JsSparseMap {
    /// `new SparseMap(length)` or `new SparseMap(Values, length)`.
    ///
    /// Upstream branches on `arguments.length < 2`, which napi cannot observe.
    /// It can observe whether the second parameter arrived, and the two agree
    /// on every call except `new SparseMap(x, undefined)` — where upstream sees
    /// two arguments and this sees one. That is the same
    /// omitted-versus-`undefined` blind spot as `SparseSet.forEach`'s `scope`,
    /// and it is recorded rather than papered over.
    ///
    /// The four shapes, each matching what upstream does with it:
    ///
    /// | call | here |
    /// |---|---|
    /// | `new SparseMap(10)` | `Array` store, length 10 |
    /// | `new SparseMap(Uint8Array, 10)` | 8-bit store, length 10 |
    /// | `new SparseMap(Uint8Array)` | throws the pointer-array message — upstream computes `getPointerArray(Uint8Array)`, whose `size - 1` is `NaN`, and every comparison against `NaN` is false, so it falls through to exactly that `throw` |
    /// | `new SparseMap(10, 20)` | throws "Values is not a constructor" — upstream reaches `new (10)(20)` |
    #[napi(constructor)]
    pub fn new(env: Env, values: Either<u32, Unknown>, length: Option<u32>) -> Result<Self> {
        let (constructor, length) = match (values, length) {
            // `new SparseMap(length)` — the default store.
            (Either::A(length), None) => (None, length),
            // `new SparseMap(Values, length)`.
            (Either::B(constructor), Some(length)) => (Some(constructor), length),
            // `new SparseMap(Values)`: upstream takes the constructor as the
            // length and `getPointerArray` throws on the resulting NaN.
            (Either::B(_), None) => return Err(failure(POINTER_ARRAY_TOO_LARGE)),
            // `new SparseMap(10, 20)`: upstream reaches `new (10)(20)`.
            (Either::A(_), Some(_)) => {
                return Err(Error::new(
                    Status::InvalidArg,
                    "SparseMap: Values is not a constructor",
                ))
            }
        };

        let width = match constructor {
            None => None,
            Some(constructor) => resolve_value_constructor(&env, constructor)?,
        };

        let inner = match width {
            None => CoreMap::array(length as usize),
            Some(width) => CoreMap::typed(length as usize, width),
        };

        inner.map(|inner| Self { inner }).map_err(failure)
    }

    /// Entries currently in the map. Can exceed `length`; see the core docs.
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.size() as u32
    }

    /// Capacity the map was built with.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.length() as u32
    }

    #[napi]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    #[napi]
    pub fn has(&self, member: u32) -> bool {
        self.inner.has(member as usize)
    }

    /// `Either<f64, Undefined>` rather than `Option<f64>`: napi renders `None`
    /// as `null`, and upstream's `get` of an absent member is `undefined`,
    /// which `assert.strictEqual` distinguishes.
    #[napi]
    pub fn get(&self, member: u32) -> Either<f64, Undefined> {
        self.inner.get(member as usize).into()
    }

    /// Upstream returns `this` for chaining.
    #[napi]
    pub fn set<'a>(&mut self, this: This<'a>, member: u32, value: f64) -> This<'a> {
        self.inner.set(member as usize, value);

        this
    }

    #[napi(js_name = "delete")]
    pub fn delete(&mut self, member: u32) -> bool {
        self.inner.delete(member as usize)
    }

    /// A fresh cursor over the members — upstream's `keys()`.
    #[napi]
    pub fn keys(&self, env: Env, this: Reference<JsSparseMap>) -> Result<JsSparseMapKeys> {
        Ok(JsSparseMapKeys {
            cursor: BridgeCursor::open_projected(project(env, this)?, Projection::Keys),
        })
    }

    /// A fresh cursor over the values — upstream's `values()`.
    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsSparseMap>) -> Result<JsSparseMapValues> {
        Ok(JsSparseMapValues {
            cursor: BridgeCursor::open_projected(project(env, this)?, Projection::Values),
        })
    }

    /// A fresh cursor over `[key, value]` pairs — upstream's `entries()`, and
    /// the method `Symbol.iterator` is aliased to (see `crate::cursor`).
    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsSparseMap>) -> Result<JsSparseMapEntries> {
        Ok(JsSparseMapEntries {
            cursor: BridgeCursor::open_projected(project(env, this)?, Projection::Entries),
        })
    }

    /// Upstream's own `forEach` — a plain loop over the two arrays.
    ///
    /// The callback is `(value, key)`, in that order: upstream writes
    /// `callback.call(scope, this.vals[i], this.dense[i])`, so the *value* is
    /// the first argument. Both may be `undefined` once `size` has run past
    /// `length`, which is why neither is a bare number here.
    ///
    /// The loop re-reads `this.size` on every iteration, matching upstream and
    /// **not** matching the frozen-length cursors above; a callback that
    /// deletes entries shortens this loop and would not shorten a walk. That
    /// difference is upstream's.
    ///
    /// `scope` carries the same `arguments.length > 1 ? scope : this` blind
    /// spot as `SparseSet.forEach`: `forEach(cb, undefined)` binds the map
    /// here where upstream binds `undefined`. The omitted-argument case, which
    /// is the only one the original suite uses, is exact.
    #[napi]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<ForEachArgs, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let mut index = 0;

        while index < self.inner.size() {
            let value: Either<f64, Undefined> = self.inner.vals().slot(index).into();
            let key: Either<u32, Undefined> = self.inner.dense().try_get(index).into();

            match &scope {
                Some(scope) => callback.apply(*scope, (value, key).into())?,
                None => callback.apply(this, (value, key).into())?,
            };

            index += 1;
        }

        Ok(())
    }
}

/// The cursor `SparseMap.prototype.keys()` hands out.
#[napi(iterator, js_name = "SparseMapKeys")]
pub struct JsSparseMapKeys {
    cursor: BridgeCursor<JsSparseMap, CoreMap<f64>>,
}

impl Generator for JsSparseMapKeys {
    type Yield = Either<u32, Undefined>;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        yielded(self.cursor.step().map(|step| match step {
            Projected::Key(key) => key,
            other => unreachable!("a Keys projection cannot yield {other:?}"),
        }))
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}

/// The cursor `SparseMap.prototype.values()` hands out.
#[napi(iterator, js_name = "SparseMapValues")]
pub struct JsSparseMapValues {
    cursor: BridgeCursor<JsSparseMap, CoreMap<f64>>,
}

impl Generator for JsSparseMapValues {
    type Yield = Either<f64, Undefined>;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        yielded(self.cursor.step().map(|step| match step {
            Projected::Value(value) => value,
            other => unreachable!("a Values projection cannot yield {other:?}"),
        }))
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}

/// The cursor `SparseMap.prototype.entries()` hands out, and the one
/// `[...map]` goes through.
///
/// The only cursor in the port whose `Yield` is not an `Either`: upstream
/// builds the pair `[dense[i], vals[i]]` and yields **the array**, so a missing
/// half is `undefined` *inside* a yielded value and the step itself is never a
/// gap. `Option::None` therefore keeps its plain meaning of `{done: true}`.
#[napi(iterator, js_name = "SparseMapEntries")]
pub struct JsSparseMapEntries {
    cursor: BridgeCursor<JsSparseMap, CoreMap<f64>>,
}

impl Generator for JsSparseMapEntries {
    type Yield = Vec<Either<f64, Undefined>>;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        match self.cursor.step() {
            Step::Item(Projected::Entry(key, value)) => {
                Some(vec![key.map(f64::from).into(), value.into()])
            }
            Step::Item(other) => unreachable!("an Entries projection cannot yield {other:?}"),
            // Unreachable through this projection: `slot` always returns the
            // pair. Kept as `{done: true}` rather than an `unreachable!`
            // because a gap is a legitimate `Step` and panicking across the FFI
            // boundary to say so would be worse than terminating.
            Step::Gap | Step::Done => None,
        }
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}

/// Keep the JS-owned instance alive and project to the core map inside it.
///
/// Split out because all three cursor factories need it identically; see
/// `JsSparseSet::values` for what `share_with` is doing and why the resulting
/// shared borrow deliberately coexists with `&mut self`.
fn project(
    env: Env,
    this: Reference<JsSparseMap>,
) -> Result<SharedReference<JsSparseMap, &'static CoreMap<f64>>> {
    this.share_with(env, |map| Ok(&map.inner))
}

/// Which value store `Values` names, or an error naming what is supported.
///
/// Resolved by **identity**, not by `.name`: `{name: 'Uint8Array'}` is trivial
/// to forge and `strict_equals` against the real global is not. Returning
/// `Ok(None)` means `Array`.
fn resolve_value_constructor(env: &Env, values: Unknown) -> Result<Option<PointerWidth>> {
    let global = env.get_global()?;

    for (name, width) in VALUE_CONSTRUCTORS {
        // `_unchecked` because a constructor is a JS *function*, and the
        // checked getter validates for `Object`. Same reason as
        // `crate::cursor::install_iterator_factories`.
        let candidate: Unknown = global.get_named_property_unchecked(name)?;

        if env.strict_equals(values, candidate)? {
            return Ok(*width);
        }
    }

    Err(Error::new(
        Status::InvalidArg,
        format!(
            "SparseMap: unsupported value array constructor. This port models the \
             three unsigned typed-array widths and a plain Array, so `Values` must be \
             one of {}. Upstream accepts any array constructor; the signed and \
             floating widths are a documented gap, not a rejection of your input.",
            VALUE_CONSTRUCTORS
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ))
}

fn failure(message: &str) -> Error {
    Error::new(Status::GenericFailure, message.to_owned())
}

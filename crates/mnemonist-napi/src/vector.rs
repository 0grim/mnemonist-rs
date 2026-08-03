//! JS bridge for [`mnemonist_core::structures::vector`].
//!
//! Thin translation only: every behavioural decision lives in the core crate.
//! Four adaptations are worth knowing about.
//!
//! 1. **`ArrayClass` is resolved by identity against four globals.** Upstream
//!    accepts any array-like constructor; this port models
//!    `Uint8Array`/`Uint16Array`/`Uint32Array`/`Float64Array` (see the core
//!    module docs for the full scope cut) and refuses everything else,
//!    naming the supported set — the same call `sparse-map`'s bridge makes
//!    for its `Values` constructor.
//! 2. **`Vector.PointerVector` has no real `ArrayClass` value to resolve.**
//!    Upstream's `pointerArrayFactory` is a private function inside
//!    `vector.js`, reachable only through `Vector.PointerVector =
//!    subClass(pointerArrayFactory)` — there is no global a caller could pass
//!    to the base constructor to reach it. So `PointerVector` gets its own
//!    pair of hidden factories ([`JsVector::pointer_vector`],
//!    [`JsVector::pointer_vector_from`]), and [`install_vector_subclasses`]
//!    wires `Vector.PointerVector` (and the four `Vector.<Width>Vector`
//!    convenience subclasses this port supports) onto the exported class at
//!    load time, in JavaScript, exactly as `crate::statics` does for `X.of`.
//! 3. **The growth policy is a JS function called from Rust; see
//!    `crate::bit_vector`.** Same `JsPolicy`/`Rc<RefCell<Option<Error>>>`
//!    shape, for the same reason: a JS policy can throw, which no `Option`
//!    can express, and every method that can trigger a growth
//!    (`push`/`grow`/`resize`/`reallocate`/`applyPolicy`) holds the vector
//!    across a JS call that may re-enter it. A `RefCell` panic inside a
//!    `#[napi]` method aborts the process (napi 3.12 does not
//!    `catch_unwind` a sync call), so every borrow here is fallible.
//! 4. **`Vector.from`'s pushed values are coerced with `ToNumber`, not
//!    checked to already be numbers.** `push`/`set` themselves take a typed
//!    `f64` parameter, which napi refuses for a non-number argument --
//!    slightly narrower than upstream's implicit typed-array coercion, and
//!    the same simplification `hashed-array-tree`'s bridge makes.
//!
//! The core structure is held in a [`RefCell`] for the same reason as
//! [`crate::stack`]/[`crate::queue`]/[`crate::bit_vector`]: a plain `&self` is
//! `noalias readonly` to LLVM, and a JS growth policy or a `forEach` callback
//! can mutate the vector while a `&self` method is still on the stack. See
//! `crate::cursor::CellCursor` and PORTBUG-1.

use std::cell::RefCell;
use std::rc::Rc;

use mnemonist_core::structures::vector::{
    default_policy, Error as CoreError, Storage, Vector as CoreVector, MISSING_ARRAY_CLASS,
};
use mnemonist_core::utils::typed_arrays::{PointerVec, PointerWidth};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::cursor::{yielded, CellCursor};
use crate::foreach;

/// Raised when a growth policy calls back into the vector that is growing.
/// Same wording class as `crate::bit_vector`'s `REENTRANT_POLICY`.
const REENTRANT_POLICY: &str = "mnemonist-rs/Vector: the growth policy called back into the \
     vector while it was growing. Upstream serves such a call from a half-grown vector; \
     this port refuses it, because the vector is mid-operation and cannot answer honestly. \
     See the module docs and PORTBUG-1.";

/// `Vector.from`'s own message when neither a capacity nor a guessable
/// iterable is given.
const CANNOT_GUESS_LENGTH: &str = "mnemonist/vector.from: could not guess iterable length. \
     Please provide desired capacity as last argument.";

/// The concrete backing this bridge resolved `ArrayClass` to, plus the name
/// upstream would report as `this.ArrayClass.name` -- which is what the
/// `set` out-of-bounds message interpolates.
#[derive(Clone, Copy)]
enum Kind {
    Fixed(PointerWidth),
    F64,
}

/// The four globals this port resolves `ArrayClass` to, the width each
/// names, and the convenience subclass name it gets installed under.
/// `Float64Array` is not a [`PointerWidth`] at all, hence [`Kind`] rather
/// than reusing `sparse_map`'s `Option<PointerWidth>` shape.
const ARRAY_CLASSES: &[(&str, Kind, &str)] = &[
    ("Uint8Array", Kind::Fixed(PointerWidth::U8), "Uint8Vector"),
    (
        "Uint16Array",
        Kind::Fixed(PointerWidth::U16),
        "Uint16Vector",
    ),
    (
        "Uint32Array",
        Kind::Fixed(PointerWidth::U32),
        "Uint32Vector",
    ),
    ("Float64Array", Kind::F64, "Float64Vector"),
];

/// A growable array over a typed-array-like backing store.
#[napi(js_name = "Vector")]
pub struct JsVector {
    inner: RefCell<CoreVector>,
    /// Where [`JsPolicy`] leaves an exception thrown by the JS policy.
    thrown: Rc<RefCell<Option<Error>>>,
}

#[napi]
impl JsVector {
    /// `new Vector(ArrayClass, initialCapacityOrOptions)`.
    #[napi(constructor)]
    pub fn new(
        env: Env,
        array_class: Option<Unknown>,
        initial_capacity_or_options: Option<Either<f64, Object>>,
    ) -> Result<Self> {
        // `arguments.length < 1`, which napi cannot see directly but can
        // observe through an omitted first parameter.
        let array_class =
            array_class.ok_or_else(|| Error::new(Status::InvalidArg, MISSING_ARRAY_CLASS))?;
        let (kind, _class_name) = resolve_array_class(&env, &array_class)?;
        let (capacity, length, policy) = resolve_options(&env, initial_capacity_or_options)?;
        let thrown = Rc::new(RefCell::new(None));

        let inner = build(kind, capacity, length, wrap_policy(policy, env, &thrown));

        Ok(Self {
            inner: RefCell::new(inner),
            thrown,
        })
    }

    /// `new Vector.PointerVector(initialCapacityOrOptions)`. Installed onto
    /// `Vector.PointerVector` by [`install_vector_subclasses`]; not part of
    /// the class's own public shape (upstream has no `ArrayClass` value for
    /// a caller to reach this through the base constructor either).
    #[napi(factory, js_name = "__pointerVector")]
    pub fn pointer_vector(
        env: Env,
        initial_capacity_or_options: Option<Either<f64, Object>>,
    ) -> Result<Self> {
        let (capacity, length, policy) = resolve_options(&env, initial_capacity_or_options)?;
        let thrown = Rc::new(RefCell::new(None));
        let policy = wrap_policy(policy, env, &thrown);

        CoreVector::pointer_with_policy(capacity, length, policy)
            .map(|inner| Self {
                inner: RefCell::new(inner),
                thrown,
            })
            .map_err(raise)
    }

    #[napi(getter)]
    pub fn length(&self) -> Result<u32> {
        Ok(self.read()?.length() as u32)
    }

    #[napi(getter)]
    pub fn capacity(&self) -> Result<u32> {
        Ok(self.read()?.capacity() as u32)
    }

    /// The backing typed array. A **copy** -- upstream's own test only ever
    /// reads it for an `instanceof` check
    /// (`assert(vector.array instanceof Uint8Array)`), which a copy answers
    /// exactly as well as write-through would, and napi cannot hand out a
    /// live view onto a Rust `Vec` regardless.
    #[napi(getter)]
    pub fn array(&self) -> Result<Either4<Uint8Array, Uint16Array, Uint32Array, Float64Array>> {
        Ok(match self.read()?.storage() {
            Storage::Fixed(values) | Storage::Pointer(values) => match values {
                PointerVec::U8(values) => Either4::A(Uint8Array::new(values.clone())),
                PointerVec::U16(values) => Either4::B(Uint16Array::new(values.clone())),
                PointerVec::U32(values) => Either4::C(Uint32Array::new(values.clone())),
            },
            Storage::F64(values) => Either4::D(Float64Array::new(values.clone())),
        })
    }

    /// `#.set(index, value)`. Upstream returns `this`.
    #[napi]
    pub fn set<'a>(&self, this: This<'a>, index: f64, value: f64) -> Result<This<'a>> {
        self.write()?
            .set(count(index), value)
            .map_err(|error| self.raise(error))?;

        Ok(this)
    }

    /// `undefined` on an out-of-bound read -- `Either`, not `Option`: napi
    /// renders `None` as `null` and upstream's miss is `undefined` (DIV-FIXED-STACK-1).
    #[napi]
    pub fn get(&self, index: f64) -> Result<Either<f64, Undefined>> {
        Ok(match self.read()?.get(count(index)) {
            Some(value) => Either::A(value),
            None => Either::B(()),
        })
    }

    #[napi]
    pub fn apply_policy(&self, override_capacity: Option<f64>) -> Result<u32> {
        // The borrow ends before `raise`, which — for a policy that
        // re-enters — must not find the vector locked either.
        let outcome = self.read()?.apply_policy(override_capacity.map(count));

        outcome
            .map(|capacity| capacity as u32)
            .map_err(|error| self.raise(error))
    }

    #[napi]
    pub fn reallocate<'a>(&self, this: This<'a>, capacity: f64) -> Result<This<'a>> {
        self.write()?
            .reallocate(count(capacity))
            .map_err(|error| self.raise(error))?;

        Ok(this)
    }

    #[napi]
    pub fn grow<'a>(&self, this: This<'a>, capacity: Option<f64>) -> Result<This<'a>> {
        self.write()?
            .grow(capacity.map(count))
            .map_err(|error| self.raise(error))?;

        Ok(this)
    }

    #[napi]
    pub fn resize<'a>(&self, this: This<'a>, length: f64) -> Result<This<'a>> {
        self.write()?
            .resize(count(length))
            .map_err(|error| self.raise(error))?;

        Ok(this)
    }

    /// Returns the new length, as upstream does.
    #[napi]
    pub fn push(&self, value: f64) -> Result<u32> {
        self.write()?
            .push(value)
            .map(|length| length as u32)
            .map_err(|error| self.raise(error))
    }

    /// `undefined` on an empty vector, which is upstream's bare `return;`.
    #[napi]
    pub fn pop(&self) -> Result<Either<f64, Undefined>> {
        Ok(match self.write()?.pop() {
            Some(value) => Either::A(value),
            None => Either::B(()),
        })
    }

    /// A fresh cursor over the values, upstream's `values()` and the method
    /// `Symbol.iterator` is aliased to (see `crate::cursor`).
    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsVector>) -> Result<JsVectorValues> {
        let source = this.share_with(env, |vector| Ok(&vector.inner))?;

        Ok(JsVectorValues {
            cursor: CellCursor::open(source),
        })
    }

    /// A fresh cursor over `[index, value]` pairs, upstream's `entries()`.
    #[napi]
    pub fn entries(&self, env: Env, this: Reference<JsVector>) -> Result<JsVectorEntries> {
        let source = this.share_with(env, |vector| Ok(&vector.inner))?;

        Ok(JsVectorEntries {
            cursor: CellCursor::open(source),
            index: 0,
        })
    }

    /// `Vector.from(iterable, ArrayClass, capacity)`.
    #[napi(factory)]
    pub fn from(
        env: Env,
        iterable: Unknown,
        array_class: Unknown,
        capacity: Option<f64>,
    ) -> Result<Self> {
        let (kind, _class_name) = resolve_array_class(&env, &array_class)?;

        // `arguments.length < 3`: guess when the capacity is omitted.
        let capacity = match capacity {
            Some(capacity) => count(capacity),
            None => match crate::iterables::guess_length(&env, &iterable)? {
                Some(length) => count(length),
                None => {
                    return Err(Error::new(Status::InvalidArg, CANNOT_GUESS_LENGTH));
                }
            },
        };

        let mut inner = build(kind, capacity, 0, Box::new(default_policy));

        for slot in foreach::collect(&env, iterable)? {
            let value = foreach::to_number(&env, &slot.get(&env)?)?;

            inner.push(value).map_err(raise)?;
        }

        Ok(Self {
            inner: RefCell::new(inner),
            thrown: Rc::new(RefCell::new(None)),
        })
    }

    /// `Vector.PointerVector.from(iterable, capacity)`. Installed by
    /// [`install_vector_subclasses`]; see [`JsVector::pointer_vector`].
    #[napi(factory, js_name = "__pointerVectorFrom")]
    pub fn pointer_vector_from(env: Env, iterable: Unknown, capacity: Option<f64>) -> Result<Self> {
        let capacity = match capacity {
            Some(capacity) => count(capacity),
            None => match crate::iterables::guess_length(&env, &iterable)? {
                Some(length) => count(length),
                None => {
                    return Err(Error::new(Status::InvalidArg, CANNOT_GUESS_LENGTH));
                }
            },
        };

        let mut inner = CoreVector::pointer(capacity, 0).map_err(raise)?;

        for slot in foreach::collect(&env, iterable)? {
            let value = foreach::to_number(&env, &slot.get(&env)?)?;

            inner.push(value).map_err(raise)?;
        }

        Ok(Self {
            inner: RefCell::new(inner),
            thrown: Rc::new(RefCell::new(None)),
        })
    }

    /// A shared borrow, or the re-entrancy error. Never `borrow()`: see
    /// `crate::bit_vector`'s module docs for why a panic here would take the
    /// process down rather than reach JavaScript.
    fn read(&self) -> Result<std::cell::Ref<'_, CoreVector>> {
        self.inner
            .try_borrow()
            .map_err(|_| Error::new(Status::GenericFailure, REENTRANT_POLICY))
    }

    fn write(&self) -> Result<std::cell::RefMut<'_, CoreVector>> {
        self.inner
            .try_borrow_mut()
            .map_err(|_| Error::new(Status::GenericFailure, REENTRANT_POLICY))
    }

    /// Prefer an exception thrown *by the JS policy* over the core's
    /// classification of its result. The core's own `Display` already
    /// interpolates `this.ArrayClass.name` where upstream does (see
    /// `mnemonist_core::structures::vector::Storage::class_name`), so nothing
    /// else needs assembling here.
    fn raise(&self, error: CoreError) -> Error {
        if let Some(thrown) = self.thrown.borrow_mut().take() {
            return thrown;
        }

        Error::new(Status::GenericFailure, error.to_string())
    }
}

/// The cursor `Vector.prototype.values()` hands out.
#[napi(iterator, js_name = "VectorValues")]
pub struct JsVectorValues {
    cursor: CellCursor<JsVector, CoreVector>,
}

impl Generator for JsVectorValues {
    type Yield = Either<f64, Undefined>;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        yielded(self.cursor.step())
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}

/// The cursor `Vector.prototype.entries()` hands out.
#[napi(iterator, js_name = "VectorEntries")]
pub struct JsVectorEntries {
    cursor: CellCursor<JsVector, CoreVector>,
    index: u32,
}

impl Generator for JsVectorEntries {
    type Yield = (u32, Either<f64, Undefined>);
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        use mnemonist_core::cursor::Step;

        let value: Either<f64, Undefined> = match self.cursor.step() {
            Step::Item(value) => Either::A(value),
            Step::Gap => Either::B(()),
            Step::Done => return None,
        };
        let index = self.index;

        self.index += 1;

        Some((index, value))
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Self::Yield> {
        None
    }
}

fn build(
    kind: Kind,
    capacity: usize,
    length: usize,
    policy: mnemonist_core::structures::vector::Policy,
) -> CoreVector {
    match kind {
        Kind::Fixed(width) => CoreVector::fixed_with_policy(width, capacity, length, policy),
        Kind::F64 => CoreVector::f64_with_policy(capacity, length, policy),
    }
}

/// Which of the four supported globals `ArrayClass` names, by identity --
/// not by `.name`, which any object can forge. See the module docs for what
/// this port does and does not model.
fn resolve_array_class(env: &Env, array_class: &Unknown) -> Result<(Kind, &'static str)> {
    let global = env.get_global()?;

    for (name, kind, _) in ARRAY_CLASSES {
        let candidate: Unknown = global.get_named_property_unchecked(name)?;

        if env.strict_equals(*array_class, candidate)? {
            return Ok((*kind, name));
        }
    }

    Err(Error::new(
        Status::InvalidArg,
        format!(
            "Vector: unsupported array class. This port models {}. Upstream accepts \
             any array-like constructor; the signed and clamped typed-array widths, \
             Float32Array, a plain Array backing and a caller-supplied factory are a \
             documented gap.",
            ARRAY_CLASSES
                .iter()
                .map(|(name, _, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ))
}

/// The `initialCapacityOrOptions` union: a bare number, or
/// `{initialCapacity, initialLength, policy}`. `factory` is deliberately
/// unread here -- see the module docs on `Vector.PointerVector`, which is the
/// only place this port honours it.
#[allow(clippy::type_complexity)]
fn resolve_options(
    env: &Env,
    initial_capacity_or_options: Option<Either<f64, Object>>,
) -> Result<(
    usize,
    usize,
    Option<Function<'static, f64, Unknown<'static>>>,
)> {
    match initial_capacity_or_options {
        None => Ok((0, 0, None)),
        Some(Either::A(capacity)) => Ok((count(capacity), 0, None)),
        Some(Either::B(options)) => {
            let capacity = field(&options, "initialCapacity")?;
            let length = field(&options, "initialLength")?;
            let policy = options.get::<Function<f64, Unknown<'static>>>("policy")?;

            let _ = env; // kept for symmetry with sibling bridges' signatures

            Ok((capacity, length, policy))
        }
    }
}

/// One option field, defaulting to `0` exactly as upstream's `|| 0` does.
fn field(options: &Object, name: &str) -> Result<usize> {
    Ok(options.get::<f64>(name)?.map_or(0, count))
}

/// A JS number used as a length/capacity/index. Same treatment as the
/// `HashedArrayTree`/`BitVector` bridges: truncate, and clamp what a `usize`
/// cannot hold.
fn count(value: f64) -> usize {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }

    value.trunc() as usize
}

fn wrap_policy(
    policy: Option<Function<'static, f64, Unknown<'static>>>,
    env: Env,
    thrown: &Rc<RefCell<Option<Error>>>,
) -> mnemonist_core::structures::vector::Policy {
    match policy {
        None => Box::new(default_policy),
        Some(policy) => {
            let policy = JsPolicy {
                callable: match policy.create_ref() {
                    Ok(callable) => callable,
                    // A `create_ref` failure here would have to be surfaced
                    // at construction time; upstream's own policy is stored
                    // without any such fallibility, so this mirrors
                    // `crate::bit_vector`'s treatment: fall back to the
                    // default rather than lose the constructor's `Result`
                    // shape over an the-world-is-ending allocator failure.
                    Err(_) => return Box::new(default_policy),
                },
                env,
                thrown: Rc::clone(thrown),
            };

            Box::new(move |capacity| policy.call(capacity))
        }
    }
}

/// A JS growth policy, callable from the core's `Box<dyn Fn>`. Identical
/// shape to `crate::bit_vector::JsPolicy`.
struct JsPolicy {
    callable: FunctionRef<f64, Unknown<'static>>,
    env: Env,
    thrown: Rc<RefCell<Option<Error>>>,
}

impl JsPolicy {
    fn call(&self, capacity: f64) -> Option<f64> {
        let result = self
            .callable
            .borrow_back(&self.env)
            .and_then(|callable| callable.call(capacity));

        match result {
            Err(error) => {
                *self.thrown.borrow_mut() = Some(error);
                None
            }
            Ok(value) => match value.get_type() {
                Ok(ValueType::Number) => f64::from_unknown(value).ok(),
                _ => None,
            },
        }
    }
}

fn raise(error: CoreError) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

/// Install `Vector.Uint8Vector`/`Uint16Vector`/`Uint32Vector`/`Float64Vector`
/// and `Vector.PointerVector`, plus each subclass's `.from`.
///
/// Upstream's `subClass()` builds a fresh constructor whose prototype is a
/// **copy** of `Vector.prototype`'s own methods, not a real subclass -- a
/// `Vector.Uint8Vector` instance is not `instanceof Vector`. This installer
/// gets the same *observable* shape more simply: each wrapper's body
/// `return`s a real `JsVector` (built by the base constructor, or by the
/// hidden `__pointerVector*` factories for `PointerVector`), and a
/// constructor that explicitly returns an object is used as that object by
/// `new` -- so `new Vector.Uint8Vector(5)` works without `Uint8Vector`
/// sharing any prototype with `Vector` at all, exactly as upstream's
/// `SubClass` does not.
///
/// Only the four widths this port models get a convenience subclass;
/// `Int8Vector` and friends are not installed at all, matching the base
/// constructor's refusal.
pub fn install_vector_subclasses(exports: &Object, env: &Env) -> Result<()> {
    let Some(vector) = exports.get::<Unknown>("Vector")? else {
        return Ok(());
    };

    let installer: Function<'_, FnArgs<(Unknown, String)>, Unknown> = env.run_script(
        "(function (Vector, className) { \
           var Ctor = globalThis[className]; \
           var Sub = function (initialCapacityOrOptions) { \
             return new Vector(Ctor, initialCapacityOrOptions); \
           }; \
           Sub.from = function (iterable, capacity) { \
             return Vector.from(iterable, Ctor, capacity); \
           }; \
           return Sub; \
         })",
    )?;

    for (name, _, subclass_name) in ARRAY_CLASSES {
        let sub = installer.call((vector, (*name).to_owned()).into())?;
        let mut vector_object: Object = exports
            .get("Vector")?
            .ok_or_else(|| Error::new(Status::GenericFailure, "Vector missing from exports"))?;

        vector_object.set_named_property(subclass_name, sub)?;
    }

    let pointer_installer: Function<'_, Unknown, Unknown> = env.run_script(
        "(function (Vector) { \
           var Sub = function (initialCapacityOrOptions) { \
             return Vector.__pointerVector(initialCapacityOrOptions); \
           }; \
           Sub.from = function (iterable, capacity) { \
             return Vector.__pointerVectorFrom(iterable, capacity); \
           }; \
           return Sub; \
         })",
    )?;

    let pointer_sub = pointer_installer.call(vector)?;
    let mut vector_object: Object = exports
        .get("Vector")?
        .ok_or_else(|| Error::new(Status::GenericFailure, "Vector missing from exports"))?;

    vector_object.set_named_property("PointerVector", pointer_sub)?;

    Ok(())
}

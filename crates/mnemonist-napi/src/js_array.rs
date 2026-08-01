//! A real JavaScript array, driving
//! [`mnemonist_core::structures::heap::Store`].
//!
//! # Why the heaps do not keep their items in Rust
//!
//! Every other bridge in this crate stores elements in a `Vec` and materialises
//! a JS array only on the way out. The heaps cannot, for three reasons that are
//! each independently sufficient.
//!
//! 1. **`Heap.heapify(compare, array)` mutates the caller's array in place.**
//!    `test/heap.js` heapifies a literal `[3, 5, 1, 56, 0, 13, 4]` and then
//!    consumes *that array*. A port that copied it in and out would pass this
//!    assertion and be a different function.
//! 2. **`FixedReverseHeap` is parameterised by an `ArrayClass`.**
//!    `new FixedReverseHeap(Uint8Array, 3)` stores into a `Uint8Array`, so
//!    `push(300)` keeps `44`, and `consume()` must return something that
//!    satisfies `instanceof Uint8Array`. Both are asserted upstream. Writing
//!    through the real typed array gets the narrowing, the class and the
//!    `ToUint32` semantics for free and exactly.
//! 3. **The comparator is JavaScript, so a comparison is a JS call anyway.**
//!    Keeping the array on the other side of the boundary costs an element
//!    access per comparison against a call that was already crossing.
//!
//! # What this buys, beyond fidelity
//!
//! Every accessor below is a genuine property access on a genuine JS object.
//! A getter, a `Proxy` trap or a subclassed `push` therefore runs where
//! upstream's would, and the re-entrancy that
//! [`Store`](mnemonist_core::structures::heap::Store) was designed for extends
//! to the array as well as to the comparator. Nothing here holds a Rust borrow
//! across a call into JavaScript.
//!
//! # Lifetimes
//!
//! A `#[napi]` class field cannot borrow, so the array is held as an owning
//! `napi_ref` ([`Handle`]) plus the raw `napi_env` it belongs to — the same
//! two-part design, and the same justification, as [`crate::js_slot`]. A
//! `napi_env` is stable for the life of the environment and every use is
//! inside a call on it.

use std::ptr;
use std::rc::Rc;

use mnemonist_core::structures::heap::{Store, INVALID_ARRAY_LENGTH};
use napi::bindgen_prelude::*;
use napi::sys;

use crate::js_slot::{Handle, JsSlot};

/// A handle to a live JavaScript array (or typed array).
///
/// [`Clone`] shares the handle, which is what
/// [`Store`](mnemonist_core::structures::heap::Store) requires: a clone must be
/// the *same* array, so that `Heap.prototype.clear`'s rebinding detaches an
/// in-flight sift instead of truncating it.
#[derive(Clone)]
pub struct JsArray {
    env: sys::napi_env,
    object: Rc<Handle>,
}

impl JsArray {
    /// Adopt an existing array-like object.
    pub fn capture(env: &Env, value: &Unknown) -> Result<Self> {
        Ok(Self {
            env: env.raw(),
            object: Rc::new(Handle::new(env, value)?),
        })
    }

    /// `[]` — a fresh plain array.
    pub fn empty(env: &Env) -> Result<Self> {
        let mut value = ptr::null_mut();

        // SAFETY: `env` is live; the out-parameter is written on success.
        check(
            unsafe { sys::napi_create_array(env.raw(), &mut value) },
            "napi_create_array",
        )?;

        Self::adopt(env.raw(), value)
    }

    /// `new Ctor(argument)` — used for `new ArrayClass(capacity)`.
    ///
    /// The argument is passed through as the JavaScript value it is, not as a
    /// coerced length, because `new Array(x)` is what upstream writes and its
    /// behaviour on a non-integer, a negative or a string is the class's to
    /// decide — including the `RangeError` that `new Array(-1)` raises before
    /// `FixedReverseHeap`'s own guards ever run.
    pub fn construct(env: &Env, constructor: &Unknown, argument: &Unknown) -> Result<Self> {
        let value = new_instance(env.raw(), constructor.raw(), argument.raw())?;

        Self::adopt(env.raw(), value)
    }

    /// The array, as a value in the caller's scope.
    pub fn value<'env>(&self, env: &'env Env) -> Result<Unknown<'env>> {
        let raw = self.object.value(env.raw())?;

        // SAFETY: `napi_get_reference_value` produced a handle in `env`'s scope.
        Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), raw) })
    }

    /// The array as a storable slot — the same object, not a copy.
    ///
    /// `Heap.prototype.items` is a public property upstream and a caller can
    /// write *through* it, so this hands back the reference rather than a
    /// snapshot.
    pub fn as_slot(&self) -> JsSlot {
        JsSlot::Referenced(Rc::clone(&self.object))
    }

    /// The array, as a raw value. Every accessor below starts here.
    fn raw(&self) -> Result<sys::napi_value> {
        self.object.value(self.env)
    }

    fn adopt(env: sys::napi_env, value: sys::napi_value) -> Result<Self> {
        let owner = Env::from_raw(env);
        // SAFETY: `value` is a live handle in `owner`'s current scope.
        let unknown = unsafe { Unknown::from_raw_unchecked(env, value) };

        Ok(Self {
            env,
            object: Rc::new(Handle::new(&owner, &unknown)?),
        })
    }

    /// Call one of the array's own methods — `push`, `pop`, `slice`.
    ///
    /// Deliberately a real method lookup rather than an equivalent open-coded
    /// element write: upstream calls `heap.push(item)`, and on anything that is
    /// not a plain `Array` that is a different operation (a `Uint8Array` has no
    /// `push` and throws) which the port has no business smoothing over.
    fn call_method(&self, name: &str, args: &[sys::napi_value]) -> Result<sys::napi_value> {
        let object = self.raw()?;
        let method = named_property(self.env, object, name)?;
        let mut value_type = 0;

        // SAFETY: `method` is a live handle.
        check(
            unsafe { sys::napi_typeof(self.env, method, &mut value_type) },
            "napi_typeof",
        )?;

        // `typeof heap.pop !== 'function'`, which is reachable: `Heap.from` on
        // a typed array reaches `consume`, and a typed array has no `pop`.
        // Upstream dies there too, with V8's own
        // `TypeError: heap.pop is not a function`. Ours is an `Error` and names
        // only the method, because the receiver in V8's message comes from the
        // *source text* of the call site. Divergence recorded in
        // `docs/modules/heap.md`; letting an N-API status number reach a user
        // instead would be strictly worse.
        if value_type != sys::ValueType::napi_function {
            return Err(Error::new(
                Status::GenericFailure,
                format!("{name} is not a function"),
            ));
        }

        let mut result = ptr::null_mut();

        // SAFETY: `object` and `method` are live handles; `args` is a slice of
        // live handles of the stated length.
        check(
            unsafe {
                sys::napi_call_function(
                    self.env,
                    object,
                    method,
                    args.len(),
                    args.as_ptr(),
                    &mut result,
                )
            },
            &format!("napi_call_function({name})"),
        )?;

        Ok(result)
    }

    fn to_slot(&self, value: sys::napi_value) -> Result<JsSlot> {
        let env = Env::from_raw(self.env);
        // SAFETY: `value` is a live handle in the current scope.
        let unknown = unsafe { Unknown::from_raw_unchecked(self.env, value) };

        JsSlot::new(&env, &unknown)
    }

    fn raw_slot(&self, slot: &JsSlot) -> Result<sys::napi_value> {
        // SAFETY: rebuilds or resolves the slot into the current scope.
        unsafe { ToNapiValue::to_napi_value(self.env, slot.clone()) }
    }
}

impl Store for JsArray {
    type Item = JsSlot;
    type Error = Error;

    /// `throw new Error(message)` — or a `RangeError`, where the thing doing
    /// the throwing upstream is V8 rather than mnemonist.
    ///
    /// Core raises by message and the bridge picks the constructor, because
    /// `mnemonist-core` has no notion of a JavaScript error class. Two messages
    /// reach here: `Heap.replace`'s, which upstream raises with
    /// `throw new Error(...)`, and `new Array(n)`'s, which is V8's
    /// `RangeError: Invalid array length`.
    ///
    /// The `RangeError` is thrown *into the environment* and reported back as
    /// [`Status::PendingException`], which is what makes napi re-throw the real
    /// object instead of wrapping it in a fresh `Error` — the same mechanism a
    /// throwing comparator rides out on.
    fn raise(&self, message: &'static str) -> Error {
        if message == INVALID_ARRAY_LENGTH {
            let env = Env::from_raw(self.env);

            if env.throw_range_error(message, None).is_ok() {
                return Error::new(Status::PendingException, message);
            }
        }

        Error::new(Status::GenericFailure, message)
    }

    /// `array.length` — a named property, not `napi_get_array_length`, because
    /// the latter refuses a typed array and `FixedReverseHeap`'s is one.
    fn length(&self) -> Result<usize> {
        let object = self.raw()?;
        let length = named_property(self.env, object, "length")?;

        Ok(as_double(self.env, length)? as usize)
    }

    fn get(&self, index: usize) -> Result<JsSlot> {
        let object = self.raw()?;
        let mut value = ptr::null_mut();

        // SAFETY: `object` is live; out-of-range reads answer `undefined`,
        // which is exactly what the algorithms expect to see.
        check(
            unsafe { sys::napi_get_element(self.env, object, index as u32, &mut value) },
            "napi_get_element",
        )?;

        self.to_slot(value)
    }

    fn set(&self, index: usize, value: JsSlot) -> Result<()> {
        let object = self.raw()?;
        let raw = self.raw_slot(&value)?;

        // SAFETY: both handles are live. Writing past the end grows a plain
        // array with holes and is dropped by a typed array, in both cases
        // matching the JavaScript the algorithms were written against.
        check(
            unsafe { sys::napi_set_element(self.env, object, index as u32, raw) },
            "napi_set_element",
        )
    }

    fn push(&self, value: JsSlot) -> Result<usize> {
        let raw = self.raw_slot(&value)?;
        let length = self.call_method("push", &[raw])?;

        Ok(as_double(self.env, length)? as usize)
    }

    fn pop(&self) -> Result<JsSlot> {
        let value = self.call_method("pop", &[])?;

        self.to_slot(value)
    }

    fn set_length(&self, length: usize) -> Result<()> {
        let object = self.raw()?;
        let value = double(self.env, length as f64)?;

        // SAFETY: both handles are live. `length` is non-writable on a typed
        // array, where a sloppy-mode assignment is silently ignored; this is
        // the same non-throwing `[[Set]]`.
        check(
            unsafe {
                sys::napi_set_named_property(self.env, object, c"length".as_ptr().cast(), value)
            },
            "napi_set_named_property(length)",
        )
    }

    /// `new ArrayClass(length)`, where the class is read off the array itself.
    ///
    /// Upstream writes `new Array(l)` inside `Heap.consume` and
    /// `new iterable.constructor(1)` inside `nsmallest`. Both are reproduced by
    /// this one operation, because a `Heap`'s items array *is* a plain `Array`
    /// and its `constructor` therefore *is* `Array`. Reading the class off the
    /// object rather than remembering it is also what `nsmallest` does.
    fn allocate(&self, length: usize) -> Result<Self> {
        let object = self.raw()?;
        let constructor = named_property(self.env, object, "constructor")?;
        let value = new_instance(self.env, constructor, double(self.env, length as f64)?)?;

        Self::adopt(self.env, value)
    }

    /// `new Array(length)`, whatever class this array is.
    ///
    /// The distinction from [`allocate`](Store::allocate) is upstream's and it
    /// is observable: `Heap.prototype.clear` is `this.items = []` and
    /// `Heap.consume` opens with `var array = new Array(l)`, both unconditional
    /// literals, while `nsmallest`'s `n === 1` path writes
    /// `new iterable.constructor(1)`. An earlier cut used `allocate` for all
    /// three, so `Heap.from(new Uint8Array(…)).consume()` came back a
    /// `Uint8Array` where upstream gives a plain `Array`.
    fn plain_array(&self, length: usize) -> Result<Self> {
        let mut value = ptr::null_mut();

        // SAFETY: `env` is live; the out-parameter is written on success.
        check(
            unsafe { sys::napi_create_array_with_length(self.env, length, &mut value) },
            "napi_create_array_with_length",
        )?;

        Self::adopt(self.env, value)
    }

    fn undefined(&self) -> Result<JsSlot> {
        Ok(JsSlot::Undefined)
    }

    fn slice(&self, start: usize, end: usize) -> Result<Self> {
        let start = double(self.env, start as f64)?;
        let end = double(self.env, end as f64)?;
        let value = self.call_method("slice", &[start, end])?;

        Self::adopt(self.env, value)
    }
}

/// `new constructor(argument)`.
fn new_instance(
    env: sys::napi_env,
    constructor: sys::napi_value,
    argument: sys::napi_value,
) -> Result<sys::napi_value> {
    let mut value = ptr::null_mut();

    // SAFETY: `constructor` is a live handle; a non-constructor is reported as
    // a status rather than being invoked, and `new Array(-1)`-style range
    // errors surface as a pending exception which napi re-throws unchanged.
    check(
        unsafe { sys::napi_new_instance(env, constructor, 1, &argument, &mut value) },
        "napi_new_instance",
    )?;

    Ok(value)
}

pub(crate) fn named_property(
    env: sys::napi_env,
    object: sys::napi_value,
    name: &str,
) -> Result<sys::napi_value> {
    let key = std::ffi::CString::new(name)
        .map_err(|_| Error::new(Status::InvalidArg, "property name contains a NUL"))?;
    let mut value = ptr::null_mut();

    // SAFETY: `object` is a live handle and `key` outlives the call.
    check(
        unsafe { sys::napi_get_named_property(env, object, key.as_ptr(), &mut value) },
        "napi_get_named_property",
    )?;

    Ok(value)
}

pub(crate) fn double(env: sys::napi_env, value: f64) -> Result<sys::napi_value> {
    let mut result = ptr::null_mut();

    // SAFETY: `env` is live; the out-parameter is written on success.
    check(
        unsafe { sys::napi_create_double(env, value, &mut result) },
        "napi_create_double",
    )?;

    Ok(result)
}

pub(crate) fn as_double(env: sys::napi_env, value: sys::napi_value) -> Result<f64> {
    let mut result = 0.0;

    // SAFETY: `value` is a live handle. A non-number is refused by status
    // rather than coerced, which is why callers that need coercion go through
    // `napi_coerce_to_number` instead.
    check(
        unsafe { sys::napi_get_value_double(env, value, &mut result) },
        "napi_get_value_double",
    )?;

    Ok(result)
}

pub(crate) fn check(status: sys::napi_status, call: &str) -> Result<()> {
    if status == sys::Status::napi_ok {
        return Ok(());
    }

    // A JavaScript exception raised inside the call is already pending; saying
    // so here is what makes napi re-throw *it* rather than replacing it with a
    // message about N-API. This is the path a throwing comparator takes.
    if status == sys::Status::napi_pending_exception {
        return Err(Error::new(
            Status::PendingException,
            format!("{call} left an exception pending"),
        ));
    }

    Err(Error::new(
        Status::GenericFailure,
        format!("{call} failed with status {status}"),
    ))
}

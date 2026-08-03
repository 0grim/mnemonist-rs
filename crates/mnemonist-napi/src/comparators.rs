//! JS bridge for [`mnemonist_core::utils::comparators`] — capability tier T2.
//!
//! A comparator is a JavaScript function called *from inside* a Rust operation,
//! once per comparison, in the middle of a sift. That re-entrancy is the whole
//! of this tier, and three decisions follow from it.
//!
//! # 1. The comparator is held as a value, never as a borrow
//!
//! [`BridgeComparator`] owns an `napi_ref` and the raw `napi_env`, so it is
//! `'static` and can live inside a `#[napi]` class across calls. Nothing holds
//! a Rust borrow of the heap while it is invoked; see
//! [`mnemonist_core::structures::heap::Store`] for why that is the load-bearing
//! part rather than an optimisation.
//!
//! # 2. A thrown comparator propagates the *original* exception
//!
//! `napi_call_function` returns `napi_pending_exception`, which
//! [`crate::js_array::check`] converts into an `Error` tagged
//! [`Status::PendingException`]. napi's `throw_into` sees a genuinely pending
//! exception and re-throws *that*, so `assert.throws(…, /my message/)` on a
//! user comparator matches the user's error and not a wrapper.
//!
//! Upstream has no `try`/`finally` anywhere in `heap.js`, so a comparator that
//! throws mid-`push` leaves the heap with `items.length` one ahead of `size`.
//! That is reproduced, not repaired — NOTES BUG-HEAP-1.
//!
//! # 3. The comparator's *return value* is a JavaScript number, coerced
//!
//! Upstream never inspects the returned value's type; it writes `< 0`, `> 0`
//! and `>= 0`. So the result is put through `ToNumber` and compared as an
//! `f64`, which makes a comparator returning `'x'`, `{}` or `undefined` answer
//! "equal" exactly as it does upstream, and makes one returning a `Symbol`
//! throw exactly as it does upstream.
//!
//! `BigInt` is the one exception and it is not a rounding error: `ToNumber(1n)`
//! throws a `TypeError`, but `1n < 0` does **not** — the relational operators
//! use `ToNumeric`. So a `BigInt` result is reduced to its sign directly, which
//! is all three predicates need.
//!
//! # `DEFAULT_COMPARATOR` is ported, but `<` is not
//!
//! `DEFAULT_COMPARATOR` is two relational operators wrapped in two `if`s. The
//! `if`s are [`mnemonist_core::utils::comparators::default_comparator`]. The
//! operators are not library logic: `a < b` on two arbitrary JS values runs
//! `ToPrimitive`, which calls user `valueOf`/`toString` and can throw. [`Operand`]
//! answers them natively for the two cases that can be stated exactly —
//! number against number, string against string — and defers anything
//! involving an object, a symbol or a mixed pair to the engine, through a
//! two-line helper. Re-implementing `ToPrimitive` in Rust would be a port of V8,
//! not of mnemonist.

use std::cell::RefCell;
use std::ptr;
use std::rc::Rc;

use mnemonist_core::utils::comparators::{
    self, Comparator, MaybeUndefined, Relational, Sentinel, TupleComparator,
};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::js_array::{as_double, check, named_property};
use crate::js_slot::{Handle, JsSlot};

/// The comparator a heap actually holds.
///
/// Three variants rather than "always a JS function" because `new Heap()` with
/// no argument must not need one: upstream's default is a module-level closure,
/// and reproducing it as a native comparison keeps `new Heap()` from depending
/// on a value stashed at module-load time.
#[derive(Clone)]
pub enum BridgeComparator {
    /// `comparators.DEFAULT_COMPARATOR`, run natively.
    Default(sys::napi_env),
    /// A user-supplied JavaScript function.
    Js(sys::napi_env, Rc<Handle>),
    /// `reverseComparator(inner)` — the argument swap, not a negation.
    Reversed(Box<BridgeComparator>),
}

impl BridgeComparator {
    /// `comparator || DEFAULT_COMPARATOR`, then
    /// `typeof this.comparator !== 'function'` — upstream's two lines, in
    /// order, with the caller's message.
    ///
    /// Note that the falsy test is `||`, so `new Heap(0)` and `new Heap('')`
    /// take the default silently while `new Heap('test')` throws. The original
    /// suite asserts the second.
    pub fn resolve(env: &Env, comparator: Option<Unknown>, message: &str) -> Result<Self> {
        let Some(candidate) = comparator else {
            return Ok(Self::Default(env.raw()));
        };

        if !is_truthy(env, &candidate)? {
            return Ok(Self::Default(env.raw()));
        }

        if candidate.get_type()? != ValueType::Function {
            return Err(Error::new(Status::GenericFailure, message.to_owned()));
        }

        Ok(Self::Js(env.raw(), Rc::new(Handle::new(env, &candidate)?)))
    }

    /// `reverseComparator(this.comparator)`.
    pub fn reversed(self) -> Self {
        Self::Reversed(Box::new(self))
    }
}

impl Comparator<JsSlot, Error> for BridgeComparator {
    fn compare(&self, a: &JsSlot, b: &JsSlot) -> Result<f64> {
        match self {
            Self::Default(env) => default_compare(*env, a, b),
            Self::Js(env, function) => call_comparator(*env, function.value(*env)?, a, b),
            // `function (a, b) { return comparator(b, a); }` — the swap, and
            // nothing else. Negating instead would differ for any comparator
            // that is not antisymmetric, and `MaxHeap` is built on this.
            Self::Reversed(inner) => inner.compare(b, a),
        }
    }
}

/// One JavaScript value, as an operand of `<` and `>`.
///
/// Carries the environment because the fallback path needs to ask the engine.
pub struct Operand {
    env: sys::napi_env,
    slot: JsSlot,
}

impl Operand {
    /// Wrap a stored slot together with the environment `<`/`>` may need to
    /// fall back to. `env` must be live for the whole life of the operand.
    pub fn new(env: sys::napi_env, slot: JsSlot) -> Self {
        Self { env, slot }
    }

    fn relational(&self, other: &Self, greater: bool) -> Result<bool> {
        match (&self.slot, &other.slot) {
            // Number against number. `NaN` makes both operators false in Rust
            // exactly as in JavaScript, so this needs no special case.
            (JsSlot::Number(left), JsSlot::Number(right)) => {
                Ok(if greater { left > right } else { left < right })
            }
            // String against string: JavaScript compares UTF-16 code units,
            // which is what the slot already holds. Rust's `str` ordering would
            // have compared UTF-8 bytes, and the two disagree above the BMP.
            (JsSlot::String(left), JsSlot::String(right)) => Ok(if greater {
                left.as_slice() > right.as_slice()
            } else {
                left.as_slice() < right.as_slice()
            }),
            // Anything else — an object, a symbol, a bigint, or a mixed pair —
            // goes to the engine, because getting it right means running
            // `ToPrimitive` and `ToNumeric`, both of which can call user code.
            _ => engine_relational(self.env, &self.slot, &other.slot, greater),
        }
    }
}

impl Relational<Error> for Operand {
    fn js_lt(&self, other: &Self) -> Result<bool> {
        self.relational(other, false)
    }

    fn js_gt(&self, other: &Self) -> Result<bool> {
        self.relational(other, true)
    }
}

/// `undefined` — what a hole and a past-the-end read both answer.
impl MaybeUndefined for JsSlot {
    fn is_undefined(&self) -> bool {
        matches!(self, JsSlot::Undefined)
    }
}

/// `Infinity` is an ordinary JS number, which is exactly why upstream's
/// `var min = Infinity` sentinel is a bug rather than a convention. See
/// `Unset` in [`mnemonist_core::structures::heap`] and NOTES BUG-HEAP-2/BUG-HEAP-3.
impl Sentinel for JsSlot {
    fn infinity(negative: bool) -> Option<Self> {
        Some(JsSlot::Number(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }))
    }

    fn is_infinity(&self, negative: bool) -> bool {
        match self {
            JsSlot::Number(value) => {
                *value
                    == if negative {
                        f64::NEG_INFINITY
                    } else {
                        f64::INFINITY
                    }
            }
            _ => false,
        }
    }
}

/// `DEFAULT_COMPARATOR(a, b)` over two slots.
pub fn default_compare(env: sys::napi_env, a: &JsSlot, b: &JsSlot) -> Result<f64> {
    comparators::default_comparator(&Operand::new(env, a.clone()), &Operand::new(env, b.clone()))
}

/// `DEFAULT_REVERSE_COMPARATOR(a, b)` over two slots.
pub fn default_reverse_compare(env: sys::napi_env, a: &JsSlot, b: &JsSlot) -> Result<f64> {
    comparators::default_reverse_comparator(
        &Operand::new(env, a.clone()),
        &Operand::new(env, b.clone()),
    )
}

/// Call a JavaScript comparator and reduce its answer to a number.
fn call_comparator(
    env: sys::napi_env,
    function: sys::napi_value,
    a: &JsSlot,
    b: &JsSlot,
) -> Result<f64> {
    let result = call(env, function, a, b)?;

    to_number(env, result)
}

/// `f(a, b)` with `this` undefined.
///
/// `compare(item, parent)` is a bare call upstream, so `this` is `undefined` —
/// which a sloppy-mode callee sees as `globalThis`. That substitution is the
/// callee's, not this bridge's.
fn call(
    env: sys::napi_env,
    function: sys::napi_value,
    a: &JsSlot,
    b: &JsSlot,
) -> Result<sys::napi_value> {
    // SAFETY: rebuilds or resolves each slot into the current scope.
    let arguments = unsafe {
        [
            ToNapiValue::to_napi_value(env, a.clone())?,
            ToNapiValue::to_napi_value(env, b.clone())?,
        ]
    };
    let mut undefined = ptr::null_mut();

    // SAFETY: `env` is live; the out-parameter is written on success.
    check(
        unsafe { sys::napi_get_undefined(env, &mut undefined) },
        "napi_get_undefined",
    )?;

    let mut result = ptr::null_mut();

    // SAFETY: `function` and the argument handles are live. A JavaScript
    // exception raised inside surfaces as `napi_pending_exception`, which
    // `check` tags so that napi re-throws the original error object.
    check(
        unsafe {
            sys::napi_call_function(
                env,
                undefined,
                function,
                arguments.len(),
                arguments.as_ptr(),
                &mut result,
            )
        },
        "comparator",
    )?;

    Ok(result)
}

/// `ToNumber(value)`, except for `BigInt` — see the module docs.
fn to_number(env: sys::napi_env, value: sys::napi_value) -> Result<f64> {
    let mut value_type = 0;

    // SAFETY: `value` is a live handle.
    check(
        unsafe { sys::napi_typeof(env, value, &mut value_type) },
        "napi_typeof",
    )?;

    if value_type == sys::ValueType::napi_bigint {
        return bigint_sign(env, value);
    }

    let mut coerced = ptr::null_mut();

    // SAFETY: `value` is live. A `Symbol` operand raises a `TypeError`, which
    // is what `symbol < 0` does in JavaScript too.
    check(
        unsafe { sys::napi_coerce_to_number(env, value, &mut coerced) },
        "napi_coerce_to_number",
    )?;

    as_double(env, coerced)
}

/// A `BigInt`'s sign, which is all `< 0`, `> 0` and `>= 0` can see.
fn bigint_sign(env: sys::napi_env, value: sys::napi_value) -> Result<f64> {
    let mut sign_bit = 0;
    let mut lossless = false;
    let mut words = 0i64;

    // SAFETY: `value` is a BigInt, checked by the caller. `lossless` is
    // ignored deliberately: a value too large for an `i64` is still non-zero
    // and still has the sign the truncation reports.
    check(
        unsafe { sys::napi_get_value_bigint_int64(env, value, &mut words, &mut lossless) },
        "napi_get_value_bigint_int64",
    )?;

    if words == 0 && !lossless {
        // A multiple of 2^64: non-zero, and the sign has to come from the word
        // representation instead.
        let mut word_count = 0;

        // SAFETY: a query call with both out-pointers null, which is the only
        // shape Node accepts (see `crate::js_slot`).
        check(
            unsafe {
                sys::napi_get_value_bigint_words(
                    env,
                    value,
                    ptr::null_mut(),
                    &mut word_count,
                    ptr::null_mut(),
                )
            },
            "napi_get_value_bigint_words",
        )?;

        let mut buffer = vec![0u64; word_count];

        // SAFETY: `buffer` has room for exactly the reported count.
        check(
            unsafe {
                sys::napi_get_value_bigint_words(
                    env,
                    value,
                    &mut sign_bit,
                    &mut word_count,
                    buffer.as_mut_ptr(),
                )
            },
            "napi_get_value_bigint_words",
        )?;

        if buffer.iter().all(|word| *word == 0) {
            return Ok(0.0);
        }

        return Ok(if sign_bit != 0 { -1.0 } else { 1.0 });
    }

    Ok(words.signum() as f64)
}

// Two helper functions, compiled once, that ask the engine for `<` and `>`.
//
// Cached per thread because the alternative is compiling a script inside a sift
// loop. The references are intentionally never released: they live for the
// environment, which is the process, and a `napi_ref` released during teardown
// would be released after the values it names.
thread_local! {
    static OPERATORS: RefCell<Option<(Rc<Handle>, Rc<Handle>)>> = const { RefCell::new(None) };
}

const LESS_THAN: &str = "(function (a, b) { return a < b; })";
const GREATER_THAN: &str = "(function (a, b) { return a > b; })";

fn engine_relational(env: sys::napi_env, a: &JsSlot, b: &JsSlot, greater: bool) -> Result<bool> {
    let owner = Env::from_raw(env);

    let operators = OPERATORS.with(|cell| -> Result<(Rc<Handle>, Rc<Handle>)> {
        if let Some(pair) = cell.borrow().as_ref() {
            return Ok(pair.clone());
        }

        let less: Unknown = owner.run_script(LESS_THAN)?;
        let more: Unknown = owner.run_script(GREATER_THAN)?;
        let pair = (
            Rc::new(Handle::new(&owner, &less)?),
            Rc::new(Handle::new(&owner, &more)?),
        );

        *cell.borrow_mut() = Some(pair.clone());

        Ok(pair)
    })?;

    let function = if greater { operators.1 } else { operators.0 };
    let result = call(env, function.value(env)?, a, b)?;
    let mut boolean = false;

    // SAFETY: `result` is the boolean `<` or `>` produced.
    check(
        unsafe { sys::napi_get_value_bool(env, result, &mut boolean) },
        "napi_get_value_bool",
    )?;

    Ok(boolean)
}

/// `if (!iterable)` — JS truthiness, reused here for `comparator || DEFAULT`.
fn is_truthy(env: &Env, value: &Unknown) -> Result<bool> {
    let mut coerced = ptr::null_mut();

    // SAFETY: `value` is a live handle; `ToBoolean` never throws.
    check(
        unsafe { sys::napi_coerce_to_bool(env.raw(), value.raw(), &mut coerced) },
        "napi_coerce_to_bool",
    )?;

    let mut boolean = false;

    // SAFETY: `coerced` is a boolean.
    check(
        unsafe { sys::napi_get_value_bool(env.raw(), coerced, &mut boolean) },
        "napi_get_value_bool",
    )?;

    Ok(boolean)
}

// ---------------------------------------------------------------------------
// The four exports of `utils/comparators.js`.
//
// Two are functions and go out as `#[napi]` functions. The other two *return*
// functions, and napi has no way to hand back a freshly created closure from a
// `#[napi]` signature -- `Function<'env>` borrows the `Env` the call owns. So
// the closure-making half of each is [`INSTALLER`], evaluated once at module
// load, and it is upstream's own source: `reverseComparator`'s body is three
// lines with no logic to port, and `createTupleComparator`'s comparison is
// `__tupleCompare` below, which is `mnemonist_core`'s.
// ---------------------------------------------------------------------------

/// `comparators.DEFAULT_COMPARATOR`.
#[napi(js_name = "DEFAULT_COMPARATOR")]
pub fn js_default_comparator(env: Env, a: Unknown, b: Unknown) -> Result<f64> {
    default_compare(env.raw(), &JsSlot::new(&env, &a)?, &JsSlot::new(&env, &b)?)
}

/// `comparators.DEFAULT_REVERSE_COMPARATOR`.
///
/// Upstream ships this as its own function rather than as
/// `reverseComparator(DEFAULT_COMPARATOR)`; the two agree on every input but
/// are not the same function object.
#[napi(js_name = "DEFAULT_REVERSE_COMPARATOR")]
pub fn js_default_reverse_comparator(env: Env, a: Unknown, b: Unknown) -> Result<f64> {
    default_reverse_compare(env.raw(), &JsSlot::new(&env, &a)?, &JsSlot::new(&env, &b)?)
}

/// The body of the comparator `createTupleComparator(size)` returns.
///
/// The closure is JavaScript because napi cannot return one; the *comparison*
/// is [`TupleComparator`], which is the port.
#[napi(js_name = "__tupleCompare")]
pub fn js_tuple_compare(env: Env, size: u32, a: Unknown, b: Unknown) -> Result<f64> {
    let comparator = TupleComparator {
        size: size as usize,
    };
    let left = read_tuple(&env, a, comparator.size)?;
    let right = read_tuple(&env, b, comparator.size)?;

    comparator.compare(&left, &right)
}

/// The first `size` members of a JS array-like, as operands.
///
/// Reads past the end answer `undefined`, which compares false both ways — the
/// same non-ordering `a[i]` past the end produces upstream.
fn read_tuple(env: &Env, value: Unknown, size: usize) -> Result<Vec<Operand>> {
    let mut members = Vec::with_capacity(size);

    for index in 0..size {
        let mut member = ptr::null_mut();

        // SAFETY: `value` is a live handle; a non-object raises a status.
        check(
            unsafe { sys::napi_get_element(env.raw(), value.raw(), index as u32, &mut member) },
            "napi_get_element",
        )?;

        // SAFETY: `member` is a live handle in the current scope.
        let member = unsafe { Unknown::from_raw_unchecked(env.raw(), member) };

        members.push(Operand::new(env.raw(), JsSlot::new(env, &member)?));
    }

    Ok(members)
}

/// `reverseComparator` verbatim, plus the closure half of
/// `createTupleComparator`.
///
/// Evaluated once, at module load, for the reason DIV-STACK-7 gives about `X.of`: it
/// is arity/closure glue with no logic, and writing it as JavaScript keeps the
/// addon self-contained instead of pushing a semantic into the test shim.
const INSTALLER: &str = "(function (exports) { \
     exports.reverseComparator = function (comparator) { \
       return function (a, b) { return comparator(b, a); }; \
     }; \
     var tupleCompare = exports.__tupleCompare; \
     delete exports.__tupleCompare; \
     exports.createTupleComparator = function (size) { \
       return function (a, b) { return tupleCompare(size, a, b); }; \
     }; \
   })";

/// Install the two exports that return functions.
///
/// Called from the addon's single `#[napi(module_exports)]` hook.
pub fn install_comparator_factories(exports: &Object, env: &Env) -> Result<()> {
    let installer: Function<'_, &Object, Unknown> = env.run_script(INSTALLER)?;

    installer.call(exports)?;

    Ok(())
}

/// `Number(value)` for a length-like constructor argument, as `new Array(n)`
/// would see it. Shared by both heap bridges.
pub(crate) fn coerce_length(env: &Env, value: &Unknown) -> Result<f64> {
    let mut coerced = ptr::null_mut();

    // SAFETY: `value` is a live handle.
    check(
        unsafe { sys::napi_coerce_to_number(env.raw(), value.raw(), &mut coerced) },
        "napi_coerce_to_number",
    )?;

    as_double(env.raw(), coerced)
}

/// `typeof value === 'number'`.
pub(crate) fn is_number(value: &Unknown) -> Result<bool> {
    Ok(value.get_type()? == ValueType::Number)
}

/// `typeof target.<name> === 'number' ? target.<name> : undefined`.
pub(crate) fn numeric_property(env: &Env, target: &Unknown, name: &str) -> Result<Option<f64>> {
    let raw = named_property(env.raw(), target.raw(), name)?;
    let mut value_type = 0;

    // SAFETY: `raw` is a live handle.
    check(
        unsafe { sys::napi_typeof(env.raw(), raw, &mut value_type) },
        "napi_typeof",
    )?;

    if value_type != sys::ValueType::napi_number {
        return Ok(None);
    }

    Ok(Some(as_double(env.raw(), raw)?))
}

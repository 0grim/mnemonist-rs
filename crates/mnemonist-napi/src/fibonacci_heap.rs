//! JS bridge for [`mnemonist_core::structures::fibonacci_heap`].
//!
//! Thin translation only — every behavioural decision lives in core. Four
//! things the bridge carries.
//!
//! 1. **The core structure is held in a [`RefCell`], `borrow()`-only.**
//!    Same reasoning as [`crate::heap`]'s bridge: the comparator is called
//!    *from inside* `push`/`pop` (`consolidate`), so a `borrow_mut()`
//!    anywhere here would deadlock against a re-entrant call reaching back
//!    into this same instance. Every core method takes `&self` for exactly
//!    that reason.
//! 2. **`MaxFibonacciHeap` is installed as JavaScript, not a second `#[napi]`
//!    class.** Upstream's is `MaxFibonacciHeap.prototype =
//!    FibonacciHeap.prototype` — the same B-75-shaped anti-pattern
//!    `crate::heap`'s bridge already documents for `Heap`/`MaxHeap`, and a
//!    second native class would silently *fix* it instead of reproducing it
//!    (NOTES.md B-221). See [`install_fibonacci_heap_statics`].
//! 3. **`.from` always drains through the 5-branch `forEach` dispatch.**
//!    Unlike `Heap.from`, which special-cases an array-like source,
//!    `FibonacciHeap.from` upstream is unconditionally
//!    `forEach(iterable, function (value) { heap.push(value); })` — no
//!    array-like fast path exists to reproduce. `push` is exactly `N`
//!    ordinary pushes either way, so this is a direct translation rather
//!    than a divergence.
//! 4. **`inspect` is not ported.** A Node display convenience with no
//!    upstream assertion, same call as every other T2-tier module here.

use std::cell::RefCell;

use mnemonist_core::structures::fibonacci_heap::FibonacciHeap as CoreHeap;
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::comparators::BridgeComparator;
use crate::foreach;
use crate::js_slot::JsSlot;

/// Upstream's constructor message, verbatim — and verbatim for
/// `MaxFibonacciHeap` too: upstream's own `MaxFibonacciHeap` constructor
/// throws the identical `FibonacciHeap.constructor` wording rather than
/// naming itself, so one constant serves both factories correctly rather
/// than by coincidence.
const FIB_HEAP_COMPARATOR: &str =
    "mnemonist/FibonacciHeap.constructor: given comparator should be a function.";

type Inner = CoreHeap<JsSlot, BridgeComparator, Error>;

/// A Fibonacci heap over arbitrary JavaScript values.
#[napi(js_name = "FibonacciHeap")]
pub struct JsFibonacciHeap {
    inner: RefCell<Inner>,
}

#[napi]
impl JsFibonacciHeap {
    /// `new FibonacciHeap(comparator)`.
    #[napi(constructor)]
    pub fn new(env: Env, comparator: Option<Unknown>) -> Result<Self> {
        let comparator = BridgeComparator::resolve(&env, comparator, FIB_HEAP_COMPARATOR)?;

        Ok(Self {
            inner: RefCell::new(CoreHeap::new(comparator)),
        })
    }

    /// `FibonacciHeap.__max(comparator)` — the factory the installed
    /// `MaxFibonacciHeap` calls. Deleted from view once `MaxFibonacciHeap`
    /// has closed over it; see [`install_fibonacci_heap_statics`].
    ///
    /// `comparator.reversed()`, not [`CoreHeap::new_max`]: `BridgeComparator`
    /// folds `reverseComparator` into its own `Reversed` variant (see
    /// `crate::comparators`), so reversing one stays the same Rust type
    /// (`BridgeComparator`) rather than becoming
    /// `core::utils::comparators::Reversed<BridgeComparator>` — a different
    /// type `Inner`'s alias does not name. `crate::heap`'s `JsHeap::max`
    /// takes the identical route for the identical reason.
    #[napi(factory, js_name = "__max")]
    pub fn max(env: Env, comparator: Option<Unknown>) -> Result<Self> {
        let comparator = BridgeComparator::resolve(&env, comparator, FIB_HEAP_COMPARATOR)?;

        Ok(Self {
            inner: RefCell::new(CoreHeap::new(comparator.reversed())),
        })
    }

    /// `this.size`. `i64`, matching core — see NOTES.md B-220 and
    /// `mnemonist_core::structures::fibonacci_heap`'s own docs: a
    /// re-entrant `clear()` from inside `consolidate` can drive this
    /// negative, and upstream's own arithmetic reflects that rather than
    /// clamping it.
    #[napi(getter)]
    pub fn size(&self) -> i64 {
        self.inner.borrow().size()
    }

    /// `#.clear`.
    #[napi]
    pub fn clear(&self) {
        self.inner.borrow().clear();
    }

    /// `#.push` — returns `++this.size`.
    #[napi]
    pub fn push(&self, env: Env, item: Unknown) -> Result<i64> {
        let item = JsSlot::new(&env, &item)?;

        self.inner.borrow().push(item)
    }

    /// `#.peek` — `undefined` when empty.
    ///
    /// `Either<JsSlot, Undefined>`, not `Option<JsSlot>`: napi's own
    /// `ToNapiValue for Option<T>` renders `None` as `null`, and
    /// `test/fibonacci-heap.js`'s very first `peek` assertion is
    /// `assert.strictEqual(heap.peek(), undefined)` — `null` fails it
    /// outright. Every other peek/pop-shaped method in this crate
    /// (`crate::queue`, `crate::stack`, …) takes the identical route.
    #[napi]
    pub fn peek(&self) -> Either<JsSlot, Undefined> {
        self.inner.borrow().peek().into()
    }

    /// `#.pop` — `undefined` when empty. See [`peek`](Self::peek) for why
    /// this is `Either`, not `Option`.
    #[napi]
    pub fn pop(&self) -> Result<Either<JsSlot, Undefined>> {
        self.inner.borrow().pop().map(Into::into)
    }

    /// `FibonacciHeap.from(iterable, comparator)`.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown, comparator: Option<Unknown>) -> Result<Self> {
        let comparator = BridgeComparator::resolve(&env, comparator, FIB_HEAP_COMPARATOR)?;
        let values = foreach::collect(&env, iterable)?;

        Ok(Self {
            inner: RefCell::new(CoreHeap::from_iter(values, comparator)?),
        })
    }

    /// `MaxFibonacciHeap.from(iterable, comparator)`, reached through the
    /// installed `MaxFibonacciHeap`.
    #[napi(factory, js_name = "__maxFrom")]
    pub fn max_from(env: Env, iterable: Unknown, comparator: Option<Unknown>) -> Result<Self> {
        let comparator = BridgeComparator::resolve(&env, comparator, FIB_HEAP_COMPARATOR)?;
        let values = foreach::collect(&env, iterable)?;

        Ok(Self {
            inner: RefCell::new(CoreHeap::from_iter(values, comparator.reversed())?),
        })
    }
}

/// Upstream's load-time tail, evaluated once:
///
/// ```js
/// function MaxFibonacciHeap(comparator) { ... }
/// MaxFibonacciHeap.prototype = FibonacciHeap.prototype;
/// FibonacciHeap.MinFibonacciHeap = FibonacciHeap;
/// FibonacciHeap.MaxFibonacciHeap = MaxFibonacciHeap;
/// ```
///
/// The prototype assignment is the whole point, exactly as it is for
/// `Heap`/`MaxHeap` (`crate::heap`'s `INSTALLER`): it is what makes
/// `new FibonacciHeap() instanceof MaxFibonacciHeap` true upstream (NOTES.md
/// B-221), and a second native class would have its own prototype and
/// silently repair that. `.bind(FibonacciHeap)` is not decoration either: a
/// `#[napi(factory)]` instantiates with `napi_new_instance(this)`, so a
/// factory pulled off the constructor and called bare would die with
/// `Failed to create instance of class` — the exact failure heap.rs's own
/// docs record hitting first.
const INSTALLER: &str = "(function (FibonacciHeap) { \
     var makeMax = FibonacciHeap.__max.bind(FibonacciHeap), \
         makeMaxFrom = FibonacciHeap.__maxFrom.bind(FibonacciHeap); \
     function MaxFibonacciHeap(comparator) { return makeMax(comparator); } \
     MaxFibonacciHeap.prototype = FibonacciHeap.prototype; \
     MaxFibonacciHeap.from = function (iterable, comparator) { \
       return makeMaxFrom(iterable, comparator); \
     }; \
     FibonacciHeap.MinFibonacciHeap = FibonacciHeap; \
     FibonacciHeap.MaxFibonacciHeap = MaxFibonacciHeap; \
   })";

/// Install `MaxFibonacciHeap`, `FibonacciHeap.MinFibonacciHeap` and
/// `FibonacciHeap.MaxFibonacciHeap`.
///
/// Called from the addon's single `#[napi(module_exports)]` hook
/// (`crate::statics::install_variadic_factories`).
///
/// # What this cannot remove
///
/// `FibonacciHeap.__max` and `FibonacciHeap.__maxFrom` stay on the
/// constructor: they are `#[napi(factory)]`s, and napi defines a class's own
/// properties `writable: false, enumerable: false, configurable: false`, so
/// `delete` is a no-op on them — measured, same as `crate::heap`'s D-75
/// residual. Non-enumerable, so `Object.keys`/`for...in`/`deepStrictEqual`
/// cannot see them; the bridge's only addition to upstream's surface.
pub fn install_fibonacci_heap_statics(exports: &mut Object, env: &Env) -> Result<()> {
    let constructor: Unknown = exports
        .get("FibonacciHeap")?
        .ok_or_else(|| missing("FibonacciHeap"))?;
    let installer: Function<'_, Unknown, Unknown> = env.run_script(INSTALLER)?;

    installer.call(constructor)?;

    Ok(())
}

fn missing(what: &str) -> Error {
    Error::new(
        Status::GenericFailure,
        format!(
            "cannot install the fibonacci-heap statics: `exports.{what}` does not exist. The \
             installer and the addon's exports have drifted apart."
        ),
    )
}

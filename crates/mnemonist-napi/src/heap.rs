//! JS bridge for [`mnemonist_core::structures::heap`].
//!
//! Thin translation only: every behavioural decision lives in the core crate.
//! What the bridge has to *carry* is the six things that only exist once
//! JavaScript is in the picture.
//!
//! 1. **`items` is a real JavaScript array**, not a `Vec` materialised on the
//!    way out. `Heap.heapify(compare, array)` mutates the caller's array in
//!    place and `test/heap.js` then consumes that same array, so there is no
//!    copy-in/copy-out design that is not a different function. See
//!    [`crate::js_array`].
//! 2. **The core structure is held in a [`RefCell`], and only ever
//!    `borrow()`ed.** DIV-STACK-5's reason applies unchanged — a `&self` on a `Freeze`
//!    type is `noalias readonly` and JavaScript mutates this object from inside
//!    a callback. What is *new* here is that the callback is a **comparator**,
//!    running in the middle of a sift, so `borrow_mut()` would deadlock against
//!    itself. Every core method takes `&self` for exactly that reason, and the
//!    shared borrows below nest safely when a comparator re-enters.
//! 3. **`MaxHeap` is installed as JavaScript.** Upstream's is
//!    `MaxHeap.prototype = Heap.prototype`, which makes every `Heap` an
//!    `instanceof MaxHeap` and vice versa (NOTES BUG-HEAP-4). A second `#[napi]` class
//!    would have its own prototype and would quietly *fix* that, so `MaxHeap` is
//!    upstream's four lines, evaluated once at load — the same call
//!    [`crate::statics`] makes for `X.of`.
//! 4. **`Heap.nsmallest`'s two-argument form** is upstream's
//!    `arguments.length === 2`. napi cannot see arity, so the discriminator is
//!    "was a third argument supplied", which differs only for an explicit
//!    `nsmallest(cmp, n, undefined)`.
//! 5. **`inspect` is not ported.** A Node display convenience with no upstream
//!    assertion.
//! 6. **`comparator` is not exposed.** Upstream's is a public property holding a
//!    function; ours is a [`BridgeComparator`], and the two representations do
//!    not have a common JavaScript value. No upstream assertion reads it. See
//!    `docs/modules/heap.md`.

use std::cell::RefCell;

use mnemonist_core::structures::heap::{
    self as core_heap, Heap as CoreHeap, Source, Store, REPLACE_EMPTY,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::comparators::{coerce_length, numeric_property, BridgeComparator};
use crate::foreach;
use crate::js_array::JsArray;
use crate::js_slot::JsSlot;

/// Upstream's constructor messages, verbatim.
const HEAP_COMPARATOR: &str = "mnemonist/Heap.constructor: given comparator should be a function.";
const MAX_HEAP_COMPARATOR: &str =
    "mnemonist/MaxHeap.constructor: given comparator should be a function.";

type Inner = CoreHeap<JsArray, BridgeComparator>;

/// A binary minimum heap over arbitrary JavaScript values.
#[napi(js_name = "Heap")]
pub struct JsHeap {
    inner: RefCell<Inner>,
}

#[napi]
impl JsHeap {
    /// `new Heap(comparator)`.
    #[napi(constructor)]
    pub fn new(env: Env, comparator: Option<Unknown>) -> Result<Self> {
        let comparator = BridgeComparator::resolve(&env, comparator, HEAP_COMPARATOR)?;

        Ok(Self {
            inner: RefCell::new(CoreHeap::new(JsArray::empty(&env)?, comparator)),
        })
    }

    /// `Heap.__max(comparator)` — the factory the installed `MaxHeap` calls.
    ///
    /// Deleted from the exported constructor once `MaxHeap` has closed over it;
    /// see [`install_heap_statics`].
    #[napi(factory, js_name = "__max")]
    pub fn max(env: Env, comparator: Option<Unknown>) -> Result<Self> {
        let comparator = BridgeComparator::resolve(&env, comparator, MAX_HEAP_COMPARATOR)?;

        Ok(Self {
            inner: RefCell::new(CoreHeap::new(JsArray::empty(&env)?, comparator.reversed())),
        })
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    /// `this.items` — the live array, so a caller can write through it exactly
    /// as they can upstream.
    #[napi(getter)]
    pub fn items(&self) -> JsSlot {
        self.inner.borrow().items().as_slot()
    }

    /// `#.clear` — a **new** array, not a truncation.
    #[napi]
    pub fn clear(&self) -> Result<()> {
        self.inner.borrow().clear()
    }

    /// `#.push` — returns `++this.size`.
    #[napi]
    pub fn push(&self, env: Env, item: Unknown) -> Result<u32> {
        let item = JsSlot::new(&env, &item)?;

        Ok(self.inner.borrow().push(item)? as u32)
    }

    /// `#.peek` — `this.items[0]`, `undefined` when empty.
    #[napi]
    pub fn peek(&self) -> Result<JsSlot> {
        self.inner.borrow().peek()
    }

    /// `#.pop` — `undefined` when empty.
    #[napi]
    pub fn pop(&self) -> Result<JsSlot> {
        self.inner.borrow().pop()
    }

    /// `#.replace` — throws on an empty heap.
    #[napi]
    pub fn replace(&self, env: Env, item: Unknown) -> Result<JsSlot> {
        let item = JsSlot::new(&env, &item)?;

        self.inner.borrow().replace(item)
    }

    /// `#.pushpop`.
    #[napi]
    pub fn pushpop(&self, env: Env, item: Unknown) -> Result<JsSlot> {
        let item = JsSlot::new(&env, &item)?;

        self.inner.borrow().pushpop(item)
    }

    /// `#.consume` — drains the heap into a sorted array.
    #[napi]
    pub fn consume(&self) -> Result<JsSlot> {
        Ok(self.inner.borrow().consume()?.as_slot())
    }

    /// `#.toArray` — the same, over a clone, so the heap survives.
    #[napi(js_name = "toArray")]
    pub fn to_array(&self) -> Result<JsSlot> {
        Ok(self.inner.borrow().to_array()?.as_slot())
    }

    /// `Heap.from(iterable, comparator)`.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown, comparator: Option<Unknown>) -> Result<Self> {
        let comparator = BridgeComparator::resolve(&env, comparator, HEAP_COMPARATOR)?;

        Ok(Self {
            inner: RefCell::new(CoreHeap::from_items(
                materialise(&env, iterable)?,
                comparator,
            )?),
        })
    }

    /// `MaxHeap.from(iterable, comparator)`, reached through the installed
    /// `MaxHeap`.
    #[napi(factory, js_name = "__maxFrom")]
    pub fn max_from(env: Env, iterable: Unknown, comparator: Option<Unknown>) -> Result<Self> {
        let comparator = BridgeComparator::resolve(&env, comparator, MAX_HEAP_COMPARATOR)?;

        Ok(Self {
            inner: RefCell::new(CoreHeap::from_items(
                materialise(&env, iterable)?,
                comparator.reversed(),
            )?),
        })
    }
}

/// The ten raw-array helpers upstream exports on the `Heap` constructor.
///
/// # Why they are not `#[napi]` statics on `JsHeap`
///
/// Measured, after nine of `test/heap.js`'s fourteen cases failed with
/// `heap.push is not a function`: **napi-rs registers a class's statics and its
/// prototype methods through one name table.** Upstream has *both* `Heap.push`
/// and `Heap.prototype.push`, with different signatures, and five such pairs in
/// all -- `push`, `pop`, `replace`, `pushpop`, `consume`. Declaring both halves
/// makes the prototype half silently vanish.
///
/// JavaScript has no such conflict, because a constructor and its prototype are
/// different objects. So the statics live on a class of their own, which
/// [`install_heap_statics`] copies onto `Heap` and then deletes from the
/// addon's exports -- leaving exactly upstream's surface.
#[napi(js_name = "HeapStatics")]
pub struct JsHeapStatics;

#[napi]
impl JsHeapStatics {
    #[napi(js_name = "siftDown")]
    pub fn static_sift_down(
        env: Env,
        comparator: Unknown,
        heap: Unknown,
        start_index: u32,
        index: u32,
    ) -> Result<()> {
        let (comparator, heap) = raw_pair(&env, comparator, heap)?;

        core_heap::sift_down(&comparator, &heap, start_index as usize, index as usize)
    }

    #[napi(js_name = "siftUp")]
    pub fn static_sift_up(env: Env, comparator: Unknown, heap: Unknown, index: u32) -> Result<()> {
        let (comparator, heap) = raw_pair(&env, comparator, heap)?;

        core_heap::sift_up(&comparator, &heap, index as usize)
    }

    #[napi(js_name = "push")]
    pub fn static_push(env: Env, comparator: Unknown, heap: Unknown, item: Unknown) -> Result<()> {
        let (comparator, heap) = raw_pair(&env, comparator, heap)?;

        core_heap::push(&comparator, &heap, JsSlot::new(&env, &item)?)
    }

    #[napi(js_name = "pop")]
    pub fn static_pop(env: Env, comparator: Unknown, heap: Unknown) -> Result<JsSlot> {
        let (comparator, heap) = raw_pair(&env, comparator, heap)?;

        core_heap::pop(&comparator, &heap)
    }

    #[napi(js_name = "replace")]
    pub fn static_replace(
        env: Env,
        comparator: Unknown,
        heap: Unknown,
        item: Unknown,
    ) -> Result<JsSlot> {
        let (comparator, heap) = raw_pair(&env, comparator, heap)?;

        core_heap::replace(&comparator, &heap, JsSlot::new(&env, &item)?)
    }

    #[napi(js_name = "pushpop")]
    pub fn static_pushpop(
        env: Env,
        comparator: Unknown,
        heap: Unknown,
        item: Unknown,
    ) -> Result<JsSlot> {
        let (comparator, heap) = raw_pair(&env, comparator, heap)?;

        core_heap::pushpop(&comparator, &heap, JsSlot::new(&env, &item)?)
    }

    /// `Heap.heapify(compare, array)` — in place, on the caller's array.
    #[napi(js_name = "heapify")]
    pub fn static_heapify(env: Env, comparator: Unknown, array: Unknown) -> Result<()> {
        let (comparator, array) = raw_pair(&env, comparator, array)?;

        core_heap::heapify(&comparator, &array)
    }

    /// `Heap.consume(compare, heap)` — drains the caller's array.
    #[napi(js_name = "consume")]
    pub fn static_consume(env: Env, comparator: Unknown, heap: Unknown) -> Result<JsSlot> {
        let (comparator, heap) = raw_pair(&env, comparator, heap)?;

        Ok(core_heap::consume(&comparator, &heap)?.as_slot())
    }

    /// `Heap.nsmallest([compare], n, iterable)`.
    #[napi(js_name = "nsmallest")]
    pub fn static_nsmallest(
        env: Env,
        first: Unknown,
        second: Unknown,
        third: Option<Unknown>,
    ) -> Result<JsSlot> {
        let (comparator, n, iterable) = top_arguments(&env, first, second, third)?;
        let source = source_of(&env, iterable)?;

        Ok(core_heap::nsmallest(&comparator, n, source)?.as_slot())
    }

    /// `Heap.nlargest([compare], n, iterable)`.
    #[napi(js_name = "nlargest")]
    pub fn static_nlargest(
        env: Env,
        first: Unknown,
        second: Unknown,
        third: Option<Unknown>,
    ) -> Result<JsSlot> {
        let (comparator, n, iterable) = top_arguments(&env, first, second, third)?;
        let source = source_of(&env, iterable)?;

        Ok(core_heap::nlargest(&comparator, n, source)?.as_slot())
    }
}

/// A comparator and an array, as every raw-array static takes them.
fn raw_pair<'env>(
    env: &Env,
    comparator: Unknown<'env>,
    array: Unknown<'env>,
) -> Result<(BridgeComparator, JsArray)> {
    Ok((
        BridgeComparator::resolve(env, Some(comparator), HEAP_COMPARATOR)?,
        JsArray::capture(env, &array)?,
    ))
}

/// `if (arguments.length === 2) { iterable = n; n = compare; compare = DEFAULT; }`
///
/// `n` leaves here as the JavaScript number it arrived as, **unvalidated**.
/// Upstream never validates it either: it is compared (`n === 1`,
/// `n >= iterable.length`), used as a `slice` end, and used as a *loop counter*
/// — and the only construct that can refuse it is the `new Array(n)` on the
/// iterable path, which core raises from where upstream has it. An earlier cut
/// checked it here, which made `Heap.nsmallest(cmp, -1, array)` throw where
/// upstream answers.
fn top_arguments<'env>(
    env: &Env,
    first: Unknown<'env>,
    second: Unknown<'env>,
    third: Option<Unknown<'env>>,
) -> Result<(BridgeComparator, f64, Unknown<'env>)> {
    match third {
        Some(iterable) => Ok((
            BridgeComparator::resolve(env, Some(first), HEAP_COMPARATOR)?,
            coerce_length(env, &second)?,
            iterable,
        )),
        None => Ok((
            BridgeComparator::resolve(env, None, HEAP_COMPARATOR)?,
            coerce_length(env, &first)?,
            second,
        )),
    }
}

/// `iterables.isArrayLike(iterable) ? … : forEach(iterable, …)`.
///
/// The branch is a JavaScript question — `Array.isArray(t) || isTypedArray(t)`
/// — so it is answered here and handed to core as a
/// [`Source`](mnemonist_core::structures::heap::Source).
fn source_of(env: &Env, iterable: Unknown) -> Result<Source<JsArray>> {
    if foreach::is_array_like(env, &iterable)? {
        return Ok(Source::ArrayLike(JsArray::capture(env, &iterable)?));
    }

    Ok(Source::Iterable {
        values: foreach::collect(env, iterable)?,
        guessed_length: guess_length(env, &iterable)?,
        plain: JsArray::empty(env)?,
    })
}

/// `iterables.guessLength(target)` — `target.length`, else `target.size`, else
/// absent.
fn guess_length(env: &Env, target: &Unknown) -> Result<Option<usize>> {
    for name in ["length", "size"] {
        let Some(length) = numeric_property(env, target, name)? else {
            continue;
        };

        // A negative or fractional guess would only reach `new Array(n)`, which
        // refuses it; upstream would throw there too. Treating it as absent
        // here leaves `n` uncorrected, which is the same non-event as
        // `undefined < n`.
        if length.is_finite() && length >= 0.0 && length.fract() == 0.0 {
            return Ok(Some(length as usize));
        }

        return Ok(None);
    }

    Ok(None)
}

/// `iterables.isArrayLike(iterable) ? iterable.slice() : iterables.toArray(iterable)`
fn materialise(env: &Env, iterable: Unknown) -> Result<JsArray> {
    if foreach::is_array_like(env, &iterable)? {
        let source = JsArray::capture(env, &iterable)?;
        let length = source.length()?;

        return source.slice(0, length);
    }

    let values = foreach::collect(env, iterable)?;
    let array = JsArray::empty(env)?;

    for value in values {
        array.push(value)?;
    }

    Ok(array)
}

/// Upstream's load-time tail: the eight raw-array statics under their real
/// names, then `MaxHeap`.
///
/// The `.bind(Heap)` is not decoration. A `#[napi(factory)]` builds its instance
/// with `napi_new_instance(this)`, so a factory pulled off the constructor and
/// called bare dies with `Failed to create instance of class`. Binding keeps
/// the receiver while still letting the temporary property be deleted, so the
/// addon's public surface ends up exactly upstream's.
///
/// ```js
/// function MaxHeap(comparator) { … }
/// MaxHeap.prototype = Heap.prototype;
/// Heap.MinHeap = Heap;
/// Heap.MaxHeap = MaxHeap;
/// ```
///
/// The prototype assignment is the whole point: it is what makes
/// `new Heap() instanceof MaxHeap` true upstream, and a second native class
/// would have silently corrected it. The two factories are captured into the
/// closure and then deleted from the constructor, so the addon's public surface
/// is exactly upstream's.
const INSTALLER: &str = "(function (Heap, statics) { \
     ['siftUp', 'siftDown', 'push', 'pop', 'replace', 'pushpop', 'heapify', \
      'consume', 'nsmallest', 'nlargest'].forEach(function (name) { \
       Heap[name] = statics[name].bind(statics); \
     }); \
     var makeMax = Heap.__max.bind(Heap), \
         makeMaxFrom = Heap.__maxFrom.bind(Heap); \
     function MaxHeap(comparator) { return makeMax(comparator); } \
     MaxHeap.prototype = Heap.prototype; \
     MaxHeap.from = function (iterable, comparator) { return makeMaxFrom(iterable, comparator); }; \
     Heap.MinHeap = Heap; \
     Heap.MaxHeap = MaxHeap; \
   })";

/// Install the raw-array statics, `MaxHeap`, `Heap.MinHeap` and `Heap.MaxHeap`.
///
/// Called from the addon's single `#[napi(module_exports)]` hook. `REPLACE_EMPTY`
/// is referenced here so that the constant the original suite matches against
/// (`assert.throws(…, /replace/)`) is visibly the one core raises.
///
/// # The two properties this cannot remove
///
/// `Heap.__max` and `Heap.__maxFrom` stay on the constructor. They are
/// `#[napi(factory)]`s, and napi defines a class's own properties with
/// `writable: false, enumerable: false, configurable: false`, so `delete` is a
/// no-op on them -- measured. They are non-enumerable, so no `Object.keys`,
/// `for...in` or `deepStrictEqual` can see them, and they are the bridge's only
/// addition to upstream's surface. Recorded in `docs/modules/heap.md` rather
/// than papered over.
pub fn install_heap_statics(exports: &mut Object, env: &Env) -> Result<()> {
    debug_assert!(REPLACE_EMPTY.contains("replace"));

    let constructor: Unknown = exports.get("Heap")?.ok_or_else(|| missing("Heap"))?;
    let statics: Unknown = exports
        .get("HeapStatics")?
        .ok_or_else(|| missing("HeapStatics"))?;
    let installer: Function<'_, FnArgs<(Unknown, Unknown)>, Unknown> = env.run_script(INSTALLER)?;

    installer.call((constructor, statics).into())?;

    // `HeapStatics` is scaffolding, not a module. `exports` is an ordinary
    // object, so unlike a napi class's own properties this delete works.
    exports.delete_named_property("HeapStatics")?;

    Ok(())
}

fn missing(what: &str) -> Error {
    Error::new(
        Status::GenericFailure,
        format!(
            "cannot install the heap statics: `exports.{what}` does not exist. The \
             installer and the addon's exports have drifted apart."
        ),
    )
}

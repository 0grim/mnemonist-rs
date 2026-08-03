//! JS bridge for [`mnemonist_core::structures::multi_set`].
//!
//! # `count` arguments are JavaScript values, not `f64`
//!
//! `add`/`remove`/`set` all run their argument through JavaScript's own
//! numeric coercion before core ever sees a plain number — see
//! `mnemonist_core::structures::multi_set`'s module docs for exactly which
//! rules (`=== 0`, `< 0`, `count = count || 1`, `typeof count !== 'number'`).
//! [`classify_count`] extracts the three facts those rules actually need
//! (`is_number`, JS truthiness, and a `ToNumber`-style coercion for the `< 0`
//! comparison) from the raw argument, and [`resolve_add_or_remove`]/
//! [`resolve_set`] apply them in upstream's own order — the same division of
//! labour every other T2/T3 `typeof` guard in this codebase makes.
//!
//! Only strings, numbers, booleans, `null` and `undefined` are classified in
//! detail; every other type (object, symbol, function, bigint) is treated as
//! an always-truthy, `NaN`-coercing non-number, which is correct for the
//! truthiness half and an approximation for the numeric half. Untested by
//! `test/multi-set.js`, which only ever passes a numeric-looking string
//! (`'56'`) as its one non-number case — see `docs/modules/multi-set.md`.

use std::cell::RefCell;
use std::rc::Rc;

use mnemonist_core::structures::multi_set::{self as core_multi_set, MultiSet as CoreMultiSet};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::foreach;
use crate::js_key::JsKey;

type Core = CoreMultiSet<JsKey>;

const ADD_NOT_NUMBER: &str = "mnemonist/multi-set.add: given count should be a number.";
const SET_NOT_NUMBER: &str = "mnemonist/multi-set.set: given count should be a number.";
const REMOVE_NOT_NUMBER: &str = "mnemonist/multi-set.remove: given count should be a number.";

/// What `add`/`remove`/`set`'s `count` argument classifies as: the three
/// facts upstream's coercion rules need. See the module docs.
struct CountArg {
    is_number: bool,
    truthy: bool,
    /// A `ToNumber`-style coercion, used only for the `< 0` relational
    /// comparison — never for the `typeof` check, which looks at the
    /// *original* value.
    numeric: f64,
}

/// `Number(text)` for the one shape this needs to get right: a plain decimal
/// literal, optionally signed. Not upstream's full numeric-literal grammar
/// (hex, `Infinity`, exponents are approximated by `str::parse`, not
/// guaranteed identical) — untested beyond `'56'`.
fn js_number_like(text: &str) -> f64 {
    let trimmed = text.trim();

    if trimmed.is_empty() {
        return 0.0;
    }

    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

fn classify_count(value: Option<&Unknown>) -> Result<CountArg> {
    let Some(value) = value else {
        return Ok(CountArg {
            is_number: false,
            truthy: false,
            numeric: f64::NAN,
        });
    };

    Ok(match value.get_type()? {
        ValueType::Undefined => CountArg {
            is_number: false,
            truthy: false,
            numeric: f64::NAN,
        },
        ValueType::Null => CountArg {
            is_number: false,
            truthy: false,
            numeric: 0.0,
        },
        ValueType::Number => {
            // SAFETY: `get_type` just reported this exact type.
            let n: f64 = unsafe { value.cast::<f64>()? };

            CountArg {
                is_number: true,
                truthy: n != 0.0 && !n.is_nan(),
                numeric: n,
            }
        }
        ValueType::String => {
            // SAFETY: as above.
            let text: String = unsafe { value.cast::<String>()? };
            let numeric = js_number_like(&text);

            CountArg {
                is_number: false,
                truthy: !text.is_empty(),
                numeric,
            }
        }
        ValueType::Boolean => {
            // SAFETY: as above.
            let flag: bool = unsafe { value.cast::<bool>()? };

            CountArg {
                is_number: false,
                truthy: flag,
                numeric: if flag { 1.0 } else { 0.0 },
            }
        }
        // Object, function, symbol, bigint: always truthy in JavaScript.
        _ => CountArg {
            is_number: false,
            truthy: true,
            numeric: f64::NAN,
        },
    })
}

/// `add`/`remove`'s shared prologue: `if (count === 0) return; if (count < 0)
/// return this.<other>(item, -count); count = count || 1; if (typeof count
/// !== 'number') throw ...`.
///
/// Returns the `f64` to hand to [`CoreMultiSet::add`]/[`CoreMultiSet::remove`]
/// directly — both already implement the same `=== 0`/`< 0`/fold-`NaN`
/// dance internally, so whatever this returns for a non-throwing path is
/// safe to pass straight through.
fn resolve_add_or_remove(count: Option<&Unknown>, message: &'static str) -> Result<f64> {
    let count = classify_count(count)?;

    if count.is_number && count.numeric == 0.0 {
        return Ok(0.0);
    }

    if count.numeric < 0.0 {
        return Ok(count.numeric);
    }

    if count.truthy {
        if !count.is_number {
            return Err(Error::new(Status::GenericFailure, message.to_owned()));
        }

        return Ok(count.numeric);
    }

    Ok(1.0)
}

/// `set`'s prologue: the `typeof` check is unconditional and first, with no
/// early return ahead of it — simpler than `add`/`remove`.
fn resolve_set(count: Option<&Unknown>) -> Result<f64> {
    let classified = classify_count(count)?;

    if !classified.is_number {
        return Err(Error::new(
            Status::GenericFailure,
            SET_NOT_NUMBER.to_owned(),
        ));
    }

    Ok(classified.numeric)
}

/// `top`'s guard: `typeof n !== 'number' || n <= 0`.
fn resolve_top(n: &Unknown) -> Result<usize> {
    let classified = classify_count(Some(n))?;

    if !classified.is_number || classified.numeric <= 0.0 {
        return Err(Error::new(
            Status::GenericFailure,
            core_multi_set::TOP_ARITY.to_owned(),
        ));
    }

    Ok(classified.numeric as usize)
}

#[napi(js_name = "MultiSet")]
pub struct JsMultiSet {
    inner: RefCell<Core>,
}

#[napi]
impl JsMultiSet {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(Core::new()),
        }
    }

    #[napi(getter)]
    pub fn size(&self) -> f64 {
        self.inner.borrow().size()
    }

    /// `i64`, not `u32`: NOTES.md BUG-MULTI-SET-2 can drive this negative (deleting an
    /// item that was never in the set), and the port reproduces that
    /// bug-for-bug rather than silently clamping it.
    #[napi(getter)]
    pub fn dimension(&self) -> i64 {
        self.inner.borrow().dimension()
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    #[napi]
    pub fn add<'a>(&self, this: This<'a>, item: JsKey, count: Option<Unknown>) -> Result<This<'a>> {
        let resolved = resolve_add_or_remove(count.as_ref(), ADD_NOT_NUMBER)?;

        self.inner.borrow_mut().add(item, resolved);

        Ok(this)
    }

    #[napi]
    pub fn set<'a>(&self, this: This<'a>, item: JsKey, count: Option<Unknown>) -> Result<This<'a>> {
        let resolved = resolve_set(count.as_ref())?;

        self.inner.borrow_mut().set(item, resolved);

        Ok(this)
    }

    #[napi]
    pub fn remove<'a>(
        &self,
        this: This<'a>,
        item: JsKey,
        count: Option<Unknown>,
    ) -> Result<This<'a>> {
        let resolved = resolve_add_or_remove(count.as_ref(), REMOVE_NOT_NUMBER)?;

        self.inner.borrow_mut().remove(item, resolved);

        Ok(this)
    }

    #[napi]
    pub fn has(&self, item: JsKey) -> bool {
        self.inner.borrow().has(&item)
    }

    /// `#.delete`. See `mnemonist_core::structures::multi_set`'s module docs
    /// (NOTES.md BUG-MULTI-SET-2) for why this reports `true` and disturbs
    /// `size`/`dimension` even for an item that was never present.
    #[napi(js_name = "delete")]
    pub fn delete(&self, item: JsKey) -> bool {
        self.inner.borrow_mut().delete(&item)
    }

    #[napi]
    pub fn multiplicity(&self, item: JsKey) -> f64 {
        self.inner.borrow().multiplicity(&item)
    }

    /// Upstream's `get`, an alias of `multiplicity`.
    #[napi]
    pub fn get(&self, item: JsKey) -> f64 {
        self.multiplicity(item)
    }

    /// Upstream's `count`, an alias of `multiplicity`.
    #[napi]
    pub fn count(&self, item: JsKey) -> f64 {
        self.multiplicity(item)
    }

    #[napi]
    pub fn frequency(&self, item: JsKey) -> f64 {
        self.inner.borrow().frequency(&item)
    }

    /// Upstream's `edit`, always returning `this` — see the core module docs
    /// on why the untested `undefined`-on-a-missing-`a` return is not
    /// modelled.
    #[napi]
    pub fn edit<'a>(&self, this: This<'a>, a: JsKey, b: JsKey) -> This<'a> {
        self.inner.borrow_mut().edit(a, b);

        this
    }

    #[napi]
    pub fn top(&self, n: Unknown) -> Result<Vec<CountPair>> {
        let n = resolve_top(&n)?;

        let survivors = self
            .inner
            .borrow()
            .top(n)
            .map_err(|message| Error::new(Status::GenericFailure, message.to_owned()))?;

        Ok(survivors
            .into_iter()
            .map(|(item, count)| CountPair(item, count))
            .collect())
    }

    /// Upstream's `forEach`: the item, `multiplicity` times, `(value,
    /// value)` per call.
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(JsKey, JsKey)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let mut cursor = self.inner.borrow().repeat_cursor();

        let mut step = || {
            let inner = self.inner.borrow();

            cursor.step(inner.items())
        };

        while let Some(item) = step() {
            let args = FnArgs::from((item.clone(), item));

            match &scope {
                Some(scope) => callback.apply(*scope, args)?,
                None => callback.apply(this, args)?,
            };
        }

        Ok(())
    }

    /// Upstream's `forEachMultiplicity`: `this.items.forEach(callback,
    /// scope)` directly.
    #[napi(js_name = "forEachMultiplicity")]
    pub fn for_each_multiplicity(
        &self,
        this: This,
        callback: Function<FnArgs<(f64, JsKey)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let mut cursor = mnemonist_core::map::MapCursor::open();

        let mut step = || {
            let inner = self.inner.borrow();

            cursor
                .step(inner.items())
                .map(|(item, count)| (item.clone(), *count))
        };

        while let Some((item, count)) = step() {
            let args = FnArgs::from((count, item));

            match &scope {
                Some(scope) => callback.apply(*scope, args)?,
                None => callback.apply(this, args)?,
            };
        }

        Ok(())
    }

    #[napi]
    pub fn keys(&self, env: Env, this: Reference<JsMultiSet>) -> Result<JsMultiSetKeys> {
        Ok(JsMultiSetKeys {
            source: this.share_with(env, |set| Ok(&set.inner))?,
            state: mnemonist_core::map::MapCursor::open(),
        })
    }

    #[napi]
    pub fn multiplicities(
        &self,
        env: Env,
        this: Reference<JsMultiSet>,
    ) -> Result<JsMultiSetMultiplicities> {
        Ok(JsMultiSetMultiplicities {
            source: this.share_with(env, |set| Ok(&set.inner))?,
            state: mnemonist_core::map::MapCursor::open(),
        })
    }

    /// Upstream's `values`, and its `Symbol.iterator` — the flattened
    /// (`item` repeated `multiplicity` times) walk.
    #[napi]
    pub fn values(&self, env: Env, this: Reference<JsMultiSet>) -> Result<JsMultiSetValues> {
        Ok(JsMultiSetValues {
            source: this.share_with(env, |set| Ok(&set.inner))?,
            state: mnemonist_core::structures::multi_set::RepeatCursor::open(),
        })
    }

    /// `MultiSet.from(iterable)`: `forEach(iterable, function(value) {
    /// set.add(value); })`.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown) -> Result<Self> {
        let values = collect_values(&env, iterable)?;
        let set = Self::new();

        {
            let mut inner = set.inner.borrow_mut();

            for value in values {
                inner.add(value, 1.0);
            }
        }

        Ok(set)
    }

    /// `MultiSet.isSubset(A, B)`.
    #[napi(js_name = "isSubset")]
    pub fn is_subset(a: ClassInstance<JsMultiSet>, b: ClassInstance<JsMultiSet>) -> bool {
        core_multi_set::is_subset(&a.inner.borrow(), &b.inner.borrow())
    }

    /// `MultiSet.isSuperset(A, B)`.
    #[napi(js_name = "isSuperset")]
    pub fn is_superset(a: ClassInstance<JsMultiSet>, b: ClassInstance<JsMultiSet>) -> bool {
        core_multi_set::is_superset(&a.inner.borrow(), &b.inner.borrow())
    }
}

impl Default for JsMultiSet {
    fn default() -> Self {
        Self::new()
    }
}

/// `[item, count]`, for `#.top`/`#.multiplicities`.
pub struct CountPair(JsKey, f64);

impl ToNapiValue for CountPair {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let key = unsafe { ToNapiValue::to_napi_value(env, val.0)? };
        let count = unsafe { ToNapiValue::to_napi_value(env, val.1)? };

        let mut pair = std::ptr::null_mut();
        napi::check_status!(
            unsafe { sys::napi_create_array_with_length(env, 2, &mut pair) },
            "mnemonist-rs: failed to build a [item, count] pair"
        )?;
        napi::check_status!(
            unsafe { sys::napi_set_element(env, pair, 0, key) },
            "mnemonist-rs: failed to set a pair's item"
        )?;
        napi::check_status!(
            unsafe { sys::napi_set_element(env, pair, 1, count) },
            "mnemonist-rs: failed to set a pair's count"
        )?;

        Ok(pair)
    }
}

/// Every value the `for_each`-style dispatch visits, classified straight
/// into [`JsKey`] — the shape `MultiSet.from`'s collector needs. Mirrors
/// `bi_map`/`multi_map`'s own local `collect_pairs`, one value narrower.
fn collect_values(env: &Env, iterable: Unknown) -> Result<Vec<JsKey>> {
    let sink = Rc::new(RefCell::new(Vec::<JsKey>::new()));
    let collected = Rc::clone(&sink);

    let collector: Function<'_, Unknown, ()> =
        env.create_function_from_closure("collect_values", move |context| {
            let value: Unknown = match context.length() {
                0 => foreach::undefined(context.env)?,
                _ => context.get(0)?,
            };

            collected.borrow_mut().push(JsKey::from_unknown(&value)?);

            Ok(())
        })?;

    // SAFETY: `collector` is a JS function this call just created.
    let collector = unsafe { Unknown::from_raw_unchecked(env.raw(), collector.raw()) };

    foreach::for_each(env, iterable, collector)?;

    let values = std::mem::take(&mut *sink.borrow_mut());

    Ok(values)
}

#[napi(iterator, js_name = "MultiSetKeys")]
pub struct JsMultiSetKeys {
    source: SharedReference<JsMultiSet, &'static RefCell<Core>>,
    state: mnemonist_core::map::MapCursor,
}

impl Generator for JsMultiSetKeys {
    type Yield = JsKey;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<JsKey> {
        let borrowed = self.source.borrow();

        self.state
            .step(borrowed.items())
            .map(|(item, _)| item.clone())
    }

    fn complete(&mut self, _value: Option<()>) -> Option<JsKey> {
        None
    }
}

#[napi(iterator, js_name = "MultiSetMultiplicities")]
pub struct JsMultiSetMultiplicities {
    source: SharedReference<JsMultiSet, &'static RefCell<Core>>,
    state: mnemonist_core::map::MapCursor,
}

impl Generator for JsMultiSetMultiplicities {
    type Yield = CountPair;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<CountPair> {
        let borrowed = self.source.borrow();

        self.state
            .step(borrowed.items())
            .map(|(item, count)| CountPair(item.clone(), *count))
    }

    fn complete(&mut self, _value: Option<()>) -> Option<CountPair> {
        None
    }
}

#[napi(iterator, js_name = "MultiSetValues")]
pub struct JsMultiSetValues {
    source: SharedReference<JsMultiSet, &'static RefCell<Core>>,
    state: mnemonist_core::structures::multi_set::RepeatCursor<JsKey>,
}

impl Generator for JsMultiSetValues {
    type Yield = JsKey;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<JsKey> {
        let borrowed = self.source.borrow();

        self.state.step(borrowed.items())
    }

    fn complete(&mut self, _value: Option<()>) -> Option<JsKey> {
        None
    }
}

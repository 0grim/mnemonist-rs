//! JS bridge for [`mnemonist_core::structures::passjoin_index`].
//!
//! # Re-entrancy, same shape as `crate::bk_tree`
//!
//! `levenshtein` is a JS callback invoked repeatedly *from inside*
//! [`CoreIndex::try_search`]'s own loop, the identical shape
//! `crate::bk_tree`'s `distance` has: the `RefCell` borrow taken for the
//! whole `search` call cannot be released between distance calls, because
//! core owns the loop and knows nothing about `RefCell`. So, exactly as
//! `bk_tree.rs`:
//!
//! * `search` returns `Result`, because the borrow can fail;
//! * `read`/`write` use `try_borrow`/`try_borrow_mut`, never the panicking
//!   forms — a `RefCell` panic inside a `#[napi]` method aborts the process;
//! * a `levenshtein` that calls back into the *same* index while it is
//!   searching meets that outstanding borrow and gets a clear, catchable
//!   error instead of a crash.
//!
//! Unlike `bk_tree`, the distance function is not stored on the struct at
//! all — see `mnemonist_core::structures::passjoin_index::PassjoinIndex`'s
//! own docs for why it is a parameter of `try_search` instead, which lets
//! this bridge build a fresh [`FunctionRef`]-backed closure per call from
//! whatever `levenshtein` the caller currently has, exactly mirroring
//! upstream's `this.levenshtein(...)`.
//!
//! # `also accepts an array-like of characters` is not modelled
//!
//! Upstream's `add`/`search`/`.length` work over anything with a numeric
//! `.length` and indexed access, including a plain array of characters —
//! `test/passjoin-index.js` only ever passes strings, so only `String` is
//! accepted here; anything else is a type error at the boundary rather than
//! a silent misinterpretation.

use std::cell::{Ref, RefCell, RefMut};

use mnemonist_core::structures::passjoin_index::{Error as CoreError, PassjoinIndex as CoreIndex};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

/// `mnemonist/passjoin-index: \`levenshtein\` should be a function returning
/// edit distance between two strings.`
const NOT_A_FUNCTION: &str = mnemonist_core::structures::passjoin_index::INVALID_LEVENSHTEIN;

const REENTRANT_LEVENSHTEIN: &str =
    "mnemonist-rs/PassjoinIndex: the levenshtein function called back into the index while it \
     was searching. Upstream would serve such a call from an index that is mid-search; this \
     port refuses it instead, catchably. See `bk_tree.rs`'s module docs for the same shape.";

type Levenshtein = FunctionRef<FnArgs<(String, String)>, f64>;

fn raise(error: CoreError) -> Error {
    Error::new(Status::InvalidArg, error.to_string())
}

/// A real JS `Set`, built fresh from a `Vec<String>` — upstream's `search`
/// returns a genuine `Set`, and `assert.deepStrictEqual` distinguishes a
/// `Set` from an `Array` of the same elements (different constructors), so
/// rendering this as a plain array would fail the original suite outright.
pub struct RenderedSet(Vec<String>);

impl ToNapiValue for RenderedSet {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let js_env = Env::from_raw(env);
        let array = Array::from_vec(&js_env, val.0)?;

        let global = js_env.get_global()?;
        let constructor: Function<'_, Array, Unknown> =
            global.get_named_property_unchecked("Set")?;
        let instance = constructor.new_instance(array)?;

        Ok(instance.raw())
    }
}

/// `#.values()`'s iterator — a snapshot taken at creation, drained in order.
#[napi(iterator, js_name = "PassjoinIndexValues")]
pub struct JsPassjoinIndexValues {
    values: std::vec::IntoIter<String>,
}

impl Generator for JsPassjoinIndexValues {
    type Yield = String;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<String> {
        self.values.next()
    }

    fn complete(&mut self, _value: Option<()>) -> Option<String> {
        None
    }
}

#[napi(js_name = "PassjoinIndex")]
pub struct JsPassjoinIndex {
    inner: RefCell<CoreIndex>,
    levenshtein: Levenshtein,
}

impl JsPassjoinIndex {
    fn read(&self) -> Result<Ref<'_, CoreIndex>> {
        self.inner
            .try_borrow()
            .map_err(|_| Error::new(Status::GenericFailure, REENTRANT_LEVENSHTEIN))
    }

    fn write(&self) -> Result<RefMut<'_, CoreIndex>> {
        self.inner
            .try_borrow_mut()
            .map_err(|_| Error::new(Status::GenericFailure, REENTRANT_LEVENSHTEIN))
    }

    fn call_levenshtein(&self, env: &Env, a: &str, b: &str) -> Result<i64> {
        let callable = self.levenshtein.borrow_back(env)?;
        let result = callable.call((a.to_owned(), b.to_owned()).into())?;

        Ok(result as i64)
    }
}

#[napi]
impl JsPassjoinIndex {
    /// `new PassjoinIndex(levenshtein, k)`.
    #[napi(constructor)]
    pub fn new(levenshtein: Unknown, k: Option<f64>) -> Result<Self> {
        // Checked first, and against an `Option` rather than a required
        // `f64`: upstream checks `levenshtein` before `k`, so `new
        // PassjoinIndex(null)` (`k` omitted entirely) must still fail on
        // `/levenshtein/i`, not on napi's own "missing argument" message for
        // a required `f64` it never gets to see.
        if levenshtein.get_type()? != ValueType::Function {
            return Err(Error::new(Status::InvalidArg, NOT_A_FUNCTION));
        }

        // SAFETY: `get_type` has just reported `Function`.
        let function = unsafe { levenshtein.cast::<Function<FnArgs<(String, String)>, f64>>()? };

        let Some(k) = k else {
            return Err(raise(CoreError::InvalidK));
        };

        let inner = CoreIndex::new(k as i64).map_err(raise)?;

        Ok(Self {
            inner: RefCell::new(inner),
            levenshtein: function.create_ref()?,
        })
    }

    #[napi(getter)]
    pub fn size(&self) -> Result<u32> {
        Ok(self.read()?.size() as u32)
    }

    #[napi(getter)]
    pub fn k(&self) -> Result<f64> {
        Ok(self.read()?.k() as f64)
    }

    #[napi]
    pub fn clear(&self) -> Result<()> {
        self.write()?.clear();

        Ok(())
    }

    /// Upstream's `add`, which returns `this` for chaining.
    #[napi]
    pub fn add<'a>(&self, this: This<'a>, value: String) -> Result<This<'a>> {
        self.write()?.add(&value);

        Ok(this)
    }

    /// Upstream's `search`, returning a real `Set` — see [`RenderedSet`].
    #[napi]
    pub fn search(&self, env: Env, query: String) -> Result<RenderedSet> {
        let index = self.read()?;

        let matches = index.try_search(&query, |a, b| self.call_levenshtein(&env, a, b))?;

        Ok(RenderedSet(matches))
    }

    /// `#.forEach(callback)`.
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(String, u32)>, Unknown>,
    ) -> Result<()> {
        let index = self.read()?;
        let mut error = None;

        index.for_each(|string, i| {
            if error.is_some() {
                return;
            }

            if let Err(err) = callback.apply(this, (string.to_owned(), i as u32).into()) {
                error = Some(err);
            }
        });

        match error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    /// `#.values()` — installed onto `Symbol.iterator` too (see
    /// `crate::cursor::ITERATOR_FACTORIES`). A genuine iterator, not a plain
    /// array: `Symbol.iterator` is aliased to this method, and `for...of`
    /// requires whatever it returns to itself have a `.next()` -- an array
    /// does not. Snapshotted at creation, upstream's own closure captures
    /// `strings`/`l` once and reads live, but nothing in this index can
    /// shrink `strings` (only `clear()` resets it wholesale), so a snapshot
    /// and a live read agree on every reachable input.
    #[napi]
    pub fn values(&self) -> Result<JsPassjoinIndexValues> {
        Ok(JsPassjoinIndexValues {
            values: self.read()?.values().to_vec().into_iter(),
        })
    }

    /// `PassjoinIndex.from(iterable, levenshtein, k)`.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown, levenshtein: Unknown, k: f64) -> Result<Self> {
        let built = Self::new(levenshtein, Some(k))?;

        for slot in crate::foreach::collect(&env, iterable)? {
            let value = slot.get(&env)?;
            let string = String::from_unknown(value)?;

            built.write()?.add(&string);
        }

        Ok(built)
    }
}

/// Truncate a JS number to `i64`, the width every core helper below takes.
/// These are internal arithmetic helpers exercised directly by
/// `test/passjoin-index.js`, not user-facing boundary values, so a plain
/// truncating cast (rather than `vector.rs`'s `count`'s clamp-to-`usize`) is
/// what upstream's own untyped arithmetic effectively does too.
fn int(value: f64) -> i64 {
    value as i64
}

/// `PassjoinIndex.countKeys`/`comparator`/`partition`/`segments`/
/// `segmentPos`/`multiMatchAwareInterval`/`multiMatchAwareSubstrings` —
/// upstream's seven static helpers, each a real, independently tested
/// export on the class (not merely internal to `add`/`search`).
///
/// napi-rs has no way to attach a static method directly to another
/// class's exported constructor, so — exactly as `crate::heap`'s
/// `HeapStatics` does for `Heap` — these are exported under a throwaway
/// class of their own and [`install_passjoin_index_statics`] copies each
/// one onto `PassjoinIndex` at module load, then deletes the scaffolding
/// class from `exports`.
#[napi(js_name = "PassjoinIndexStatics")]
pub struct JsPassjoinIndexStatics;

#[napi]
impl JsPassjoinIndexStatics {
    #[napi(js_name = "countKeys")]
    pub fn static_count_keys(k: f64, s: f64) -> f64 {
        mnemonist_core::structures::passjoin_index::count_keys(int(k), int(s)) as f64
    }

    /// `-1`/`0`/`1`, matching upstream's own comparator return convention
    /// (used with `Array.prototype.sort`, which only inspects the sign).
    #[napi(js_name = "comparator")]
    pub fn static_comparator(a: String, b: String) -> i32 {
        match mnemonist_core::structures::passjoin_index::comparator(&a, &b) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    #[napi(js_name = "partition")]
    pub fn static_partition(k: f64, l: f64) -> Vec<Vec<f64>> {
        mnemonist_core::structures::passjoin_index::partition(int(k), int(l))
            .into_iter()
            .map(|(start, len)| vec![start as f64, len as f64])
            .collect()
    }

    #[napi(js_name = "segments")]
    pub fn static_segments(k: f64, string: String) -> Vec<String> {
        mnemonist_core::structures::passjoin_index::segments(int(k), &string)
    }

    #[napi(js_name = "segmentPos")]
    pub fn static_segment_pos(k: f64, i: f64, string: String) -> f64 {
        mnemonist_core::structures::passjoin_index::segment_pos(int(k), int(i), &string) as f64
    }

    #[napi(js_name = "multiMatchAwareInterval")]
    pub fn static_multi_match_aware_interval(
        k: f64,
        delta: f64,
        i: f64,
        s: f64,
        pi: f64,
        li: f64,
    ) -> Vec<f64> {
        let (start, stop) = mnemonist_core::structures::passjoin_index::multi_match_aware_interval(
            int(k),
            int(delta),
            int(i),
            int(s),
            int(pi),
            int(li),
        );

        vec![start as f64, stop as f64]
    }

    #[napi(js_name = "multiMatchAwareSubstrings")]
    pub fn static_multi_match_aware_substrings(
        k: f64,
        string: String,
        l: f64,
        i: f64,
        pi: f64,
        li: f64,
    ) -> Vec<String> {
        mnemonist_core::structures::passjoin_index::multi_match_aware_substrings(
            int(k),
            &string,
            int(l),
            int(i),
            int(pi),
            int(li),
        )
    }
}

const STATIC_NAMES: &[&str] = &[
    "countKeys",
    "comparator",
    "partition",
    "segments",
    "segmentPos",
    "multiMatchAwareInterval",
    "multiMatchAwareSubstrings",
];

const INSTALLER: &str = "(function (PassjoinIndex, statics, names) { \
     names.forEach(function (name) { \
       PassjoinIndex[name] = statics[name].bind(statics); \
     }); \
   })";

/// Copy every static in [`JsPassjoinIndexStatics`] onto `PassjoinIndex` and
/// remove the scaffolding class from `exports` -- leaving exactly upstream's
/// surface, same as `crate::heap::install_heap_statics`.
pub fn install_passjoin_index_statics(exports: &mut Object, env: &Env) -> Result<()> {
    let constructor: Unknown = exports
        .get("PassjoinIndex")?
        .ok_or_else(|| missing("PassjoinIndex"))?;
    let statics: Unknown = exports
        .get("PassjoinIndexStatics")?
        .ok_or_else(|| missing("PassjoinIndexStatics"))?;
    let installer: Function<'_, FnArgs<(Unknown, Unknown, Vec<String>)>, Unknown> =
        env.run_script(INSTALLER)?;

    let names: Vec<String> = STATIC_NAMES.iter().map(|s| (*s).to_owned()).collect();

    installer.call((constructor, statics, names).into())?;

    exports.delete_named_property("PassjoinIndexStatics")?;

    Ok(())
}

fn missing(what: &str) -> Error {
    Error::new(
        Status::GenericFailure,
        format!(
            "cannot install the passjoin-index statics: `exports.{what}` does not exist. The \
             installer and the addon's exports have drifted apart."
        ),
    )
}

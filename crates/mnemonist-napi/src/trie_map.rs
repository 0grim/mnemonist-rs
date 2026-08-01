//! JS bridge for [`mnemonist_core::structures::trie_map`].
//!
//! Three things happen here that no earlier bridge needs, all specific to a
//! trie's own shape.
//!
//! # 1. A token is a coerced string, not a `JsKey`
//!
//! Upstream indexes a plain object — `node[token]` — so a token's identity
//! *is* its property-key string, not its SameValueZero identity: `1` and
//! `"1"` are different `Map` keys but the same trie edge, because
//! `ToPropertyKey` coerces both the same way. Verified against real Node
//! 24.18.1: `new TrieMap(Array)`, `set([1], 'num')` then `set(['1'], 'str')`
//! leaves `size === 1` — the second `set` overwrote the first.
//!
//! [`Token`] is therefore `Rc<[u16]>` — UTF-16 code units, matching how
//! `crate::foreach`'s branch-1 string walk and `crate::js_slot::read_utf16`
//! already represent JS strings, for the surrogate-pair reason those give.
//! One instantiation of the core engine, `TrieMap<Token, Value>`, serves
//! both of upstream's constructor modes:
//!
//! * **string mode** (`Token` omitted, or anything but the real `Array`):
//!   each token is one code unit of the prefix string — `prefix[i]` on a
//!   string always yields a single-character string, so no coercion is
//!   needed at all.
//! * **array mode** (`Token === Array`, checked by identity against the real
//!   global exactly as [`crate::sparse_map`] resolves `Values`): each array
//!   element becomes one token, coerced with the same `String(value)` round
//!   trip [`crate::foreach::display`] already performs for its own
//!   `toString()` fallback — not upstream's full `ToPropertyKey` (which also
//!   accepts a `Symbol` unchanged), because no test in either original suite
//!   ever supplies one. See D-91's precedent for the same judgement call on
//!   `lru-cache`'s object keys.
//!
//! # 2. `find`/`values`/`keys`/`entries` echo the caller's own prefix, not a
//! rebuilt one
//!
//! Verified against real Node 24.18.1: `new TrieMap(Array)`,
//! `set([1, 2, 3], 'v')`, then `find([1])` answers `[[[1, "2", "3"], 'v']]` —
//! the **first** element of the result is the raw number `1`, the caller's
//! own array element, while `"2"` and `"3"` are coerced strings discovered
//! during the walk. Upstream's own `find`/`values`/`keys`/`entries` never
//! rebuild the prefix argument; they hold the caller's actual value
//! (`prefixStack.push(prefix)`) and only ever `.concat()`/`+` newly
//! discovered tokens onto it.
//!
//! `mnemonist_core::structures::trie_map::TrieMap::find`/`walk` therefore
//! return only the **suffix** beyond a given prefix (see that module's
//! docs), and [`assemble`] is where the two halves are rejoined: the
//! original argument, captured once as a [`PrefixEcho`], concatenated with
//! the suffix's coerced-string tokens.
//!
//! # 3. `RefCell`, again, for the same reason as every re-entrant bridge here
//!
//! `update`'s callback and the map it is updating can alias — a callback
//! that calls back into the same trie is legal JavaScript — so the core
//! structure is held in a `RefCell` and no borrow is ever alive across a
//! call into JavaScript. See `crate::default_map`'s module docs, and B-31.

use std::cell::RefCell;
use std::ptr;
use std::rc::Rc;

use mnemonist_core::structures::trie_map::{TrieMap as CoreMap, Walk as CoreWalk};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::foreach::{check, coerce_to_object, display, is_array};
use crate::js_slot::{read_utf16, JsSlot};
use crate::js_value::{release_slot, Loaned, Received, Retained};

/// One trie edge label: a coerced string, held as UTF-16 code units.
pub(crate) type Token = Rc<[u16]>;

/// A stored value slot. `None` is `undefined` — a stored word can hold it
/// (`trie.update('a', () => undefined)` is legal upstream, just as
/// `default-map` distinguishes a missing key from one holding `undefined`),
/// and `TrieMap::has` (word presence) already does not care which this is.
pub(crate) type Value = Option<Retained>;

/// The engine both `set`-shaped bridges share.
pub(crate) type Core = CoreMap<Token, Value>;

/// [`Loaned::of`], flattened through the extra `Option` a `Value` slot adds:
/// "no word here" and "a word here holding `undefined`" are the same
/// `undefined` to every upstream caller.
fn loan(value: Option<&Value>) -> Loaned {
    Loaned::of(value.and_then(Option::as_ref))
}

/// Which of upstream's two constructor modes a trie was built with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// `Token` omitted, or anything but the real `Array` — each character of
    /// a prefix string is one token.
    Str,
    /// `Token === Array` — each array element is one token.
    Array,
}

/// What to echo back as the "already consumed" head of a result, captured
/// once when a prefix argument is decoded.
///
/// Separate from [`Token`] deliberately: the echoed head keeps the caller's
/// *actual* values (a raw number, a specific object) where a discovered
/// suffix token is always a coerced string. See the module docs, part 2.
#[derive(Debug, Clone)]
pub(crate) enum PrefixEcho {
    Str(Vec<u16>),
    Array(Vec<JsSlot>),
}

/// Read a `prefix` argument into the tokens core navigates by, and the echo
/// a result is assembled from. `arg: None` is upstream's omitted argument —
/// the default empty prefix, whichever mode this trie is in.
pub(crate) fn decode_prefix(
    env: &Env,
    mode: Mode,
    arg: Option<&Unknown>,
) -> Result<(Vec<Token>, PrefixEcho)> {
    match mode {
        Mode::Str => {
            let units: Vec<u16> = match arg {
                Some(value) if value.get_type()? == ValueType::String => read_utf16(env, value)?,
                // Anything else -- omitted, or a non-string value no test
                // ever passes in string mode -- behaves as the empty prefix.
                _ => Vec::new(),
            };
            let tokens: Vec<Token> = units.iter().map(|&unit| Rc::from(vec![unit])).collect();

            Ok((tokens, PrefixEcho::Str(units)))
        }
        Mode::Array => {
            let mut tokens = Vec::new();
            let mut echo = Vec::new();

            if let Some(value) = arg {
                if is_array(env, value)? {
                    let object = coerce_to_object(env, value)?;
                    let length: Unknown = object.get_named_property_unchecked("length")?;
                    let length = crate::foreach::to_number(env, &length)? as u32;

                    for index in 0..length {
                        let element: Unknown = object.get_element(index)?;

                        echo.push(JsSlot::new(env, &element)?);

                        // `String(element)`, not full `ToPropertyKey` -- see
                        // the module docs, part 1.
                        let coerced = display(env, &element)?;
                        tokens.push(Rc::from(coerced.encode_utf16().collect::<Vec<u16>>()));
                    }
                }
            }

            Ok((tokens, PrefixEcho::Array(echo)))
        }
    }
}

/// Rejoin a walk's echoed head with a discovered suffix into the JS value
/// upstream's `prefix + k` / `prefix.concat(k)` would produce.
fn assemble(env: &Env, mode: Mode, echo: &PrefixEcho, suffix: &[Token]) -> Result<sys::napi_value> {
    match (mode, echo) {
        (Mode::Str, PrefixEcho::Str(units)) => {
            let mut full = units.clone();

            for token in suffix {
                full.extend_from_slice(token);
            }

            let string = env.create_string_utf16(&full)?;

            Ok(string.raw())
        }
        (Mode::Array, PrefixEcho::Array(elements)) => {
            let total = elements.len() + suffix.len();
            let mut array = ptr::null_mut();

            check(
                unsafe { sys::napi_create_array_with_length(env.raw(), total, &mut array) },
                "napi_create_array_with_length",
            )?;

            for (index, slot) in elements.iter().enumerate() {
                let value = slot.get(env)?;

                check(
                    unsafe { sys::napi_set_element(env.raw(), array, index as u32, value.raw()) },
                    "napi_set_element",
                )?;
            }

            for (offset, token) in suffix.iter().enumerate() {
                let string = env.create_string_utf16(token)?;

                check(
                    unsafe {
                        sys::napi_set_element(
                            env.raw(),
                            array,
                            (elements.len() + offset) as u32,
                            string.raw(),
                        )
                    },
                    "napi_set_element",
                )?;
            }

            Ok(array)
        }
        (Mode::Str, PrefixEcho::Array(_)) | (Mode::Array, PrefixEcho::Str(_)) => {
            unreachable!("mode and echo are always constructed together by decode_prefix")
        }
    }
}

/// `Token === Array`, resolved by identity against the real global — shared
/// by `JsTrieMap::new` and `crate::trie::JsTrie::new`, which decide the same
/// way.
pub(crate) fn resolve_mode(env: &Env, token: Option<Unknown>) -> Result<Mode> {
    let Some(value) = token else {
        return Ok(Mode::Str);
    };

    let global = env.get_global()?;
    let array_ctor: Unknown = global.get_named_property_unchecked("Array")?;

    Ok(if env.strict_equals(value, array_ctor)? {
        Mode::Array
    } else {
        Mode::Str
    })
}

/// Rebuild upstream's `root` property: a plain object, nested one level per
/// token, with `render_value` filling in whatever a `Word` slot holds.
///
/// Generic over `V` (not `T`: every caller's token is [`Token`]) so both
/// `JsTrieMap` (a real stored value) and `crate::trie::JsTrie` (a bare `true`)
/// share one recursive walk.
pub(crate) fn build_root<V>(
    env: &Env,
    node: mnemonist_core::structures::trie_map::NodeView<'_, Token, V>,
    render_value: &dyn Fn(&V) -> Result<sys::napi_value>,
) -> Result<sys::napi_value> {
    use mnemonist_core::structures::trie_map::Entry;

    let mut object = ptr::null_mut();
    check(
        unsafe { sys::napi_create_object(env.raw(), &mut object) },
        "napi_create_object",
    )?;

    for entry in node.entries() {
        match entry {
            Entry::Word(value) => {
                let js_value = render_value(value)?;

                set_property_utf16(env, object, &[0u16], js_value)?;
            }
            Entry::Child(token, child) => {
                let child_object = build_root(env, child, render_value)?;

                set_property_utf16(env, object, token, child_object)?;
            }
        }
    }

    Ok(object)
}

/// `object[key] = value`, where `key` is UTF-16 code units rather than a
/// UTF-8 `&str` — needed because a real key can be the sentinel (code point
/// `0`) or, in array mode, any coerced string, neither of which
/// `set_named_property`'s C-string key can carry safely.
fn set_property_utf16(
    env: &Env,
    object: sys::napi_value,
    key: &[u16],
    value: sys::napi_value,
) -> Result<()> {
    let key = env.create_string_utf16(key)?;

    check(
        unsafe { sys::napi_set_property(env.raw(), object, key.raw(), value) },
        "napi_set_property",
    )
}

/// [`Retained::new`], with `undefined` resolved to `None` rather than
/// rejected — the shape [`Received`]'s `FromNapiValue` gives a function
/// parameter, needed here because `.from()`'s collected values arrive as
/// [`JsSlot`]s instead.
fn retained_from_unknown(value: &Unknown) -> Result<Value> {
    if value.get_type()? == ValueType::Undefined {
        return Ok(None);
    }

    Retained::new(value).map(Some)
}

/// Everything [`crate::foreach::for_each`] would visit, as `(value, key)`
/// [`JsSlot`] pairs — the shape `Trie.from`/`TrieMap.from`'s collector needs.
///
/// A local, minimal collector rather than a change to
/// [`crate::foreach::collect`], for the same reason `crate::bi_map`'s own
/// `collect_pairs` is local: that helper returns `Vec<JsSlot>` for a
/// single-value walk, and this needs both the value *and* the key the
/// dispatch hands over. Kept as [`JsSlot`] rather than [`crate::js_key::JsKey`]
/// (which `crate::bi_map`'s version collects into): a trie key becomes a
/// *prefix*, decoded by [`decode_prefix`], not compared by SameValueZero.
fn collect_slot_pairs(env: &Env, iterable: Unknown) -> Result<Vec<(JsSlot, JsSlot)>> {
    use std::rc::Rc;

    let sink = Rc::new(std::cell::RefCell::new(Vec::<(JsSlot, JsSlot)>::new()));
    let collected = Rc::clone(&sink);

    let collector: Function<'_, Unknown, ()> =
        env.create_function_from_closure("collect_slot_pairs", move |context| {
            let value: Unknown = match context.length() {
                0 => crate::foreach::undefined(context.env)?,
                _ => context.get(0)?,
            };
            let key: Unknown = match context.length() {
                len if len > 1 => context.get(1)?,
                _ => crate::foreach::undefined(context.env)?,
            };

            collected.borrow_mut().push((
                JsSlot::new(context.env, &value)?,
                JsSlot::new(context.env, &key)?,
            ));

            Ok(())
        })?;

    // SAFETY: `collector` is a JS function this call just created.
    let collector = unsafe { Unknown::from_raw_unchecked(env.raw(), collector.raw()) };

    crate::foreach::for_each(env, iterable, collector)?;

    let pairs = std::mem::take(&mut *sink.borrow_mut());

    Ok(pairs)
}

/// A suffix, plus everything needed to assemble it into a full result when
/// napi converts it — mode and echo travel with every yielded value because
/// `ToNapiValue::to_napi_value` is the first point an `Env` is available
/// again after `Generator::next` returns.
pub struct AssembledPrefix {
    mode: Mode,
    echo: PrefixEcho,
    suffix: Vec<Token>,
}

impl AssembledPrefix {
    /// Used by `crate::trie` too, whose own cursor does not go through
    /// [`WalkCursor::assembled`] (a `Trie` has no value to project, so it
    /// does not share that cursor type — see that module's docs).
    pub(crate) fn new(mode: Mode, echo: PrefixEcho, suffix: Vec<Token>) -> Self {
        Self { mode, echo, suffix }
    }
}

impl ToNapiValue for AssembledPrefix {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let env = Env::from_raw(env);

        assemble(&env, val.mode, &val.echo, &val.suffix)
    }
}

/// One `[prefix, value]` pair, as `entries()` yields it.
pub struct AssembledEntry {
    pub(crate) prefix: AssembledPrefix,
    pub(crate) value: Loaned,
}

impl ToNapiValue for AssembledEntry {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let prefix = unsafe { AssembledPrefix::to_napi_value(env, val.prefix)? };
        let value = unsafe { ToNapiValue::to_napi_value(env, val.value)? };

        let mut pair = ptr::null_mut();
        check(
            unsafe { sys::napi_create_array_with_length(env, 2, &mut pair) },
            "napi_create_array_with_length",
        )?;
        check(
            unsafe { sys::napi_set_element(env, pair, 0, prefix) },
            "napi_set_element",
        )?;
        check(
            unsafe { sys::napi_set_element(env, pair, 1, value) },
            "napi_set_element",
        )?;

        Ok(pair)
    }
}

/// A resumable walk over a JS-owned trie: core's [`CoreWalk`] plus a live
/// handle to the parent, in the same shape as `crate::cursor::CellCursor` —
/// re-borrowed per step, never held across a call into JavaScript.
///
/// Not built on `crate::cursor` at all: [`CoreWalk`] is not a
/// `mnemonist_core::cursor::Sequence` (there is no frozen length; see that
/// module's docs), so it needs its own, smaller wrapper here.
pub(crate) struct WalkCursor<Owner: 'static, V: 'static> {
    source: SharedReference<Owner, &'static RefCell<CoreMap<Token, V>>>,
    walk: CoreWalk<Token>,
    mode: Mode,
    echo: PrefixEcho,
}

impl<Owner: 'static, V: 'static> WalkCursor<Owner, V> {
    /// Freeze nothing — this is `TrieMap::walk`'s job, and it is re-run
    /// here so the walk starts from the same tokens the prefix argument
    /// decoded to.
    pub(crate) fn open(
        source: SharedReference<Owner, &'static RefCell<CoreMap<Token, V>>>,
        tokens: Vec<Token>,
        mode: Mode,
        echo: PrefixEcho,
    ) -> Self {
        let walk = source.borrow().walk(tokens);

        Self {
            source,
            walk,
            mode,
            echo,
        }
    }

    /// One step, projecting the stored value out from behind the live
    /// borrow before it is dropped.
    pub(crate) fn step<T>(&mut self, project: impl FnOnce(&V) -> T) -> Option<(Vec<Token>, T)> {
        let inner = self.source.borrow();

        self.walk
            .step(&inner)
            .map(|(suffix, value)| (suffix, project(value)))
    }

    /// As [`step`](WalkCursor::step), when the caller wants only the suffix.
    pub(crate) fn suffix_only(&mut self) -> Option<Vec<Token>> {
        self.step(|_value| ()).map(|(suffix, ())| suffix)
    }

    /// Package a suffix this cursor produced into something napi can convert.
    pub(crate) fn assembled(&self, suffix: Vec<Token>) -> AssembledPrefix {
        AssembledPrefix {
            mode: self.mode,
            echo: self.echo.clone(),
            suffix,
        }
    }
}

/// Upstream's `TrieMap`.
#[napi(js_name = "TrieMap", custom_finalize)]
pub struct JsTrieMap {
    inner: RefCell<Core>,
    mode: Mode,
}

#[napi]
impl JsTrieMap {
    /// `new TrieMap(Token)`. `Token` is resolved by identity against the
    /// real global `Array`, exactly as `crate::sparse_map` resolves
    /// `Values` — anything else, including omission, is string mode.
    #[napi(constructor)]
    pub fn new(env: Env, token: Option<Unknown>) -> Result<Self> {
        Ok(Self {
            inner: RefCell::new(CoreMap::new()),
            mode: resolve_mode(&env, token)?,
        })
    }

    /// Upstream's `TrieMap.from`, which always builds a plain, string-mode
    /// trie — `Trie.from`/`TrieMap.from` both hardcode `new Ctor()`, with no
    /// `Token` argument, regardless of what the iterable contains.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown) -> Result<Self> {
        let pairs = collect_slot_pairs(&env, iterable)?;
        let mut inner = Core::new();

        for (value, key) in pairs {
            let key = key.get(&env)?;
            let (tokens, _echo) = decode_prefix(&env, Mode::Str, Some(&key))?;
            let value = value.get(&env)?;
            let slot = retained_from_unknown(&value)?;

            if let Some(mut displaced) = inner.set(tokens, slot) {
                release_slot(&mut displaced, &env)?;
            }
        }

        Ok(Self {
            inner: RefCell::new(inner),
            mode: Mode::Str,
        })
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    /// Upstream's `root` — the raw nested structure, rebuilt fresh on every
    /// read (it is a plain object upstream too, so nothing here needs to be
    /// kept in sync with it between calls).
    #[napi(getter)]
    pub fn root(&self, env: Env) -> Result<Unknown<'_>> {
        let inner = self.inner.borrow();
        let raw = build_root(&env, inner.root(), &|value: &Value| {
            // SAFETY: produces a handle in `env`'s current scope.
            unsafe { ToNapiValue::to_napi_value(env.raw(), loan(Some(value))) }
        })?;

        // SAFETY: `raw` was just built in this call, in this scope.
        Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), raw) })
    }

    #[napi]
    pub fn clear(&self, env: Env) -> Result<()> {
        let mut inner = self.inner.borrow_mut();

        for slot in inner.values_mut() {
            release_slot(slot, &env)?;
        }

        inner.clear();

        Ok(())
    }

    /// Upstream's `set`, which returns `this` for chaining.
    #[napi]
    pub fn set<'a>(
        &self,
        this: This<'a>,
        env: Env,
        prefix: Unknown,
        value: Received,
    ) -> Result<This<'a>> {
        let (tokens, _echo) = decode_prefix(&env, self.mode, Some(&prefix))?;
        let displaced = self.inner.borrow_mut().set(tokens, value.into_slot());

        if let Some(mut displaced) = displaced {
            release_slot(&mut displaced, &env)?;
        }

        Ok(this)
    }

    /// Upstream's `update`.
    ///
    /// The callback is read, then dropped, then called with the map
    /// unlocked — see the module docs, part 3 — so a re-entrant `set`/
    /// `update`/`delete` from inside it is legal rather than a `RefCell`
    /// panic. The write that follows is an ordinary `set`, whose own
    /// presence check (not a decision cached from before the callback ran)
    /// decides the `size` bookkeeping; the two agree on every non-re-entrant
    /// call, which is everything either original test file performs.
    #[napi]
    pub fn update<'a>(
        &self,
        this: This<'a>,
        env: Env,
        prefix: Unknown,
        update_fn: Function<Loaned, Received>,
    ) -> Result<This<'a>> {
        let (tokens, _echo) = decode_prefix(&env, self.mode, Some(&prefix))?;
        let old = {
            let inner = self.inner.borrow();

            loan(inner.get(tokens.iter().cloned()))
        };
        let new_value = update_fn.call(old)?;

        let displaced = self.inner.borrow_mut().set(tokens, new_value.into_slot());

        if let Some(mut displaced) = displaced {
            release_slot(&mut displaced, &env)?;
        }

        Ok(this)
    }

    #[napi]
    pub fn get(&self, env: Env, prefix: Unknown) -> Result<Loaned> {
        let (tokens, _echo) = decode_prefix(&env, self.mode, Some(&prefix))?;

        Ok(loan(self.inner.borrow().get(tokens)))
    }

    #[napi]
    pub fn has(&self, env: Env, prefix: Unknown) -> Result<bool> {
        let (tokens, _echo) = decode_prefix(&env, self.mode, Some(&prefix))?;

        Ok(self.inner.borrow().has(tokens))
    }

    #[napi(js_name = "delete")]
    pub fn delete(&self, env: Env, prefix: Unknown) -> Result<bool> {
        let (tokens, _echo) = decode_prefix(&env, self.mode, Some(&prefix))?;
        let removed = self.inner.borrow_mut().delete(tokens);

        match removed {
            None => Ok(false),
            Some(mut value) => {
                release_slot(&mut value, &env)?;

                Ok(true)
            }
        }
    }

    /// Upstream's `find`.
    #[napi]
    pub fn find(&self, env: Env, prefix: Unknown) -> Result<Vec<AssembledEntry>> {
        let (tokens, echo) = decode_prefix(&env, self.mode, Some(&prefix))?;
        let mode = self.mode;

        let inner = self.inner.borrow();

        Ok(inner
            .find(tokens)
            .into_iter()
            .map(|(suffix, value)| AssembledEntry {
                prefix: AssembledPrefix {
                    mode,
                    echo: echo.clone(),
                    suffix,
                },
                value: loan(Some(value)),
            })
            .collect())
    }

    #[napi]
    pub fn values(
        &self,
        env: Env,
        this: Reference<JsTrieMap>,
        prefix: Option<Unknown>,
    ) -> Result<JsTrieMapValues> {
        Ok(JsTrieMapValues {
            cursor: open_walk(&env, this, self.mode, prefix)?,
        })
    }

    #[napi]
    pub fn keys(
        &self,
        env: Env,
        this: Reference<JsTrieMap>,
        prefix: Option<Unknown>,
    ) -> Result<JsTrieMapKeys> {
        Ok(JsTrieMapKeys {
            cursor: open_walk(&env, this, self.mode, prefix)?,
        })
    }

    /// Upstream's `keys` and `prefixes` are the same function
    /// (`TrieMap.prototype.keys = TrieMap.prototype.prefixes`); no test
    /// checks reference identity between the two, only behaviour, which
    /// this matches by constructing the same walk.
    #[napi]
    pub fn prefixes(
        &self,
        env: Env,
        this: Reference<JsTrieMap>,
        prefix: Option<Unknown>,
    ) -> Result<JsTrieMapKeys> {
        Ok(JsTrieMapKeys {
            cursor: open_walk(&env, this, self.mode, prefix)?,
        })
    }

    #[napi]
    pub fn entries(
        &self,
        env: Env,
        this: Reference<JsTrieMap>,
        prefix: Option<Unknown>,
    ) -> Result<JsTrieMapEntries> {
        Ok(JsTrieMapEntries {
            cursor: open_walk(&env, this, self.mode, prefix)?,
        })
    }
}

fn open_walk(
    env: &Env,
    this: Reference<JsTrieMap>,
    mode: Mode,
    prefix: Option<Unknown>,
) -> Result<WalkCursor<JsTrieMap, Value>> {
    let (tokens, echo) = decode_prefix(env, mode, prefix.as_ref())?;
    let source = this.share_with(*env, |map| Ok(&map.inner))?;

    Ok(WalkCursor::open(source, tokens, mode, echo))
}

/// `Trie.SENTINEL` / `TrieMap.SENTINEL` — `String.fromCharCode(0)`, upstream's
/// reserved marker key.
///
/// Exposed because `test/trie.js` and `test/trie-map.js` both read it
/// directly (`var SENTINEL = Trie.SENTINEL;`) to build the `root` shapes they
/// assert against. Installed onto the class the same way `crate::heap`
/// installs its statics: a load-time addon property, not a shim concern (a
/// shim that added it would mean `require('@port/addon').Trie` was
/// incomplete without the test harness).
pub(crate) fn install_trie_statics(exports: &Object, env: &Env) -> Result<()> {
    let sentinel = env.create_string_utf16([0u16])?;

    for class in ["Trie", "TrieMap"] {
        let mut constructor: Object = exports
            .get(class)?
            .ok_or_else(|| missing_trie_class(class))?;

        constructor.set_named_property("SENTINEL", sentinel)?;
    }

    Ok(())
}

fn missing_trie_class(class: &str) -> Error {
    Error::new(
        Status::GenericFailure,
        format!(
            "cannot install `{class}.SENTINEL`: exports.{class} does not exist. The \
             installer and the addon's exports have drifted apart."
        ),
    )
}

impl ObjectFinalize for JsTrieMap {
    fn finalize(self, env: Env) -> Result<()> {
        for slot in self.inner.borrow_mut().values_mut() {
            release_slot(slot, &env)?;
        }

        Ok(())
    }
}

#[napi(iterator, js_name = "TrieMapValues")]
pub struct JsTrieMapValues {
    cursor: WalkCursor<JsTrieMap, Value>,
}

impl Generator for JsTrieMapValues {
    type Yield = Loaned;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<Loaned> {
        self.cursor
            .step(|value| loan(Some(value)))
            .map(|(_suffix, loaned)| loaned)
    }

    fn complete(&mut self, _value: Option<()>) -> Option<Loaned> {
        None
    }
}

#[napi(iterator, js_name = "TrieMapKeys")]
pub struct JsTrieMapKeys {
    cursor: WalkCursor<JsTrieMap, Value>,
}

impl Generator for JsTrieMapKeys {
    type Yield = AssembledPrefix;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<AssembledPrefix> {
        let suffix = self.cursor.suffix_only()?;

        Some(self.cursor.assembled(suffix))
    }

    fn complete(&mut self, _value: Option<()>) -> Option<AssembledPrefix> {
        None
    }
}

#[napi(iterator, js_name = "TrieMapEntries")]
pub struct JsTrieMapEntries {
    cursor: WalkCursor<JsTrieMap, Value>,
}

impl Generator for JsTrieMapEntries {
    type Yield = AssembledEntry;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<AssembledEntry> {
        let (suffix, loaned) = self.cursor.step(|value| loan(Some(value)))?;
        let prefix = self.cursor.assembled(suffix);

        Some(AssembledEntry {
            prefix,
            value: loaned,
        })
    }

    fn complete(&mut self, _value: Option<()>) -> Option<AssembledEntry> {
        None
    }
}

//! JS bridge for [`mnemonist_core::structures::inverted_index`].
//!
//! Read the core module's docs first. Three things live here that core
//! deliberately does not know about:
//!
//! 1. **The two tokenizers are JS callbacks**, resolved from the
//!    constructor's `descriptor` argument exactly as upstream's own
//!    truthiness-based fallback does (not merely "is it `undefined`" — see
//!    [`resolve_tokenizer`]), and called on every `add`/`get`.
//! 2. **`Array.isArray(tokens)`** on a tokenizer's return value is upstream's
//!    own guard, reproduced verbatim at the one place it can run: after the
//!    callback returns, in [`tokens_from_unknown`].
//! 3. **Tokens are [`crate::js_key::JsKey`]**, not a bespoke type: upstream's
//!    `mapping` is a real `Map`, and tokens are its keys — SameValueZero,
//!    the same reasoning `default_map.rs` documents for T3, reused rather
//!    than reinvented. Documents are [`JsSlot`]: arbitrary values, stored and
//!    handed back with their identity intact (`OBJECT_DOCS[0]` must come
//!    back as the SAME object, not a copy — the original suite's own
//!    `documents()` block asserts exactly that with `deepStrictEqual`, which
//!    for objects still requires the same shape but not the same reference;
//!    `JsSlot` gives both for free either way).
//!
//! # BUG-INVERTED-INDEX-1 at the bridge: `forEach` never calls back, by construction
//!
//! `mnemonist_core::structures::inverted_index::InvertedIndex::for_each`
//! hands back a cursor frozen at length zero. [`JsInvertedIndex::for_each`]
//! below drives it exactly like `values`/`entries` drive `documents()` — the
//! same walk primitive, just handed a cursor that can never step — so the
//! callback loop here runs zero times for the same structural reason core's
//! own docs give, not because of a special case bolted on at the boundary.

use std::cell::RefCell;

use mnemonist_core::structures::inverted_index::{
    DocumentsCursor, InvertedIndex as CoreIndex, TokensCursor,
};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;

use crate::foreach;
use crate::js_key::JsKey;
use crate::js_slot::JsSlot;

/// The index as core sees it: arbitrary JS documents, `Map`-keyed tokens.
type Core = CoreIndex<JsSlot, JsKey>;

/// A tokenizer function, called with the document or query being indexed or
/// queried. [`Tokens`] is the return type: no lifetime, because a
/// `FunctionRef` is called back long after the `Env` that constructed it is
/// gone, same reasoning as `default_map.rs`'s `Factory` and `bk_tree.rs`'s
/// `Distance`.
type Tokenizer = FunctionRef<FnArgs<(JsSlot,)>, Tokens>;

/// `!!value`, spelled out because Rust has no truthiness. Only the empty
/// string, `0`/`-0`/`NaN`, `false`, `0n`, `null` and `undefined` are falsy;
/// every object (including a bare `{}`) is truthy. Upstream's own fallback —
/// `if (!this.documentTokenizer) this.documentTokenizer = identity;` — is
/// this test, not `typeof … === 'undefined'`, so a descriptor of `0`,
/// `null`, `false` or `''` ALSO falls back to `identity`, not just an
/// omitted one. Duplicated rather than shared: `crate::foreach`,
/// `crate::fuzzy_map`, `crate::fuzzy_multi_map` and `crate::comparators` each
/// keep a private copy of this exact helper already (grepped for one before
/// writing this; none is `pub(crate)`), so this is the fifth, not a new
/// pattern.
fn is_truthy(env: &Env, value: &Unknown) -> Result<bool> {
    let mut result = false;
    let mut coerced = std::ptr::null_mut();

    // SAFETY: a live handle from `env`.
    napi::check_status!(
        unsafe { sys::napi_coerce_to_bool(env.raw(), value.raw(), &mut coerced) },
        "napi_coerce_to_bool"
    )?;
    // SAFETY: `coerced` is the boolean N-API just produced.
    napi::check_status!(
        unsafe { sys::napi_get_value_bool(env.raw(), coerced, &mut result) },
        "napi_get_value_bool"
    )?;

    Ok(result)
}

/// A tokenizer's return value, `Array.isArray`-checked and converted to
/// `Map`-comparable keys — upstream's
/// `if (!Array.isArray(tokens)) throw new Error('… should return an array of tokens.');`
/// plus the per-element read, in one place, so both the real-callback path
/// and the `identity` fallback ([`resolve_tokenizer`]) apply the identical
/// rule.
struct Tokens(Vec<JsKey>);

impl FromNapiValue for Tokens {
    unsafe fn from_napi_value(env: sys::napi_env, value: sys::napi_value) -> Result<Self> {
        let unknown = unsafe { Unknown::from_napi_value(env, value)? };
        let env = Env::from_raw(env);

        tokens_from_unknown(&env, &unknown).map(Tokens)
    }
}

/// The message a caller sees for either tokenizer, named per upstream's own
/// two (near-identical) messages.
fn not_a_function_message(which: &str) -> String {
    format!("mnemonist/InvertedIndex.constructor: {which} tokenizer is not a function.")
}

const NOT_AN_ARRAY: &str =
    "mnemonist/InvertedIndex.add: tokenizer function should return an array of tokens.";

/// `Array.isArray(tokens)` then one `JsKey::from_unknown` per element —
/// upstream's own guard and conversion, shared by a real callback's return
/// value and by the `identity` fallback's "return value" (which is simply
/// the input handed back, per upstream's `function identity(x) { return x; }`).
fn tokens_from_unknown(env: &Env, value: &Unknown) -> Result<Vec<JsKey>> {
    if !foreach::is_array(env, value)? {
        return Err(Error::new(Status::GenericFailure, NOT_AN_ARRAY));
    }

    // SAFETY: `is_array` has just confirmed this.
    let array = unsafe { value.cast::<Array>()? };
    let mut tokens = Vec::with_capacity(array.len() as usize);

    for index in 0..array.len() {
        let element: Unknown = array
            .get(index)?
            .expect("an index below Array.length always resolves, to `undefined` at worst");

        tokens.push(JsKey::from_unknown(&element)?);
    }

    Ok(tokens)
}

/// Resolve one half of the constructor's `descriptor` — upstream's
/// truthiness fallback to `identity`, then the `typeof … !== 'function'`
/// guard, in that order. `candidate` is `None` for an entirely omitted
/// `descriptor` (`this.documentTokenizer = undefined` upstream, itself
/// falsy).
fn resolve_tokenizer(
    env: &Env,
    candidate: Option<Unknown>,
    which: &str,
) -> Result<Option<Tokenizer>> {
    let truthy = match &candidate {
        Some(value) => is_truthy(env, value)?,
        None => false,
    };

    if !truthy {
        // Upstream's `identity` fallback. Modelled as `None` rather than a
        // materialised JS closure — see `JsInvertedIndex::tokenize`'s docs —
        // so there is no real function to validate or to call.
        return Ok(None);
    }

    let value = candidate.expect("truthy implies Some");

    if value.get_type()? != ValueType::Function {
        return Err(Error::new(
            Status::InvalidArg,
            not_a_function_message(which),
        ));
    }

    // SAFETY: `get_type` has just reported `Function`.
    let function = unsafe { value.cast::<Function<FnArgs<(JsSlot,)>, Tokens>>()? };

    Ok(Some(function.create_ref()?))
}

/// A document store plus a token → posting-list index.
#[napi(js_name = "InvertedIndex")]
pub struct JsInvertedIndex {
    inner: RefCell<Core>,
    document_tokenizer: Option<Tokenizer>,
    query_tokenizer: Option<Tokenizer>,
}

#[napi]
impl JsInvertedIndex {
    /// `new InvertedIndex(descriptor)` — `descriptor` is an `[docTokenizer,
    /// queryTokenizer]` pair, a single function used for both, or omitted
    /// (both default to `identity`).
    #[napi(constructor)]
    pub fn new(env: Env, descriptor: Option<Unknown>) -> Result<Self> {
        let (doc_arg, query_arg) = match &descriptor {
            Some(value) if foreach::is_array(&env, value)? => {
                // SAFETY: just confirmed.
                let array = unsafe { value.cast::<Array>()? };
                let first: Unknown = array.get(0)?.unwrap_or(undefined_of(&env)?);
                let second: Unknown = array.get(1)?.unwrap_or(undefined_of(&env)?);

                (Some(first), Some(second))
            }
            // Not an array: BOTH tokenizers resolve from the SAME single
            // value, exactly as upstream's `this.documentTokenizer =
            // this.queryTokenizer = descriptor;` does.
            Some(value) => (Some(*value), Some(*value)),
            None => (None, None),
        };

        let document_tokenizer = resolve_tokenizer(&env, doc_arg, "document")?;
        let query_tokenizer = resolve_tokenizer(&env, query_arg, "query")?;

        Ok(Self {
            inner: RefCell::new(Core::new()),
            document_tokenizer,
            query_tokenizer,
        })
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    #[napi(getter)]
    pub fn dimension(&self) -> u32 {
        self.inner.borrow().dimension() as u32
    }

    /// Upstream's `add`. Tokenizes with the DOCUMENT tokenizer, then
    /// indexes — the borrow is released before the (possibly re-entrant,
    /// PORTBUG-1-shaped) tokenizer call and re-taken to write, same discipline as
    /// every other bridge past `default-map`.
    #[napi]
    pub fn add<'a>(&self, this: This<'a>, env: Env, doc: Unknown) -> Result<This<'a>> {
        let slot = JsSlot::new(&env, &doc)?;
        let tokens = self.tokenize(&env, &self.document_tokenizer, slot.clone())?;

        self.inner.borrow_mut().add(slot, tokens);

        Ok(this)
    }

    /// Upstream's `get`: an AND query, tokenized with the QUERY tokenizer.
    #[napi]
    pub fn get(&self, env: Env, query: Unknown) -> Result<Vec<JsSlot>> {
        let slot = JsSlot::new(&env, &query)?;
        let tokens = self.tokenize(&env, &self.query_tokenizer, slot)?;

        Ok(self.inner.borrow().get(&tokens))
    }

    /// Upstream's `forEach`. See the module docs and BUG-INVERTED-INDEX-1: this
    /// walks `InvertedIndex::for_each`'s cursor, which core freezes at
    /// length zero unconditionally, so the loop body below runs zero times
    /// on every call, regardless of `size`.
    #[allow(clippy::type_complexity)]
    #[napi(js_name = "forEach")]
    pub fn for_each(
        &self,
        this: This,
        callback: Function<FnArgs<(JsSlot, u32, Object)>, Unknown>,
        scope: Option<Unknown>,
    ) -> Result<()> {
        let index_object = this.object;
        // Self-contained: captures its own `Rc` clone of `items`, so it needs
        // no borrow of `self.inner` to step -- see the core module's docs on
        // why `clear()` must not be able to invalidate it.
        let mut cursor: DocumentsCursor<JsSlot> = self.inner.borrow().for_each();
        let mut position = 0u32;

        while let Some(doc) = cursor.step() {
            let arguments = FnArgs::from((doc, position, index_object));

            match &scope {
                Some(scope) => callback.apply(*scope, arguments)?,
                None => callback.apply(this, arguments)?,
            };

            position += 1;
        }

        Ok(())
    }

    /// A fresh cursor over the stored documents — upstream's `documents()`,
    /// and its `Symbol.iterator`.
    ///
    /// No `SharedReference` to `self.inner` is needed: `DocumentsCursor`
    /// captures its own `Rc` clone of `items` at open time (see the core
    /// module's docs), so the cursor is independent of `JsInvertedIndex`
    /// once created -- exactly as upstream's own closure, which captures the
    /// array object rather than `this`, is.
    #[napi]
    pub fn documents(&self) -> JsInvertedIndexDocuments {
        JsInvertedIndexDocuments {
            cursor: self.inner.borrow().documents(),
        }
    }

    /// A fresh cursor over the distinct tokens seen, first-seen order —
    /// upstream's `tokens()`, `this.mapping.keys()`. Same independence from
    /// `self.inner` as [`JsInvertedIndex::documents`], for the identical
    /// reason.
    #[napi]
    pub fn tokens(&self) -> JsInvertedIndexTokens {
        JsInvertedIndexTokens {
            cursor: self.inner.borrow().tokens(),
        }
    }

    /// `InvertedIndex.from(iterable, descriptor)`.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown, descriptor: Option<Unknown>) -> Result<Self> {
        let built = Self::new(env, descriptor)?;
        let docs = foreach::collect(&env, iterable)?;

        for doc_slot in docs {
            let tokens = built.tokenize(&env, &built.document_tokenizer, doc_slot.clone())?;

            built.inner.borrow_mut().add(doc_slot, tokens);
        }

        Ok(built)
    }

    /// Call `tokenizer` on `input` (or, for `None`, apply upstream's
    /// `identity` directly — see [`resolve_tokenizer`]) and validate the
    /// result exactly as `Tokens::from_napi_value` does for a real callback,
    /// so the two paths cannot drift apart.
    fn tokenize(
        &self,
        env: &Env,
        tokenizer: &Option<Tokenizer>,
        input: JsSlot,
    ) -> Result<Vec<JsKey>> {
        match tokenizer {
            Some(function_ref) => {
                let callable = function_ref.borrow_back(env)?;
                let Tokens(tokens) = callable.call((input,).into())?;

                Ok(tokens)
            }
            None => {
                let value = input.get(env)?;

                tokens_from_unknown(env, &value)
            }
        }
    }
}

/// `env.get_undefined()`, wrapped as `Unknown` — what a missing array slot
/// (`descriptor[1]` when `descriptor` has one element) reads as.
fn undefined_of(env: &Env) -> Result<Unknown<'_>> {
    let value: () = ();

    // SAFETY: `()` converts to `undefined` unconditionally.
    let raw = unsafe { ToNapiValue::to_napi_value(env.raw(), value)? };

    Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), raw) })
}

/// The cursor `InvertedIndex.prototype.documents()` hands out, and its
/// `Symbol.iterator`. Self-contained — see [`JsInvertedIndex::documents`].
#[napi(iterator, js_name = "InvertedIndexDocuments")]
pub struct JsInvertedIndexDocuments {
    cursor: DocumentsCursor<JsSlot>,
}

impl Generator for JsInvertedIndexDocuments {
    type Yield = JsSlot;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<JsSlot> {
        self.cursor.step()
    }

    fn complete(&mut self, _value: Option<()>) -> Option<JsSlot> {
        None
    }
}

/// The cursor `InvertedIndex.prototype.tokens()` hands out —
/// `this.mapping.keys()`, a real `Map` iterator. Self-contained — see
/// [`JsInvertedIndex::tokens`].
#[napi(iterator, js_name = "InvertedIndexTokens")]
pub struct JsInvertedIndexTokens {
    cursor: TokensCursor<JsKey>,
}

impl Generator for JsInvertedIndexTokens {
    type Yield = JsKey;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<JsKey> {
        self.cursor.step()
    }

    fn complete(&mut self, _value: Option<()>) -> Option<JsKey> {
        None
    }
}

//! JS bridge for [`mnemonist_core::structures::trie`].
//!
//! Thin, reusing everything `crate::trie_map` already built: token decoding,
//! prefix assembly and the `Mode`/`Array`-identity resolution are identical
//! between the two upstream files, because `trie.js` is upstream's own
//! `TrieMap.prototype` copy-and-delete (see that module's docs). What is not
//! shared is the walk cursor: [`mnemonist_core::structures::trie::Trie`]
//! deliberately does not expose its inner
//! [`mnemonist_core::structures::trie_map::TrieMap`] (there is nothing for a
//! Rust caller to *do* with a bare boolean sentinel), so
//! `crate::trie_map::WalkCursor` — built to project a **value** out from
//! behind a live borrow — does not fit here. [`WalkCursor`] below is the same
//! shape with the value half removed.

use std::cell::RefCell;

use mnemonist_core::structures::trie::Trie as CoreTrie;
use mnemonist_core::structures::trie_map::Walk as CoreWalk;
use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::trie_map::{build_root, decode_prefix, resolve_mode, AssembledPrefix, Mode, Token};

/// As `crate::trie_map::WalkCursor`, minus the value projection: a `Trie` has
/// nothing to yield but the suffix itself.
struct WalkCursor<Owner: 'static> {
    source: SharedReference<Owner, &'static RefCell<CoreTrie<Token>>>,
    walk: CoreWalk<Token>,
    mode: Mode,
    echo: crate::trie_map::PrefixEcho,
}

impl<Owner: 'static> WalkCursor<Owner> {
    fn open(
        source: SharedReference<Owner, &'static RefCell<CoreTrie<Token>>>,
        tokens: Vec<Token>,
        mode: Mode,
        echo: crate::trie_map::PrefixEcho,
    ) -> Self {
        let walk = source.borrow().walk(tokens);

        Self {
            source,
            walk,
            mode,
            echo,
        }
    }

    fn step(&mut self) -> Option<Vec<Token>> {
        let inner = self.source.borrow();

        inner.step(&mut self.walk)
    }

    fn assembled(&self, suffix: Vec<Token>) -> AssembledPrefix {
        AssembledPrefix::new(self.mode, self.echo.clone(), suffix)
    }
}

/// Upstream's `Trie`.
#[napi(js_name = "Trie")]
pub struct JsTrie {
    inner: RefCell<CoreTrie<Token>>,
    mode: Mode,
}

#[napi]
impl JsTrie {
    /// `new Trie(Token)`, resolved exactly as `JsTrieMap::new` resolves it.
    #[napi(constructor)]
    pub fn new(env: Env, token: Option<Unknown>) -> Result<Self> {
        Ok(Self {
            inner: RefCell::new(CoreTrie::new()),
            mode: resolve_mode(&env, token)?,
        })
    }

    /// Upstream's `Trie.from` — always a plain, string-mode trie; see
    /// `crate::trie_map::JsTrieMap::from`'s docs for why.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown) -> Result<Self> {
        let values = crate::foreach::collect(&env, iterable)?;
        let mut inner = CoreTrie::new();

        for slot in values {
            let value = slot.get(&env)?;
            let (tokens, _echo) = decode_prefix(&env, Mode::Str, Some(&value))?;

            inner.add(tokens);
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

    /// Upstream's `root`, rebuilt fresh on every read — a `Trie` node's own
    /// value is always the bare `true` upstream's `add` stores.
    #[napi(getter)]
    pub fn root(&self, env: Env) -> Result<Unknown<'_>> {
        let inner = self.inner.borrow();
        let raw = build_root(&env, inner.root(), &|value: &bool| {
            // SAFETY: produces a handle in `env`'s current scope.
            unsafe { ToNapiValue::to_napi_value(env.raw(), *value) }
        })?;

        // SAFETY: `raw` was just built in this call, in this scope.
        Ok(unsafe { Unknown::from_raw_unchecked(env.raw(), raw) })
    }

    #[napi]
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// Upstream's `add`, which returns `this` for chaining.
    #[napi]
    pub fn add<'a>(&self, this: This<'a>, env: Env, prefix: Unknown) -> Result<This<'a>> {
        let (tokens, _echo) = decode_prefix(&env, self.mode, Some(&prefix))?;

        self.inner.borrow_mut().add(tokens);

        Ok(this)
    }

    #[napi]
    pub fn has(&self, env: Env, prefix: Unknown) -> Result<bool> {
        let (tokens, _echo) = decode_prefix(&env, self.mode, Some(&prefix))?;

        Ok(self.inner.borrow().has(tokens))
    }

    #[napi(js_name = "delete")]
    pub fn delete(&self, env: Env, prefix: Unknown) -> Result<bool> {
        let (tokens, _echo) = decode_prefix(&env, self.mode, Some(&prefix))?;

        Ok(self.inner.borrow_mut().delete(tokens))
    }

    /// Inherited unmodified from `TrieMap` upstream (`trie.js` deletes
    /// `set`/`get`/`values`/`entries` only) — see the core module's docs.
    /// No test in `test/trie.js` calls it, and `trie.d.ts` does not declare
    /// it, but it is reachable, so it is bridged too.
    #[napi]
    pub fn update<'a>(
        &self,
        this: This<'a>,
        env: Env,
        prefix: Unknown,
        update_fn: Function<bool, bool>,
    ) -> Result<This<'a>> {
        let (tokens, _echo) = decode_prefix(&env, self.mode, Some(&prefix))?;
        let old = self.inner.borrow().has(tokens.iter().cloned());
        let new_value = update_fn.call(old)?;

        self.inner.borrow_mut().update(tokens, |_old| new_value);

        Ok(this)
    }

    /// Upstream's own `Trie.prototype.find` override.
    #[napi]
    pub fn find(&self, env: Env, prefix: Unknown) -> Result<Vec<AssembledPrefix>> {
        let (tokens, echo) = decode_prefix(&env, self.mode, Some(&prefix))?;
        let mode = self.mode;

        Ok(self
            .inner
            .borrow()
            .find(tokens)
            .into_iter()
            .map(|suffix| AssembledPrefix::new(mode, echo.clone(), suffix))
            .collect())
    }

    /// Upstream's `prefixes`/`keys` — the same function, aliased. `values`
    /// and `entries` are two of the four methods `trie.js` deletes: a `Trie`
    /// has no value to project.
    #[napi]
    pub fn keys(
        &self,
        env: Env,
        this: Reference<JsTrie>,
        prefix: Option<Unknown>,
    ) -> Result<JsTrieKeys> {
        Ok(JsTrieKeys {
            cursor: open_walk(&env, this, self.mode, prefix)?,
        })
    }

    #[napi]
    pub fn prefixes(
        &self,
        env: Env,
        this: Reference<JsTrie>,
        prefix: Option<Unknown>,
    ) -> Result<JsTrieKeys> {
        Ok(JsTrieKeys {
            cursor: open_walk(&env, this, self.mode, prefix)?,
        })
    }
}

fn open_walk(
    env: &Env,
    this: Reference<JsTrie>,
    mode: Mode,
    prefix: Option<Unknown>,
) -> Result<WalkCursor<JsTrie>> {
    let (tokens, echo) = decode_prefix(env, mode, prefix.as_ref())?;
    let source = this.share_with(*env, |trie| Ok(&trie.inner))?;

    Ok(WalkCursor::open(source, tokens, mode, echo))
}

#[napi(iterator, js_name = "TrieKeys")]
pub struct JsTrieKeys {
    cursor: WalkCursor<JsTrie>,
}

impl Generator for JsTrieKeys {
    type Yield = AssembledPrefix;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<()>) -> Option<AssembledPrefix> {
        let suffix = self.cursor.step()?;

        Some(self.cursor.assembled(suffix))
    }

    fn complete(&mut self, _value: Option<()>) -> Option<AssembledPrefix> {
        None
    }
}

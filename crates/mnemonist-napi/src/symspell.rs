//! JS bridge for [`mnemonist_core::structures::symspell`].
//!
//! Thin translation only — `add`/`search` take and return plain strings and
//! numbers, so there is no `JsKey`/`JsSlot` machinery here at all, unlike
//! most of this crate's other bridges. The one real job is resolving the
//! `{maxDistance, verbosity}` options object's defaults (upstream's
//! `DEFAULT_MAX_DISTANCE = 2`, `DEFAULT_VERBOSITY = 2`), which core does not
//! see — it only ever receives the two already-resolved numbers.

use mnemonist_core::structures::symspell::{Error as CoreError, SymSpell as CoreSymSpell};
use napi::bindgen_prelude::*;
use napi_derive::napi;

const DEFAULT_MAX_DISTANCE: f64 = 2.0;
const DEFAULT_VERBOSITY: f64 = 2.0;

fn raise(error: CoreError) -> Error {
    Error::new(Status::InvalidArg, error.to_string())
}

/// One search hit — upstream's `{term, distance, count}`.
#[napi(object)]
pub struct JsSuggestion {
    pub term: String,
    pub distance: f64,
    pub count: u32,
}

impl From<mnemonist_core::structures::symspell::Suggestion> for JsSuggestion {
    fn from(suggestion: mnemonist_core::structures::symspell::Suggestion) -> Self {
        Self {
            term: suggestion.term,
            distance: suggestion.distance as f64,
            count: suggestion.count as u32,
        }
    }
}

#[napi(js_name = "SymSpell")]
pub struct JsSymSpell {
    inner: CoreSymSpell,
}

#[napi]
impl JsSymSpell {
    /// `new SymSpell(options)`.
    #[napi(constructor)]
    pub fn new(options: Option<Object>) -> Result<Self> {
        let (max_distance, verbosity) = resolve_options(options)?;

        let inner = CoreSymSpell::new(max_distance, verbosity).map_err(raise)?;

        Ok(Self { inner })
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.size() as u32
    }

    #[napi(getter, js_name = "maxDistance")]
    pub fn max_distance(&self) -> f64 {
        self.inner.max_distance()
    }

    #[napi(getter)]
    pub fn verbosity(&self) -> u32 {
        self.inner.verbosity() as u32
    }

    #[napi]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Upstream's `add`, which returns `this` for chaining.
    #[napi]
    pub fn add<'a>(&mut self, this: This<'a>, word: String) -> This<'a> {
        self.inner.add(&word);

        this
    }

    #[napi]
    pub fn search(&self, input: String) -> Vec<JsSuggestion> {
        self.inner
            .search(&input)
            .into_iter()
            .map(JsSuggestion::from)
            .collect()
    }

    /// `SymSpell.from(iterable, options)`.
    #[napi(factory)]
    pub fn from(env: Env, iterable: Unknown, options: Option<Object>) -> Result<Self> {
        let (max_distance, verbosity) = resolve_options(options)?;
        let mut inner = CoreSymSpell::new(max_distance, verbosity).map_err(raise)?;

        for slot in crate::foreach::collect(&env, iterable)? {
            let value = slot.get(&env)?;
            let word = String::from_unknown(value)?;

            inner.add(&word);
        }

        Ok(Self { inner })
    }
}

/// Resolves `{maxDistance, verbosity}`, upstream's own defaults applied when
/// a field (or the whole object) is omitted.
fn resolve_options(options: Option<Object>) -> Result<(f64, u8)> {
    let Some(options) = options else {
        return Ok((DEFAULT_MAX_DISTANCE, DEFAULT_VERBOSITY as u8));
    };

    let max_distance = options
        .get::<f64>("maxDistance")?
        .unwrap_or(DEFAULT_MAX_DISTANCE);
    let verbosity = options
        .get::<f64>("verbosity")?
        .unwrap_or(DEFAULT_VERBOSITY);

    // Upstream's own membership check (`VERBOSITY.has(this.verbosity)`) is
    // over the *raw* JS number, so a non-integral verbosity (`1.5`) fails it
    // exactly as `45` does. `u8` cannot represent that distinction directly,
    // so it is made here, before truncating.
    if verbosity.fract() != 0.0 || !(0.0..=2.0).contains(&verbosity) {
        return Err(raise(CoreError::InvalidVerbosity));
    }

    Ok((max_distance, verbosity as u8))
}

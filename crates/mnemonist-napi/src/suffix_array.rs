//! JS bridge for [`mnemonist_core::structures::suffix_array`].
//!
//! Thin translation only: every behavioural decision — including both of the
//! defects this unit reproduces, B-90 and B-91 — lives in the core crate. What
//! this layer carries is the shape of the two constructors' arguments and of
//! the two "sequence-shaped" return values.
//!
//! 1. **The constructor argument decides the alphabet.** Upstream branches on
//!    `typeof string !== 'string'`: a string is indexed with `charCodeAt`, and
//!    anything else is treated as an array whose members become property names
//!    of an alphabet object. So a string becomes [`Sequence::Text`] (UTF-16
//!    code units, which is exactly what `charCodeAt` yields) and an array
//!    becomes [`Sequence::Tokens`] with each member put through `String(x)`,
//!    which is what using it as a property key does.
//! 2. **`#.string` / `#.text` / `#.longestCommonSubsequence` return a union.**
//!    A string in the text case, an array of strings in the token case — the
//!    same two shapes upstream returns, because both are `this.text.slice(...)`
//!    of whichever kind was stored.
//! 3. **`GeneralizedSuffixArray` is exported alongside `SuffixArray`, and the
//!    shim aliases it.** Upstream's last line is
//!    `SuffixArray.GeneralizedSuffixArray = GeneralizedSuffixArray`, a CJS
//!    *namespacing* statement rather than a behaviour, and the addon exports
//!    both classes so neither is missing. Doing the alias in the addon would
//!    mean editing the one `#[napi(module_exports)]` hook in `crate::cursor`,
//!    which several agents are editing concurrently and where a merge conflict
//!    has already landed inside a function tail three times. That trade is
//!    recorded in `docs/modules/suffix-array.md` rather than left implicit.
//! 4. **The core structure is held in a [`RefCell`].** Nothing here mutates and
//!    no method takes a callback, so the aliasing hazard behind B-31 is not
//!    currently reachable — but `&self` on a `Freeze` type is `noalias
//!    readonly` to LLVM whether or not today's methods exercise it, and the
//!    cost of the cell is nil. Written this way from the start so that adding a
//!    method later cannot silently reintroduce the bug.
//! 5. **`inspect` is not ported.** A Node display convenience with no upstream
//!    assertion and no Rust equivalent.

use std::cell::RefCell;

use mnemonist_core::structures::suffix_array::{
    GeneralizedSuffixArray as CoreGeneralized, Sequence, SuffixArray as CoreSuffixArray,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// A sequence as JavaScript sees it: a string, or an array of stringified
/// tokens.
type JsSequence = Either<String, Vec<String>>;

/// `String(value)` — the coercion that turns a token into the property name
/// upstream's alphabet object is keyed by.
fn stringify(env: &Env, value: &Unknown) -> Result<String> {
    let global = env.get_global()?;
    let string_ctor: Function<'_, Unknown, String> = global.get_named_property("String")?;

    string_ctor.call(*value)
}

/// `Array.isArray(value)`.
fn is_array(env: &Env, value: &Unknown) -> Result<bool> {
    let global = env.get_global()?;
    let array: Object = global.get_named_property_unchecked("Array")?;
    let is_array: Function<'_, Unknown, bool> = array.get_named_property("isArray")?;

    is_array.call(*value)
}

/// One constructor argument, as the core's [`Sequence`].
///
/// A string is a string. Everything else is array-like, read through `length`
/// and numeric indices exactly as upstream's `convert` does — upstream never
/// checks `Array.isArray`, so neither does this, and a non-array-like argument
/// fails the same way it does there: upstream's `new Array(undefined + 0)` is a
/// `RangeError`, and so is the error raised here.
fn to_sequence(env: &Env, value: Unknown) -> Result<Sequence> {
    if value.get_type()? == ValueType::String {
        return Ok(Sequence::Text(
            String::from_unknown(value)?.encode_utf16().collect(),
        ));
    }

    let object = Object::from_unknown(value)?;
    let length: Option<f64> = object.get("length")?;
    let length = match length {
        Some(length) if length >= 0.0 && length.fract() == 0.0 => length as u32,
        _ => {
            return Err(Error::new(
                Status::InvalidArg,
                "Invalid array length".to_owned(),
            ))
        }
    };
    let mut tokens = Vec::with_capacity(length as usize);

    for index in 0..length {
        let element: Unknown = object.get_element(index)?;

        tokens.push(stringify(env, &element)?);
    }

    Ok(Sequence::Tokens(tokens))
}

/// A [`Sequence`] on its way back to JavaScript.
fn from_sequence(sequence: &Sequence) -> JsSequence {
    match sequence {
        Sequence::Text(units) => Either::A(String::from_utf16_lossy(units)),
        Sequence::Tokens(tokens) => Either::B(tokens.clone()),
    }
}

/// Positions are `usize` in the core and `u32` on the wire; a sequence long
/// enough to overflow cannot be constructed in a JavaScript string.
fn positions(array: &[usize]) -> Vec<u32> {
    array.iter().map(|&position| position as u32).collect()
}

/// Suffix array over one sequence.
#[napi(js_name = "SuffixArray")]
pub struct JsSuffixArray {
    inner: RefCell<CoreSuffixArray>,
}

#[napi]
impl JsSuffixArray {
    #[napi(constructor)]
    pub fn new(env: Env, sequence: Unknown) -> Result<Self> {
        Ok(Self {
            inner: RefCell::new(CoreSuffixArray::new(to_sequence(&env, sequence)?)),
        })
    }

    /// `#.hasArbitrarySequence` — `typeof string !== 'string'`.
    #[napi(getter, js_name = "hasArbitrarySequence")]
    pub fn has_arbitrary_sequence(&self) -> bool {
        self.inner.borrow().has_arbitrary_sequence()
    }

    /// `#.string` — the sequence, in the shape it was given.
    #[napi(getter)]
    pub fn string(&self) -> JsSequence {
        from_sequence(self.inner.borrow().sequence())
    }

    /// `#.length` — the *sequence's* length, which is what upstream stores.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.borrow().len() as u32
    }

    /// `#.array` — suffix start positions.
    #[napi(getter)]
    pub fn array(&self) -> Vec<u32> {
        positions(self.inner.borrow().array())
    }

    /// `#.toJSON`, which upstream defines as `this.array`.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Vec<u32> {
        positions(self.inner.borrow().array())
    }

    /// `#.toString`, which upstream defines as `this.array.join(',')`.
    #[napi(js_name = "toString")]
    pub fn to_js_string(&self) -> String {
        self.inner.borrow().to_joined_string()
    }
}

/// Suffix array over several sequences spliced together with `''`.
#[napi(js_name = "GeneralizedSuffixArray")]
pub struct JsGeneralizedSuffixArray {
    inner: RefCell<CoreGeneralized>,
}

#[napi]
impl JsGeneralizedSuffixArray {
    #[napi(constructor)]
    pub fn new(env: Env, sequences: Unknown) -> Result<Self> {
        let object = Object::from_unknown(sequences)?;
        let length: Option<f64> = object.get("length")?;
        let length = match length {
            Some(length) if length >= 0.0 && length.fract() == 0.0 => length as u32,
            _ => {
                return Err(Error::new(
                    Status::InvalidArg,
                    "Invalid array length".to_owned(),
                ))
            }
        };
        let mut members = Vec::with_capacity(length as usize);

        for index in 0..length {
            let element: Unknown = object.get_element(index)?;

            // Upstream decides text-vs-tokens from `strings[0]` alone and then
            // applies `push.apply` or `join` to every member; a member of the
            // other kind is silently spread into its characters. The core
            // rejects a mixed list instead (documented divergence), and this
            // classification is what feeds it.
            members.push(if is_array(&env, &element)? {
                to_sequence(&env, element)?
            } else {
                Sequence::Text(stringify(&env, &element)?.encode_utf16().collect())
            });
        }

        CoreGeneralized::new(&members)
            .map(|inner| Self {
                inner: RefCell::new(inner),
            })
            .map_err(|message| Error::new(Status::InvalidArg, message))
    }

    #[napi(getter, js_name = "hasArbitrarySequence")]
    pub fn has_arbitrary_sequence(&self) -> bool {
        self.inner.borrow().has_arbitrary_sequence()
    }

    /// `#.size` — how many sequences were spliced together.
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.borrow().size() as u32
    }

    /// `#.text` — the spliced sequence, separators included.
    #[napi(getter)]
    pub fn text(&self) -> JsSequence {
        from_sequence(self.inner.borrow().text())
    }

    /// `#.firstLength` — the length of `strings[0]`, the boundary
    /// `longestCommonSubsequence` compares positions against.
    #[napi(getter, js_name = "firstLength")]
    pub fn first_length(&self) -> u32 {
        self.inner.borrow().first_length() as u32
    }

    /// `#.length` — the length of the spliced text.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.borrow().len() as u32
    }

    /// `#.array` — suffix start positions in the spliced text.
    #[napi(getter)]
    pub fn array(&self) -> Vec<u32> {
        positions(self.inner.borrow().array())
    }

    /// `#.longestCommonSubsequence` — the longest common *substring* of the
    /// first sequence and any other, despite the name.
    #[napi(js_name = "longestCommonSubsequence")]
    pub fn longest_common_subsequence(&self) -> JsSequence {
        from_sequence(&self.inner.borrow().longest_common_subsequence())
    }

    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Vec<u32> {
        positions(self.inner.borrow().array())
    }

    #[napi(js_name = "toString")]
    pub fn to_js_string(&self) -> String {
        self.inner.borrow().to_joined_string()
    }
}

// Shim standing in for upstream `symspell.js`.
//
// The original test file does `require('../symspell.js')` and, separately,
// `require('damerau-levenshtein')` to *validate* (not compute) distances --
// `damerau-levenshtein` is a real harness dependency
// (`tests/harness-package.json`), not something this shim needs to touch.
// Resolves the native addon through `@port/addon`, published into the work
// tree's node_modules so the lookup is depth-independent.
//
// `SymSpell` has no `Symbol.iterator` upstream, so there is nothing for
// `crates/mnemonist-napi/src/cursor.rs`'s `ITERATOR_FACTORIES` to install for
// this class.
module.exports = require('@port/addon').SymSpell;

// Shim standing in for upstream `passjoin-index.js`.
//
// The original test file does `require('../passjoin-index.js')` and,
// separately, `require('leven')` for its distance function -- `leven` is a
// real harness dependency (`tests/harness-package.json`), not something this
// shim needs to touch. Resolves the native addon through `@port/addon`,
// published into the work tree's node_modules so the lookup is
// depth-independent.
//
// `PassjoinIndex.prototype[Symbol.iterator]` is aliased to `values` from Rust
// (`crates/mnemonist-napi/src/cursor.rs`'s `ITERATOR_FACTORIES`), not here --
// `test/passjoin-index.js` uses `for (var string of index)`, so that
// installation is load-bearing for this unit.
module.exports = require('@port/addon').PassjoinIndex;

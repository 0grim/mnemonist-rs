// Shim standing in for upstream `fuzzy-multi-map.js`.
//
// The original test file does `require('../fuzzy-multi-map.js')`. Resolves
// the native addon through `@port/addon`, published into the work tree's
// node_modules so the lookup is depth-independent.
//
// `FuzzyMultiMap.prototype[Symbol.iterator]` is aliased to `values` from
// Rust (`crates/mnemonist-napi/src/cursor.rs`'s `ITERATOR_FACTORIES`), not
// here.
module.exports = require('@port/addon').FuzzyMultiMap;

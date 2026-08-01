// Shim standing in for upstream `fuzzy-map.js`.
//
// The original test file does `require('../fuzzy-map.js')`. Resolves the
// native addon through `@port/addon`, published into the work tree's
// node_modules so the lookup is depth-independent.
//
// `FuzzyMap.prototype[Symbol.iterator]` is aliased to `values` from Rust
// (`crates/mnemonist-napi/src/cursor.rs`), not here -- see `default-map.js`'s
// shim for the same note.
module.exports = require('@port/addon').FuzzyMap;

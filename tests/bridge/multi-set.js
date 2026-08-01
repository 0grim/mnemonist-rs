// Shim standing in for upstream `multi-set.js`.
//
// The original test file does `require('../multi-map.js')` as well (see
// `tests/bridge/multi-map.js`) for its one `MultiSet.from(map)` case.
// Resolves the native addon through `@port/addon`, published into the work
// tree's node_modules so the lookup is depth-independent.
//
// `MultiSet.prototype[Symbol.iterator]` is aliased to `values` from Rust
// (`crates/mnemonist-napi/src/cursor.rs`'s `ITERATOR_FACTORIES`), not here.
module.exports = require('@port/addon').MultiSet;

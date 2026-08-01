// Shim standing in for upstream `multi-map.js`.
//
// The original test file does `require('../multi-map.js')` and
// `require('../vector.js')` (see `tests/bridge/vector.js`, already a
// separate shim for the already-ported `Vector` unit). Resolves the native
// addon through `@port/addon`, published into the work tree's node_modules
// so the lookup is depth-independent.
//
// `MultiMap.prototype[Symbol.iterator]` is aliased to `entries` from Rust
// (`crates/mnemonist-napi/src/cursor.rs`'s `ITERATOR_FACTORIES`), not here.
module.exports = require('@port/addon').MultiMap;

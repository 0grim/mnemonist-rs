// Shim standing in for upstream `bi-map.js`.
//
// The original test file does `require('../bi-map.js')`, so a module has to
// exist at this path inside the assembled work tree. It resolves the native
// addon through `@port/addon`, which is published into the work tree's
// node_modules so the lookup is depth-independent.
//
// Note what is NOT here: `BiMap.prototype[Symbol.iterator]` and
// `BiMapInverse.prototype[Symbol.iterator]`. Upstream aliases both to
// `entries` (its last non-inspect lines), and so does ours -- in Rust, from
// `crates/mnemonist-napi/src/cursor.rs`, when the addon loads. A shim that
// added semantics would mean the addon was incomplete without the test
// harness, which is exactly backwards.
module.exports = require('@port/addon').BiMap;

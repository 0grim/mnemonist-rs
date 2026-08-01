// Shim standing in for upstream `vector.js`. See `tests/bridge/stack.js` for
// why this file carries no semantics of its own -- `Symbol.iterator` and the
// `Uint8Vector`/`Uint16Vector`/`Uint32Vector`/`Float64Vector`/`PointerVector`
// subclasses are all installed by the addon when it loads
// (`crates/mnemonist-napi/src/cursor.rs`, `crates/mnemonist-napi/src/vector.rs`).
module.exports = require('@port/addon').Vector;

// Shim standing in for upstream `lru-cache-with-delete.js`. See
// `tests/bridge/stack.js` for why this file carries no semantics of its own
// -- `Symbol.iterator` is installed by the addon when it loads
// (`crates/mnemonist-napi/src/cursor.rs`).
module.exports = require('@port/addon').LRUCacheWithDelete;

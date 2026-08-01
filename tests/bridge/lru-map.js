// Shim standing in for upstream `lru-map.js`. See `tests/bridge/stack.js` for
// why this file carries no semantics of its own -- `Symbol.iterator` is
// installed by the addon when it loads (`crates/mnemonist-napi/src/cursor.rs`).
module.exports = require('@port/addon').LRUMap;

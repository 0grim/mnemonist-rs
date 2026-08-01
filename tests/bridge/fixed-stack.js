// Shim standing in for upstream `fixed-stack.js`. See `tests/bridge/stack.js`
// for why this file carries no semantics of its own -- `Symbol.iterator` is
// installed by the addon when it loads
// (`crates/mnemonist-napi/src/cursor.rs`), and `FixedStack` has no `of`.
module.exports = require('@port/addon').FixedStack;

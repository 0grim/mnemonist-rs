// Shim standing in for upstream `fixed-deque.js`. See `tests/bridge/stack.js`
// for why this file carries no semantics of its own -- `Symbol.iterator` is
// installed by the addon when it loads.
module.exports = require('@port/addon').FixedDeque;

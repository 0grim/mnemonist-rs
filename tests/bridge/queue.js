// Shim standing in for upstream `queue.js`. See `tests/bridge/stack.js` for
// why this file carries no semantics of its own -- `Symbol.iterator` and
// `Queue.of` are both installed by the addon when it loads.
module.exports = require('@port/addon').Queue;

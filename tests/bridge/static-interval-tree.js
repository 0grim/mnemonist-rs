// Shim standing in for upstream `static-interval-tree.js`. See
// `tests/bridge/stack.js` for why this file carries no semantics of its own --
// `StaticIntervalTree` has no iterator and no `of`.
module.exports = require('@port/addon').StaticIntervalTree;

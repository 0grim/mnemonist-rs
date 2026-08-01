// Shim standing in for upstream `hashed-array-tree.js`.
//
// The original test file does `require('../hashed-array-tree.js')`, so a module
// has to exist at this path inside the assembled work tree. It resolves the
// native addon through `@port/addon`, which is published into the work tree's
// node_modules so the lookup is depth-independent.
//
// HashedArrayTree has no iterator and no statics, so unlike `sparse-set.js`
// there is nothing for the addon to install onto the prototype at load time --
// the export is the class and nothing else.
module.exports = require('@port/addon').HashedArrayTree;

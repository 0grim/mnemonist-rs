// Shim standing in for upstream `default-weak-map.js`.
//
// The original test file does `require('../default-weak-map.js')`, so a
// module has to exist at this path inside the assembled work tree. It
// resolves the native addon through `@port/addon`, published into the work
// tree's node_modules so the lookup is depth-independent.
//
// `DefaultWeakMap` has no iterator at all upstream -- no `Symbol.iterator`,
// no `values`/`keys`/`entries` -- so there is no `ITERATOR_FACTORIES` row for
// it either. See `docs/modules/default-weak-map.md`.
module.exports = require('@port/addon').DefaultWeakMap;

// Shim standing in for upstream `utils/typed-arrays.js`.
//
// `test/sort.js` does `require('../utils/typed-arrays.js')` and uses exactly
// one export from it, `indices`. Only that one is bridged: the repo's standing
// policy for this file (see the module docs on
// `crates/mnemonist-core/src/utils/typed_arrays.rs`) is that helpers land as
// modules reach them, so that an unported helper is an absence rather than a
// stub that looks implemented.
//
// The addon exports it as `typedArraysIndices`, because `indices` is far too
// generic a name to claim at the top level of an addon that will eventually
// carry forty modules' worth of helpers. The mapping is assembled in
// `../sort.js` with the rest of this unit's export shape.
module.exports = require('../sort.js').typedArrays;

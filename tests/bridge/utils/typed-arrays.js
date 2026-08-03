// Shim standing in for upstream `utils/typed-arrays.js`.
//
// Two units reach this file, each for different exports of it, and each
// assembles the shape in its own hub rather than here `docs/METHODOLOGY.md`'s gate 3,
// Problem 2 -- an aggregate shim per *caller* unit, not per upstream file):
//
//   * `test/sort.js` uses exactly `indices`, assembled in `../sort.js`.
//   * `test/_utils.js` additionally uses `getPointerArray`,
//     `getMinimalRepresentation` and `concat`, assembled in `../_utils.js`
//     alongside this file's four siblings (`binary-search.js`,
//     `hash-tables.js`, `merge.js`, `iterables.js`).
//
// The repo's standing policy for this file (see the module docs on
// `crates/mnemonist-core/src/utils/typed_arrays.rs`) is that helpers land as
// modules reach them, so that an unported helper is an absence rather than a
// stub that looks implemented -- which is why this is two re-exports merged,
// not a rewrite of either.
var sortShape = require('../sort.js').typedArrays;
var utilsShape = require('../_utils.js').typedArrays;

module.exports = {
  indices: sortShape.indices,
  getPointerArray: utilsShape.getPointerArray,
  getMinimalRepresentation: utilsShape.getMinimalRepresentation,
  concat: utilsShape.concat
};

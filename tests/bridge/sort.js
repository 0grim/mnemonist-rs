// Shim standing in for upstream's `sort/` directory.
//
// This unit is the first whose upstream surface is not one file exporting one
// constructor. `test/sort.js` requires THREE modules -- `sort/insertion.js`,
// `sort/quick.js` and `utils/typed-arrays.js` -- and none of them exports a
// class. So there is no `module.exports = addon.Something` to write; the shim
// has to re-assemble a two-file export shape out of the addon's flat
// namespace, which is precisely DESIGN.md 2.3's Problem 2.
//
// This file is where that assembly lives, and `sort/insertion.js` and
// `sort/quick.js` are cut from it. Doing it the other way round -- two leaf
// shims and an aggregate that merely re-requires them -- would leave this file
// decorative, and it is not: `tests/verify.sh` gate 3 looks for a shim named
// after the unit, and the unit is `sort`.
//
// Names differ between the two sides on purpose. The addon exports at top
// level, so `indices` alone is far too generic a name to claim for a helper
// that only `utils/typed-arrays.js` should own; it is `typedArraysIndices`
// there and mapped back here. Renaming is not adding semantics -- no behaviour
// is implemented in this file, and the addon is complete without it.
const addon = require('@port/addon');

exports.insertion = {
  inplaceInsertionSort: addon.inplaceInsertionSort,
  inplaceInsertionSortIndices: addon.inplaceInsertionSortIndices
};

exports.quick = {
  inplaceQuickSort: addon.inplaceQuickSort,
  inplaceQuickSortIndices: addon.inplaceQuickSortIndices
};

exports.typedArrays = {
  indices: addon.typedArraysIndices
};

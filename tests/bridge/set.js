// Shim standing in for upstream `set.js`.
//
// The original test file does `require('../set.js')`, so a module has to exist
// at this path inside the assembled work tree. It resolves the native addon
// through `@port/addon`, which is published into the work tree's node_modules
// so the lookup is depth-independent.
//
// This is not the usual one-line `module.exports = addon.Something`, because
// `set.js` exports no constructor -- fourteen free functions, into an addon
// whose exports are one flat namespace. Two adaptations, both of them export
// shape rather than semantics `docs/METHODOLOGY.md`'s gate 3, Problem 2):
//
//   1. NAMES. `union` and `add` at the top level of an addon that will
//      eventually carry forty modules' worth of helpers would be indefensible,
//      so the addon prefixes them and they are mapped back here.
//
//   2. ARITY. `intersection` and `union` are variadic upstream and napi has no
//      variadic parameter, so the addon takes an array and the spread happens
//      here. The "needs at least two arguments" check stays in the port, where
//      upstream's message and threshold live -- the shim passes whatever it was
//      given, including nothing, and lets the port refuse it.
//
// No behaviour is implemented in this file. The addon is complete without it.
const addon = require('@port/addon');

exports.intersection = function() {
  return addon.setIntersection(Array.prototype.slice.call(arguments));
};

exports.union = function() {
  return addon.setUnion(Array.prototype.slice.call(arguments));
};

exports.difference = addon.setDifference;
exports.symmetricDifference = addon.setSymmetricDifference;
exports.isSubset = addon.setIsSubset;
exports.isSuperset = addon.setIsSuperset;

exports.add = addon.setAdd;
exports.subtract = addon.setSubtract;
exports.intersect = addon.setIntersect;
exports.disjunct = addon.setDisjunct;

exports.intersectionSize = addon.setIntersectionSize;
exports.unionSize = addon.setUnionSize;
exports.jaccard = addon.setJaccard;
exports.overlap = addon.setOverlap;

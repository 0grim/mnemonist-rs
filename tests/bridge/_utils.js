// Shim standing in for the FIVE files `test/_utils.js` requires --
// `utils/typed-arrays.js`, `utils/binary-search.js`, `utils/merge.js`,
// `utils/hash-tables.js` and `utils/iterables.js` -- assembled once here and
// cut into `tests/bridge/utils/*.js` leaves, exactly as `tests/bridge/sort.js`
// does for `test/sort.js`'s own three-file require-closure. Doing it the
// other way round -- five independent leaf shims -- would leave this file
// decorative, and `tests/verify.sh` gate 3 looks for a shim named after the
// unit; the unit is `_utils`.
//
// Two adaptations, both export shape rather than semantics (DESIGN.md 2.3,
// Problem 2):
//
//   1. NAMES. The addon exports one flat namespace, so most of these are
//      prefixed there (`binarySearchLowerBound`, `hashTablesJenkinsInt32`, ...)
//      and mapped back to upstream's names here. `isArrayLike`/`guessLength`/
//      `toArray`/`toArrayWithIndices` are the exception -- they were bridged
//      for `fixed-stack`/`fixed-deque`/`circular-buffer` before this unit
//      existed and already carry their upstream names at the top level.
//
//   2. ARITY. `merge`/`unionUnique`/`intersectionUnique` are variadic
//      upstream (`arguments.length === 2` takes one path, anything else
//      another) and napi has no variadic parameter -- the same gap
//      `tests/bridge/set.js` already closes for `union`/`intersection` there.
//      The dispatch itself is arity glue, not semantics: it decides which
//      addon function to call and never inspects a value upstream wouldn't
//      have inspected. `isArrayLike` is the addon's own
//      `crate::iterables::js_is_array_like`, already live at the top level.
//
// No behaviour beyond that dispatch is implemented in this file. Everything
// else is the addon.
const addon = require('@port/addon');

exports.typedArrays = {
  indices: require('./sort.js').typedArrays.indices,
  getPointerArray: addon.typedArraysGetPointerArray,
  getMinimalRepresentation: addon.typedArraysGetMinimalRepresentation,
  concat: function() {
    return addon.typedArraysConcat(Array.prototype.slice.call(arguments));
  }
};

exports.binarySearch = {
  search: addon.binarySearchSearch,
  searchWithComparator: addon.binarySearchSearchWithComparator,
  lowerBound: addon.binarySearchLowerBound,
  lowerBoundWithComparator: addon.binarySearchLowerBoundWithComparator,
  lowerBoundIndices: addon.binarySearchLowerBoundIndices,
  upperBound: addon.binarySearchUpperBound,
  upperBoundWithComparator: addon.binarySearchUpperBoundWithComparator
};

exports.hashTables = {
  hashes: {
    jenkinsInt32: addon.hashTablesJenkinsInt32
  },
  linearProbing: {
    get: addon.hashTablesLinearProbingGet,
    has: addon.hashTablesLinearProbingHas,
    set: addon.hashTablesLinearProbingSet
  }
};

exports.iterables = {
  isArrayLike: addon.isArrayLike,
  guessLength: addon.guessLength,
  toArray: addon.toArray,
  toArrayWithIndices: addon.toArrayWithIndices
};

// `merge.js`'s own three exports: variadic, dispatching on `arguments.length`
// then on `isArrayLike(arguments[0])`, exactly as upstream's own `merge`/
// `unionUnique`/`intersectionUnique` do -- see `mnemonist_core::utils::merge`'s
// module docs for the two-array vs. k-way split this mirrors.
function variadic(two, many) {
  return function() {
    var args = Array.prototype.slice.call(arguments);

    if (!addon.isArrayLike(args[0]))
      return null;

    if (args.length === 2)
      return two(args[0], args[1]);

    return many(args);
  };
}

exports.merge = {
  merge: variadic(addon.mergeTwo, addon.mergeMany),
  unionUnique: variadic(addon.unionUniqueTwo, addon.unionUniqueMany),
  intersectionUnique: variadic(
    addon.intersectionUniqueTwo,
    addon.intersectionUniqueMany
  )
};

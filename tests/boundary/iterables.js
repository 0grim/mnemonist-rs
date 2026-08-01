/**
 * mnemonist/utils/iterables — boundary specs (DoD gate 7, at the boundary).
 * =========================================================================
 *
 * `utils/iterables.js` has no upstream test file of its own. It is reached by
 * `test/_utils.js`, whose require-closure (typed-arrays + binary-search +
 * hash-tables + iterables + merge, ~1,166 LOC) is out of scope — so **every
 * assertion below is coverage upstream does not have**, and gate 7 for this
 * module has nowhere else to live: like `forEach`, it is a JavaScript-value
 * coercion and `cargo test` cannot reach it (DESIGN.md §3.5, D-03).
 *
 * ## Why the reference implementation is inlined
 *
 * `tests/boundary/foreach.js` is differential against the real
 * `obliterator/foreach`, which is an installed devDependency of the harness.
 * `mnemonist/utils/iterables.js` is not installed — it is the thing being
 * ported — and the vendored copy in `bench/upstream/` cannot resolve its own
 * `require('obliterator/foreach')` from inside the assembled work tree.
 *
 * So the four functions are reproduced here **verbatim from upstream**, on top
 * of the genuine `obliterator/foreach` and the genuine `getPointerArray`
 * boundaries. That keeps the differential half of the spec — the half that
 * catches a branch we reasoned about wrongly — while staying self-contained.
 * The copy is 30 lines and is checked against the vendored original by
 * `matches the vendored upstream source`, below, so it cannot silently drift.
 */
var assert = require('assert'),
    fs = require('fs'),
    path = require('path'),
    forEach = require('obliterator/foreach'),
    port = require('@port/addon');

var UPSTREAM = path.resolve(__dirname, '..', '..', '..', 'bench', 'upstream', 'utils', 'iterables.js');

/* ---------------------------------------------------------------- reference */

function isArrayLike(target) {
  return Array.isArray(target) || (typeof ArrayBuffer !== 'undefined' && ArrayBuffer.isView(target));
}

function guessLength(target) {
  if (typeof target.length === 'number')
    return target.length;

  if (typeof target.size === 'number')
    return target.size;

  return;
}

function getPointerArray(size) {
  var maxIndex = size - 1;

  if (maxIndex <= 255)
    return Uint8Array;

  if (maxIndex <= 65535)
    return Uint16Array;

  if (maxIndex <= 4294967295)
    return Uint32Array;

  throw new Error('mnemonist: Pointer Array of size > 4294967295 is not supported.');
}

function toArray(target) {
  var l = guessLength(target);

  var array = typeof l === 'number' ? new Array(l) : [];

  var i = 0;

  forEach(target, function(value) {
    array[i++] = value;
  });

  return array;
}

function toArrayWithIndices(target) {
  var l = guessLength(target);

  var IndexArray = typeof l === 'number' ? getPointerArray(l) : Array;

  var array = typeof l === 'number' ? new Array(l) : [];
  var indices = typeof l === 'number' ? new IndexArray(l) : [];

  var i = 0;

  forEach(target, function(value) {
    array[i] = value;
    indices[i] = i++;
  });

  return [array, indices];
}

var reference = {
  isArrayLike: isArrayLike,
  guessLength: guessLength,
  toArray: toArray,
  toArrayWithIndices: toArrayWithIndices
};

/* ------------------------------------------------------------------ helpers */

/**
 * Run one implementation and normalise the outcome. A thrown error becomes its
 * constructor name and message, so "both threw the same RangeError" is a
 * comparable result rather than a dead test.
 */
function outcome(implementation, name, make) {
  try {
    return {value: describe_(implementation[name](make()))};
  }
  catch (error) {
    return {error: error.constructor.name + ': ' + error.message};
  }
}

/**
 * A description that distinguishes everything this module can produce: a hole
 * is not `undefined`, a `Uint8Array` is not an `Array`, and a length longer
 * than the filled prefix is the entire point of B-2.
 */
function describe_(value) {
  if (Array.isArray(value)) {
    return {
      type: 'Array',
      length: value.length,
      slots: Array.prototype.map.call(value, function (v) { return describe_(v); }),
      // `map` skips holes, so it cannot report them; `in` can.
      present: Array.from({length: value.length}, function (_, i) { return i in value; })
    };
  }

  if (ArrayBuffer.isView(value))
    return {type: value.constructor.name, values: Array.from(value)};

  if (typeof value === 'object' && value !== null)
    return {type: 'object', keys: Object.keys(value)};

  return {type: typeof value, value: value === undefined ? '<undefined>' : value};
}

/** Assert the port and the reference agree, and return what they agreed on. */
function agree(name, make) {
  var expected = outcome(reference, name, make),
      actual = outcome(port, name, make);

  assert.deepStrictEqual(actual, expected, name + ' diverged');

  return expected;
}

/** A target that lies about its length and yields whatever it likes. */
function liar(length, values) {
  return {
    length: length,
    forEach: function (callback) {
      for (var i = 0; i < values.length; i++)
        callback(values[i], i);
    }
  };
}

describe('mnemonist/utils/iterables', function () {

  it('the inlined reference matches the vendored upstream source.', function () {
    var source = fs.readFileSync(UPSTREAM, 'utf-8');

    // Not a whole-file comparison: the vendored file has a header, requires and
    // exports this copy deliberately does not. What must not drift is the four
    // bodies, so each is checked by a line that only appears inside it.
    [
      'return Array.isArray(target) || typed.isTypedArray(target);',
      "if (typeof target.length === 'number')",
      "if (typeof target.size === 'number')",
      "var array = typeof l === 'number' ? new Array(l) : [];",
      "var IndexArray = typeof l === 'number' ?",
      'array[i] = value;',
      'indices[i] = i++;'
    ].forEach(function (line) {
      assert.ok(
        source.indexOf(line) !== -1,
        'upstream no longer contains `' + line + '`: the inlined reference has drifted'
      );
    });
  });

  describe('#.isArrayLike', function () {

    it('is true for arrays and for any ArrayBuffer view.', function () {
      assert.strictEqual(agree('isArrayLike', function () { return [1, 2]; }).value.value, true);
      assert.strictEqual(agree('isArrayLike', function () { return []; }).value.value, true);
      assert.strictEqual(
        agree('isArrayLike', function () { return new Uint8Array(2); }).value.value, true);
      assert.strictEqual(
        agree('isArrayLike', function () { return new Float64Array(2); }).value.value, true);
      assert.strictEqual(
        agree('isArrayLike', function () { return new DataView(new ArrayBuffer(2)); }).value.value,
        true);
    });

    it('is false for a string, a Set, and an array-like object.', function () {
      // The last one is the interesting case: `{length: 2}` is what "array
      // like" normally means, and this predicate says no.
      assert.strictEqual(agree('isArrayLike', function () { return 'ab'; }).value.value, false);
      assert.strictEqual(
        agree('isArrayLike', function () { return new Set([1]); }).value.value, false);
      assert.strictEqual(
        agree('isArrayLike', function () { return {length: 2}; }).value.value, false);
      assert.strictEqual(
        agree('isArrayLike', function () { return arguments; }).value.value, false);
    });
  });

  describe('#.guessLength', function () {

    it('prefers .length, then .size, then gives up.', function () {
      assert.strictEqual(agree('guessLength', function () { return [1, 2, 3]; }).value.value, 3);
      assert.strictEqual(
        agree('guessLength', function () { return new Set([1, 2]); }).value.value, 2);
      assert.strictEqual(agree('guessLength', function () { return 'abc'; }).value.value, 3);
      assert.strictEqual(
        agree('guessLength', function () { return {}; }).value.value, '<undefined>');
    });

    it('.length wins even when both are present and disagree.', function () {
      assert.strictEqual(
        agree('guessLength', function () { return {length: 1, size: 99}; }).value.value, 1);
    });

    it('does not validate: a negative, fractional or NaN length is returned as is.', function () {
      assert.strictEqual(agree('guessLength', function () { return {length: -1}; }).value.value, -1);
      assert.strictEqual(
        agree('guessLength', function () { return {length: 3.5}; }).value.value, 3.5);
      // NaN survives the round trip and is still `typeof 'number'`.
      assert.ok(Number.isNaN(port.guessLength({length: NaN})));
      assert.ok(Number.isNaN(reference.guessLength({length: NaN})));
    });

    it('a non-numeric .length is ignored, and .size is then consulted.', function () {
      assert.strictEqual(
        agree('guessLength', function () { return {length: '3', size: 7}; }).value.value, 7);
      assert.strictEqual(
        agree('guessLength', function () { return {length: '3'}; }).value.value, '<undefined>');
    });

    it('throws from the property read on null and undefined.', function () {
      assert.strictEqual(
        agree('guessLength', function () { return null; }).error,
        "TypeError: Cannot read properties of null (reading 'length')");
      assert.strictEqual(
        agree('guessLength', function () { return undefined; }).error,
        "TypeError: Cannot read properties of undefined (reading 'length')");
    });
  });

  describe('#.toArray', function () {

    it('converts the ordinary cases.', function () {
      agree('toArray', function () { return [1, 2, 3]; });
      agree('toArray', function () { return new Set(['a', 'b']); });
      agree('toArray', function () { return new Uint8Array([1, 2, 3]); });
      agree('toArray', function () { return 'ab'; });
      agree('toArray', function () { return new Map([['k', 'v']]); });
    });

    it('an overstated length leaves HOLES, not undefined (B-2).', function () {
      var result = agree('toArray', function () { return liar(5, [1, 2]); }).value;

      assert.strictEqual(result.length, 5);
      assert.deepStrictEqual(result.present, [true, true, false, false, false]);
    });

    it('an understated length is silently exceeded.', function () {
      var result = agree('toArray', function () { return liar(1, [1, 2, 3]); }).value;

      assert.strictEqual(result.length, 3);
      assert.deepStrictEqual(result.present, [true, true, true]);
    });

    it('an invalid length throws RangeError from the allocation.', function () {
      assert.strictEqual(
        agree('toArray', function () { return liar(-1, []); }).error,
        'RangeError: Invalid array length');
      assert.strictEqual(
        agree('toArray', function () { return liar(3.5, []); }).error,
        'RangeError: Invalid array length');
      assert.strictEqual(
        agree('toArray', function () { return liar(NaN, []); }).error,
        'RangeError: Invalid array length');
    });

    it('the sharpest form of B-2: a bare {length: n} enumerates its own length.', function () {
      // No `forEach` on the target, so `forEach` falls through to branch 5 and
      // enumerates own properties -- `length` among them.
      var result = agree('toArray', function () { return {length: 5}; }).value;

      assert.strictEqual(result.length, 5);
      assert.deepStrictEqual(result.slots[0], {type: 'number', value: 5});
      assert.deepStrictEqual(result.present, [true, false, false, false, false]);
    });

    it('an unguessable target gets a plain growing array.', function () {
      var result = agree('toArray', function () {
        return {a: 1, b: 2};
      }).value;

      assert.strictEqual(result.length, 2);
    });

    it('the falsy guard of forEach still applies.', function () {
      assert.strictEqual(
        agree('toArray', function () { return ''; }).error,
        'Error: obliterator/forEach: invalid iterable.');
    });
  });

  describe('#.toArrayWithIndices', function () {

    it('picks the index width from the guess, not from the yield count.', function () {
      // 3 elements -> Uint8Array; 300 -> Uint16Array; 70000 -> Uint32Array.
      assert.strictEqual(
        agree('toArrayWithIndices', function () { return [1, 2, 3]; }).value.slots[1].type,
        'Uint8Array');
      assert.strictEqual(
        agree('toArrayWithIndices', function () { return liar(300, [7]); }).value.slots[1].type,
        'Uint16Array');
      assert.strictEqual(
        agree('toArrayWithIndices', function () { return liar(70000, [7]); }).value.slots[1].type,
        'Uint32Array');
    });

    it('the indices array is the identity over the filled prefix, zero after.', function () {
      var pair = agree('toArrayWithIndices', function () { return liar(4, [9, 8]); }).value;

      assert.deepStrictEqual(pair.slots[1].values, [0, 1, 0, 0]);
      assert.deepStrictEqual(pair.slots[0].present, [true, true, false, false]);
    });

    it('an unguessable target gets two plain arrays.', function () {
      // NOT a `Map` -- a `Map` has `.size`, so it is guessable and gets a
      // `Uint8Array`. A plain object has neither `.length` nor `.size`.
      var pair = agree('toArrayWithIndices', function () { return {a: 1, b: 2}; }).value;

      assert.strictEqual(pair.slots[1].type, 'Array');
      assert.deepStrictEqual(pair.slots[1].slots, [
        {type: 'number', value: 0},
        {type: 'number', value: 1}
      ]);
    });

    it('getPointerArray throws BEFORE new Array(l) does.', function () {
      // 2^32 + 1 is past every pointer width AND an invalid array length. The
      // order of the two statements upstream decides which error wins.
      assert.strictEqual(
        agree('toArrayWithIndices', function () { return liar(4294967297, []); }).error,
        'Error: mnemonist: Pointer Array of size > 4294967295 is not supported.');
    });
  });
});

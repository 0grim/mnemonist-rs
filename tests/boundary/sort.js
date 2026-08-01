/**
 * Sort helpers — boundary specs (DoD gate 7, at the boundary).
 * =============================================================
 *
 * `cargo test` covers the two algorithms. What it cannot cover is anything
 * about the *object* that crosses the FFI boundary, and for a family of
 * functions whose whole contract is "in place" that is most of what matters:
 *
 *   - `test/sort.js` never checks that the returned array IS the argument. It
 *     only inspects the return value, so a port that sorted a copy and handed
 *     the copy back would pass all thirteen of its assertions while breaking
 *     every caller inside mnemonist itself.
 *   - Neither does it check that the *caller's* typed array was mutated, which
 *     is the same claim seen from the other side.
 *   - `utils/typed-arrays.js#indices` picks its element width from the length.
 *     `test/sort.js` calls it once, with 11, so only `Uint8Array` is ever
 *     constructed and the 16- and 32-bit branches are untested upstream.
 *   - The stated divergences (non-numeric elements, out-of-range windows) are
 *     refusals, and a refusal that silently became an acceptance would look
 *     like nothing at all.
 *
 * Every claim about the *values* is asserted differentially against the
 * vendored upstream source in `bench/upstream/`, so it says "the port and
 * upstream agree" rather than "the port matches what I typed". The identity
 * and width claims are asserted directly, because they are properties of the
 * object rather than of the answer.
 */
var assert = require('assert'),
    fs = require('fs'),
    Module = require('module'),
    path = require('path');

// `bench/upstream/` is vendored source, not an installed package. Point Node's
// global resolution at the work tree's installed dependencies, exactly as
// `tests/boundary/stack-queue.js` and `fuzz/oracle.js` do.
process.env.NODE_PATH = path.resolve(__dirname, '..', 'node_modules') +
  (process.env.NODE_PATH ? path.delimiter + process.env.NODE_PATH : '');
Module._initPaths();

// The work tree is assembled at an arbitrary depth under the repository root,
// so walk up until `bench/upstream` appears rather than hard-coding the depth.
function repositoryRoot() {
  var directory = __dirname;

  for (var i = 0; i < 8; i++) {
    if (fs.existsSync(path.join(directory, 'bench', 'upstream', 'sort', 'quick.js')))
      return directory;

    directory = path.dirname(directory);
  }

  throw new Error('cannot locate bench/upstream from ' + __dirname);
}

var UPSTREAM = path.join(repositoryRoot(), 'bench', 'upstream');

var upstream = {
  insertion: require(path.join(UPSTREAM, 'sort', 'insertion.js')),
  quick: require(path.join(UPSTREAM, 'sort', 'quick.js')),
  typed: require(path.join(UPSTREAM, 'utils', 'typed-arrays.js'))
};

var port = {
  insertion: require('../sort/insertion.js'),
  quick: require('../sort/quick.js'),
  typed: require('../utils/typed-arrays.js')
};

var DATA = [2, 7, 1, 5, 8, 9, 1, -3, 3, 18, 6];

/** Both flavours, so every spec below runs twice without being written twice. */
var ALGORITHMS = [
  {name: 'insertion', values: 'inplaceInsertionSort', indices: 'inplaceInsertionSortIndices'},
  {name: 'quick', values: 'inplaceQuickSort', indices: 'inplaceQuickSortIndices'}
];

describe('sort — boundary', function() {

  ALGORITHMS.forEach(function(algorithm) {

    describe(algorithm.name, function() {

      it('should return the very array it was given, not a copy.', function() {
        var array = DATA.slice();
        var returned = port[algorithm.name][algorithm.values](array, 0, array.length);

        assert.strictEqual(returned, array, 'the return value must be the argument');
        assert.deepStrictEqual(array, [-3, 1, 1, 2, 3, 5, 6, 7, 8, 9, 18]);
      });

      it('should return the very indices array it was given, and mutate it.', function() {
        var indices = port.typed.indices(DATA.length);
        var returned = port[algorithm.name][algorithm.indices](DATA.slice(), indices, 0, DATA.length);

        assert.strictEqual(returned, indices, 'the return value must be the argument');

        // Read the caller's handle, never the returned one: this is the half
        // that a port sorting a copy would fail.
        var expected = upstream[algorithm.name][algorithm.indices](
          DATA.slice(), upstream.typed.indices(DATA.length), 0, DATA.length);

        assert.deepStrictEqual(Array.from(indices), Array.from(expected));
      });

      it('should leave elements outside the window untouched, including non-numbers.', function() {
        // Upstream is duck-typed and never reads outside [lo, hi); so must the
        // port, which is why it reads a window rather than the whole array.
        var array = ['not a number', 3, 1, 2, {}];
        var expected = upstream[algorithm.name][algorithm.values](array.slice(), 1, 4);

        var returned = port[algorithm.name][algorithm.values](array, 1, 4);

        assert.deepStrictEqual(returned.slice(1, 4), expected.slice(1, 4));
        assert.strictEqual(returned[0], 'not a number');
        assert.strictEqual(returned[4], array[4]);
      });

      it('should agree with upstream across every window of a fixed array.', function() {
        for (var lo = 0; lo <= DATA.length; lo++) {
          for (var hi = lo; hi <= DATA.length; hi++) {
            var mine = port[algorithm.name][algorithm.values](DATA.slice(), lo, hi);
            var theirs = upstream[algorithm.name][algorithm.values](DATA.slice(), lo, hi);

            assert.deepStrictEqual(mine, theirs, 'values, window ' + lo + '..' + hi);

            var myIndices = port[algorithm.name][algorithm.indices](
              DATA.slice(), port.typed.indices(DATA.length), lo, hi);
            var theirIndices = upstream[algorithm.name][algorithm.indices](
              DATA.slice(), upstream.typed.indices(DATA.length), lo, hi);

            assert.deepStrictEqual(
              Array.from(myIndices), Array.from(theirIndices), 'indices, window ' + lo + '..' + hi);
          }
        }
      });

      it('should sort a Uint16Array of indices, a width upstream never tests.', function() {
        // 300 members forces `getPointerArray` past the 8-bit branch, which
        // `test/sort.js` — one call, length 11 — never leaves.
        var values = [];
        for (var i = 0; i < 300; i++) values.push((i * 7919) % 301);

        var mine = port.typed.indices(300);
        assert.strictEqual(mine.constructor, Uint16Array);

        port[algorithm.name][algorithm.indices](values, mine, 0, 300);

        var theirs = upstream[algorithm.name][algorithm.indices](
          values, upstream.typed.indices(300), 0, 300);

        assert.deepStrictEqual(Array.from(mine), Array.from(theirs));
      });

      it('should refuse a non-numeric element inside the window.', function() {
        // A stated divergence (docs/modules/sort.md): upstream compares such
        // elements through `valueOf`, which is bridge tier T2. The refusal is
        // asserted so that quietly starting to accept them would be noticed.
        assert.throws(function() {
          port[algorithm.name][algorithm.values](['b', 'a'], 0, 2);
        }, /number/);
      });

      it('should refuse a window past the end of the array.', function() {
        assert.throws(function() {
          port[algorithm.name][algorithm.values]([1, 2, 3], 0, 4);
        }, /sort bound/);

        assert.throws(function() {
          port[algorithm.name][algorithm.values]([1, 2, 3], 2, 1);
        }, /past hi/);

        assert.throws(function() {
          port[algorithm.name][algorithm.values]([1, 2, 3], 0.5, 3);
        }, /sort bound/);
      });
    });
  });

  describe('utils/typed-arrays#indices', function() {

    it('should pick the same element type as upstream at every boundary.', function() {
      // maxIndex = length - 1 against 2^n - 1, so the interesting lengths are
      // the two either side of each power. `test/sort.js` only ever passes 11.
      [0, 1, 255, 256, 257, 65535, 65536, 65537].forEach(function(length) {
        var mine = port.typed.indices(length);
        var theirs = upstream.typed.indices(length);

        assert.strictEqual(mine.constructor, theirs.constructor, 'type for ' + length);
        assert.deepStrictEqual(Array.from(mine), Array.from(theirs), 'values for ' + length);
      });
    });

    it('should throw upstream\'s own message past 2^32.', function() {
      var message = null;

      try {
        upstream.typed.indices(4294967297);
      } catch (error) {
        message = error.message;
      }

      assert.strictEqual(message, 'mnemonist: Pointer Array of size > 4294967295 is not supported.');
      assert.throws(function() {
        port.typed.indices(4294967297);
      }, new RegExp('Pointer Array of size'));
    });

    it('should truncate a fractional length while sizing the width from the raw one.', function() {
      // The subtle one, and the reason the core function takes an f64 rather
      // than a usize: `getPointerArray` compares `length - 1` as a double, so
      // it sees 256.5 and picks Uint16Array, while the TypedArray constructor
      // applies ToIndex and allocates 256 slots. The result is one width wider
      // than 256 elements need. This is the whole regime `test/sort.js`
      // never enters -- it calls `indices` once, with 11.
      [3.5, -0.5, 255.5, 256.5].forEach(function(length) {
        var mine = port.typed.indices(length);
        var theirs = upstream.typed.indices(length);

        assert.strictEqual(mine.constructor, theirs.constructor, 'type for ' + length);
        assert.deepStrictEqual(Array.from(mine), Array.from(theirs), 'values for ' + length);
      });

      assert.strictEqual(port.typed.indices(256.5).constructor, Uint16Array);
      assert.strictEqual(port.typed.indices(256.5).length, 256);
    });

    it('should refuse a negative length, as the TypedArray constructor does.', function() {
      assert.throws(function() { upstream.typed.indices(-1); }, /Invalid typed array length/);
      assert.throws(function() { port.typed.indices(-1); }, /Invalid typed array length/);
    });

    it('should refuse NaN and Infinity through getPointerArray, not the constructor.', function() {
      // Every comparison in `getPointerArray` is false for NaN, so it falls
      // through to mnemonist's own throw rather than reaching a RangeError.
      [NaN, Infinity].forEach(function(length) {
        assert.throws(function() { upstream.typed.indices(length); }, /Pointer Array of size/);
        assert.throws(function() { port.typed.indices(length); }, /Pointer Array of size/);
      });
    });
  });
});

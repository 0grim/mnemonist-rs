/**
 * obliterator/forEach — boundary specs (DoD gate 7, at the boundary).
 * ===================================================================
 *
 * `forEach` is the one primitive with no Rust-testable core: it is a
 * JavaScript-value coercion, so its semantics only exist where JavaScript
 * values do (DESIGN.md §3.5, D-03). `cargo test` cannot reach it. These specs
 * are therefore the gate-7 "what upstream does not test" coverage for the
 * dispatch, and they live outside `tests/original/` because nothing upstream
 * tests obliterator at all — mnemonist's suite exercises it only incidentally,
 * through `Stack.from([1, 2, 3])`.
 *
 * Two kinds of assertion here, deliberately:
 *
 *   1. **Differential.** The real `obliterator/foreach` is a harness devDep, so
 *      every case is run through both implementations and compared. This is
 *      what catches a branch we reasoned about wrongly.
 *   2. **Explicit.** Per-branch assertions naming the behaviour, so a
 *      divergence reads as "branch 2 stopped delegating" and not as
 *      "two opaque arrays differ".
 *
 * The differential half is not redundant with the explicit half: it pins the
 * cases we did NOT think to assert about.
 */
var assert = require('assert'),
    upstream = require('obliterator/foreach'),
    port = require('@port/addon').forEach;

/**
 * Run one implementation and normalise the outcome into something comparable:
 * either the sequence of (value, key, typeof key) triples, or the thrown
 * error's constructor and message. The key's *type* is part of the record
 * because it is the whole of D-11 — it is a number in branches 1 and 4, a
 * string in branch 5, and whatever the host chose in branch 2.
 */
function outcome(implementation, iterable) {
  var seen = [];

  try {
    implementation(iterable, function (value, key) {
      seen.push([value, key, typeof key]);
    });
  }
  catch (error) {
    return {error: error.constructor.name + ': ' + error.message};
  }

  return {seen: seen};
}

/**
 * Assert the port and upstream agree, and return what they agreed on so the
 * caller can additionally say what it should have been.
 */
function agree(make) {
  var expected = outcome(upstream, make()),
      actual = outcome(port, make());

  assert.deepStrictEqual(actual, expected);

  return expected;
}

function visited(make) {
  var result = agree(make);

  assert.ok(!result.error, 'expected iteration, got ' + result.error);

  return result.seen;
}

function threw(make) {
  var result = agree(make);

  assert.ok(result.error, 'expected a throw, got ' + JSON.stringify(result.seen));

  return result.error;
}

describe('obliterator/forEach', function () {

  describe('the falsy guard (D-12)', function () {

    it('should throw on every falsy value, verbatim.', function () {
      var falsy = {
        'empty string': function () { return ''; },
        'zero': function () { return 0; },
        'negative zero': function () { return -0; },
        'false': function () { return false; },
        'NaN': function () { return NaN; },
        'null': function () { return null; },
        'undefined': function () { return undefined; },
        'zero bigint': function () { return 0n; }
      };

      Object.keys(falsy).forEach(function (name) {
        assert.strictEqual(
          threw(falsy[name]),
          'Error: obliterator/forEach: invalid iterable.',
          name
        );
      });
    });

    it('should iterate a one-character string but throw on an empty one.', function () {
      // The sharpest form of the guard: '' and 'a' differ only in length, and
      // an empty string is a legitimately iterable value that yields zero
      // times. NOTES B-4.
      assert.strictEqual(threw(function () { return ''; }),
        'Error: obliterator/forEach: invalid iterable.');
      assert.deepStrictEqual(visited(function () { return 'a'; }), [['a', 0, 'number']]);
    });

    it('should NOT throw on empty-but-truthy containers.', function () {
      assert.deepStrictEqual(visited(function () { return []; }), []);
      assert.deepStrictEqual(visited(function () { return {}; }), []);
      assert.deepStrictEqual(visited(function () { return new Map(); }), []);
      assert.deepStrictEqual(visited(function () { return '0'; }), [['0', 0, 'number']]);
    });

    it('should check the iterable before the callback.', function () {
      // Order matters: a falsy iterable AND a bad callback reports the
      // iterable, because upstream's guards are in that order.
      assert.throws(function () { port(0, 'not a function'); },
        /obliterator\/forEach: invalid iterable\./);
    });
  });

  describe('the callback guard', function () {

    it('should throw verbatim for anything that is not a function.', function () {
      [5, undefined, null, {}, 'fn', []].forEach(function (callback) {
        var expected;

        try { upstream([1], callback); }
        catch (error) { expected = error.message; }

        assert.throws(function () { port([1], callback); }, function (error) {
          assert.strictEqual(error.message, expected);
          assert.strictEqual(error.message, 'obliterator/forEach: expecting a callback.');
          return true;
        });
      });
    });
  });

  describe('branch 1 — indexed sequences', function () {

    it('should walk arrays, passing the index as a number.', function () {
      assert.deepStrictEqual(visited(function () { return ['a', 'b']; }),
        [['a', 0, 'number'], ['b', 1, 'number']]);
    });

    it('should walk a sparse array by index, holes included.', function () {
      // `i < l` runs to the array's length, so holes are visited and arrive as
      // undefined. A port that iterated the dense elements would skip them.
      assert.deepStrictEqual(visited(function () { var a = [1]; a.length = 3; return a; }),
        [[1, 0, 'number'], [undefined, 1, 'number'], [undefined, 2, 'number']]);
    });

    it('should walk typed arrays and DataViews.', function () {
      assert.deepStrictEqual(visited(function () { return new Uint8Array([9, 8]); }),
        [[9, 0, 'number'], [8, 1, 'number']]);
      // ArrayBuffer.isView(dataView) is true, and a DataView has no `length`,
      // so `i < undefined` is false and it iterates zero times.
      assert.deepStrictEqual(visited(function () { return new DataView(new ArrayBuffer(4)); }), []);
    });

    it('should walk strings by UTF-16 code unit, splitting surrogate pairs.', function () {
      // The emoji is one code point and two code units, and `'\u{1F600}'[0]` is
      // the high surrogate alone. Iterating Rust `char`s would silently repair
      // that into one element.
      assert.deepStrictEqual(visited(function () { return '\u{1F600}x'; }),
        [['\ud83d', 0, 'number'], ['\ude00', 1, 'number'], ['x', 2, 'number']]);
    });

    it('should walk an arguments object, matched by its toString tag.', function () {
      assert.deepStrictEqual(
        visited(function () { return (function () { return arguments; })(7, 8); }),
        [[7, 0, 'number'], [8, 1, 'number']]
      );
    });

    it('should let a hostile toString hijack the branch (B-5).', function () {
      // The dispatch calls `toString()` on an arbitrary user value. Returning
      // the arguments tag routes a plain object through the indexed branch,
      // where it would otherwise have gone to branch 5 and yielded string keys.
      assert.deepStrictEqual(
        visited(function () {
          return {length: 2, 0: 'x', 1: 'y', toString: function () { return '[object Arguments]'; }};
        }),
        [['x', 0, 'number'], ['y', 1, 'number']]
      );
    });

    it('should propagate a throwing toString (B-5).', function () {
      assert.strictEqual(
        threw(function () { return {toString: function () { throw new Error('boom'); }}; }),
        'Error: boom'
      );
    });

    it('should die the way V8 dies when toString is absent entirely.', function () {
      // Object.create(null) has no toString, and the dispatch calls it
      // unguarded. The error names obliterator's own variable.
      assert.strictEqual(
        threw(function () { return Object.create(null); }),
        'TypeError: iterable.toString is not a function'
      );
    });

    it('should read length once and compare it numerically.', function () {
      // `i < l` is a numeric comparison against whatever `length` is, so 2.5
      // admits three iterations. Nothing rounds, and nothing validates.
      assert.deepStrictEqual(
        visited(function () {
          return {
            length: 2.5, 0: 'a', 1: 'b', 2: 'c',
            toString: function () { return '[object Arguments]'; }
          };
        }),
        [['a', 0, 'number'], ['b', 1, 'number'], ['c', 2, 'number']]
      );
    });
  });

  describe('branch 2 — delegation, which preempts 3 and 4 (D-10, D-11)', function () {

    it('should hand a Map its own forEach, so the key is a STRING key.', function () {
      // The single most load-bearing assertion in this file. A Map has a
      // Symbol.iterator and would yield [key, value] pairs with numeric
      // indices through branch 3/4 — but it owns a #.forEach, so it never
      // gets there, and the callback receives (value, key).
      assert.deepStrictEqual(
        visited(function () { return new Map([['a', 1], ['b', 2]]); }),
        [[1, 'a', 'string'], [2, 'b', 'string']]
      );
    });

    it('should hand a Set its own forEach, where the key IS the value.', function () {
      assert.deepStrictEqual(
        visited(function () { return new Set([4, 5]); }),
        [[4, 4, 'number'], [5, 5, 'number']]
      );
    });

    it('should delegate to a user-defined forEach, arguments and all.', function () {
      var received = [];

      port({
        forEach: function (callback) {
          received.push(arguments.length);
          callback('only', 'a key of my choosing', 'a third argument');
        }
      }, function (value, key, extra) {
        received.push([value, key, extra]);
      });

      assert.deepStrictEqual(received, [
        1,
        ['only', 'a key of my choosing', 'a third argument']
      ]);
    });

    it('should call the host forEach with the host as `this`.', function () {
      var host = {
        forEach: function (callback) { callback(this === host); }
      };
      var seen = null;

      port(host, function (value) { seen = value; });

      assert.strictEqual(seen, true);
    });

    it('should prefer a forEach over Symbol.iterator on the same object.', function () {
      // Both present. Branch 2 wins, so the iterator is never touched.
      var touched = false;
      var target = {
        forEach: function (callback) { callback('from forEach', 'K'); }
      };

      target[Symbol.iterator] = function () {
        touched = true;
        return {next: function () { return {done: true}; }};
      };

      var seen = [];

      port(target, function (value, key) { seen.push([value, key]); });

      assert.deepStrictEqual(seen, [['from forEach', 'K']]);
      assert.strictEqual(touched, false, 'Symbol.iterator must not be reached');
    });

    it('should propagate what the host forEach throws.', function () {
      assert.strictEqual(
        threw(function () {
          return {forEach: function () { throw new RangeError('host said no'); }};
        }),
        'RangeError: host said no'
      );
    });
  });

  describe('branch 3 — iterables coerced to iterators', function () {

    it('should call Symbol.iterator on an object that has no next.', function () {
      assert.deepStrictEqual(
        visited(function () {
          var target = {};

          target[Symbol.iterator] = function () {
            var i = 0;

            return {next: function () { return i < 2 ? {value: i++, done: false} : {done: true}; }};
          };

          return target;
        }),
        [[0, 0, 'number'], [1, 1, 'number']]
      );
    });

    it('should drive a generator.', function () {
      assert.deepStrictEqual(
        visited(function () { return (function* () { yield 'g0'; yield 'g1'; })(); }),
        [['g0', 0, 'number'], ['g1', 1, 'number']]
      );
    });

    it('should call Symbol.iterator with the target as `this`.', function () {
      var target = {marked: true};

      target[Symbol.iterator] = function () {
        var self = this,
            done = false;

        return {
          next: function () {
            if (done) return {done: true};

            done = true;

            return {value: self.marked, done: false};
          }
        };
      };

      var seen = [];

      port(target, function (value) { seen.push(value); });

      assert.deepStrictEqual(seen, [true]);
    });

    it('should skip the coercion when the target already has a next.', function () {
      // `Symbol.iterator in iterable && typeof iterable.next !== 'function'`.
      // An array iterator satisfies both halves of the first test and fails the
      // second, so it is drained as-is rather than re-derived.
      assert.deepStrictEqual(
        visited(function () { return [1, 2][Symbol.iterator](); }),
        [[1, 0, 'number'], [2, 1, 'number']]
      );
    });
  });

  describe('branch 4 — draining an iterator', function () {

    it('should pass its OWN counter, not anything the iterator supplies.', function () {
      assert.deepStrictEqual(
        visited(function () {
          var i = 0;

          return {next: function () { return i < 3 ? {value: 100 + i++, done: false} : {done: true}; }};
        }),
        [[100, 0, 'number'], [101, 1, 'number'], [102, 2, 'number']]
      );
    });

    it('should test done with !== true, so a falsy-but-not-false done keeps going.', function () {
      // `s.done !== true`, strictly. `done: 0` is not `true`, so the drain
      // continues; a port testing truthiness would stop at the first step.
      assert.deepStrictEqual(
        visited(function () {
          var i = 0;

          return {next: function () { return i < 2 ? {value: i++, done: 0} : {done: true}; }};
        }),
        [[0, 0, 'number'], [1, 1, 'number']]
      );
    });

    it('should stop only on a strict true, even from a truthy non-true done.', function () {
      var steps = 0;

      port({
        next: function () {
          steps++;

          if (steps === 1) return {value: 'a', done: 'yes'};

          return {done: true};
        }
      }, function () {});

      assert.strictEqual(steps, 2, 'done: "yes" is not done');
    });

    it('should read value even when the step object omits it.', function () {
      assert.deepStrictEqual(
        visited(function () {
          var sent = false;

          return {
            next: function () {
              if (sent) return {done: true};

              sent = true;

              return {done: false};
            }
          };
        }),
        [[undefined, 0, 'number']]
      );
    });
  });

  describe('branch 5 — plain objects (D-11, D-15)', function () {

    it('should pass the KEY as a string, not an index.', function () {
      assert.deepStrictEqual(
        visited(function () { return {a: 1, b: 2}; }),
        [[1, 'a', 'string'], [2, 'b', 'string']]
      );
    });

    it('should follow JS property enumeration order exactly.', function () {
      // Integer-like keys ascending first, then string keys in insertion order
      // — regardless of how the literal was written. Delegated to the engine
      // rather than reimplemented (D-15).
      assert.deepStrictEqual(
        visited(function () { return {b: 1, 2: 'two', a: 3, 1: 'one'}; }),
        [
          ['one', '1', 'string'],
          ['two', '2', 'string'],
          [1, 'b', 'string'],
          [3, 'a', 'string']
        ]
      );
    });

    it('should visit own properties only, skipping inherited ones.', function () {
      assert.deepStrictEqual(
        visited(function () {
          return Object.create({inherited: 1}, {own: {value: 2, enumerable: true}});
        }),
        [[2, 'own', 'string']]
      );
    });

    it('should skip non-enumerable own properties.', function () {
      assert.deepStrictEqual(
        visited(function () {
          return Object.defineProperty({visible: 1}, 'hidden', {value: 2, enumerable: false});
        }),
        [[1, 'visible', 'string']]
      );
    });

    it('should enumerate `length` like any other key, which is B-2 in miniature.', function () {
      // `{length: 5}` is not array-like enough for branch 1 and hits branch 5,
      // where `length` is simply another own property. This is the input that
      // makes `iterables.toArray` produce a sparse array upstream (NOTES B-2).
      assert.deepStrictEqual(
        visited(function () { return {length: 5}; }),
        [[5, 'length', 'string']]
      );
    });

    it('should invoke getters, once each.', function () {
      var reads = 0,
          target = {};

      Object.defineProperty(target, 'lazy', {
        enumerable: true,
        get: function () { reads++; return 'computed'; }
      });

      var seen = [];

      port(target, function (value, key) { seen.push([value, key]); });

      assert.deepStrictEqual(seen, [['computed', 'lazy']]);
      assert.strictEqual(reads, 1);
    });
  });

  describe('truthy primitives — the unguarded hole (B-30)', function () {

    it('should die in the `in` operator, not in obliterator.', function () {
      // A truthy primitive survives the falsy guard, is not an indexed
      // sequence, has no #.forEach — and then meets `Symbol.iterator in
      // iterable`, which requires an object. The error a caller sees names V8's
      // operator rather than the library that called it.
      assert.strictEqual(
        threw(function () { return 5; }),
        "TypeError: Cannot use 'in' operator to search for 'Symbol(Symbol.iterator)' in 5"
      );
      assert.strictEqual(
        threw(function () { return true; }),
        "TypeError: Cannot use 'in' operator to search for 'Symbol(Symbol.iterator)' in true"
      );
      assert.strictEqual(
        threw(function () { return 10n; }),
        "TypeError: Cannot use 'in' operator to search for 'Symbol(Symbol.iterator)' in 10"
      );
      assert.strictEqual(
        threw(function () { return Symbol('x'); }),
        "TypeError: Cannot use 'in' operator to search for 'Symbol(Symbol.iterator)' in Symbol(x)"
      );
    });
  });

  describe('values crossing the boundary', function () {

    it('should hand back the very same objects, not copies.', function () {
      // The port stores JS values as refcounted handles; a copy would break
      // strictEqual and, through Stack.from, object identity in the structure.
      var first = {},
          second = [],
          seen = [];

      port([first, second], function (value) { seen.push(value); });

      assert.strictEqual(seen[0], first);
      assert.strictEqual(seen[1], second);
    });

    it('should survive undefined, null and NaN as element values.', function () {
      assert.deepStrictEqual(
        visited(function () { return [undefined, null, NaN]; }),
        [
          [undefined, 0, 'number'],
          [null, 1, 'number'],
          [NaN, 2, 'number']
        ]
      );
    });
  });
});

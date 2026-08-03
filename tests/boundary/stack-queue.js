/**
 * Stack and Queue — boundary specs (DoD gate 7, at the boundary).
 * ===============================================================
 *
 * `cargo test` covers what the core structures do. It cannot cover what
 * happens when JavaScript *aliases* one: mutating a collection from inside its
 * own `forEach` callback, or from between two `next()` calls on a cursor it
 * still holds. The borrow checker forbids exactly that in Rust, which is the
 * point of `docs/DECISIONS.md`'s iteration section — the behaviour lives at the boundary, so the tests
 * for it do too.
 *
 * Every case here is asserted **differentially** against the vendored upstream
 * source in `bench/upstream/`, and then again explicitly, so a failure says
 * which behaviour broke rather than which two JSON blobs differ.
 *
 * Two of these specs were red when they were first written, against real bugs
 * in this port — see `stack forEach` (the callback that clears) and
 * `cursor #.return` (breaking out of a `for…of`). Both are recorded in
 * `docs/modules/stack.md`.
 */
var assert = require('assert'),
    fs = require('fs'),
    Module = require('module'),
    path = require('path');

// `bench/upstream/` is vendored source, not an installed package, so it has no
// node_modules of its own -- and `stack.js` requires obliterator at load time.
// Point Node's global resolution at the work tree's installed dependencies,
// exactly as `fuzz/oracle.js` does for the same reason.
process.env.NODE_PATH = path.resolve(__dirname, '..', 'node_modules') +
  (process.env.NODE_PATH ? path.delimiter + process.env.NODE_PATH : '');
Module._initPaths();

// The work tree is assembled at an arbitrary depth under the repository root,
// so walk up until `bench/upstream` appears rather than hard-coding `../../..`.
function repositoryRoot() {
  var directory = __dirname;

  for (var i = 0; i < 8; i++) {
    if (fs.existsSync(path.join(directory, 'bench', 'upstream', 'stack.js')))
      return directory;

    directory = path.dirname(directory);
  }

  throw new Error('cannot locate bench/upstream from ' + __dirname);
}

var UPSTREAM = path.join(repositoryRoot(), 'bench', 'upstream');

var pairs = {
  Stack: {
    upstream: require(path.join(UPSTREAM, 'stack.js')),
    port: require('@port/addon').Stack,
    add: 'push',
    take: 'pop'
  },
  Queue: {
    upstream: require(path.join(UPSTREAM, 'queue.js')),
    port: require('@port/addon').Queue,
    add: 'enqueue',
    take: 'dequeue'
  }
};

/**
 * Run `scenario` against both implementations and assert they agree.
 * Returns the agreed value so the caller can also say what it should be.
 */
function agree(name, scenario) {
  var pair = pairs[name],
      results = ['upstream', 'port'].map(function (side) {
        try {
          return {value: scenario(pair[side], pair)};
        }
        catch (error) {
          return {error: error.constructor.name + ': ' + error.message};
        }
      });

  assert.deepStrictEqual(results[1], results[0], name + ': port and upstream disagree');
  assert.ok(!results[0].error, name + ' threw: ' + results[0].error);

  return results[0].value;
}

Object.keys(pairs).forEach(function (name) {
  var add = pairs[name].add,
      take = pairs[name].take;

  describe(name + ' (boundary)', function () {

    describe('return values upstream never asserts', function () {

      it('should return the new size from #.' + add + '.', function () {
        assert.deepStrictEqual(
          agree(name, function (C) {
            var c = new C();

            return [c[add]('a'), c[add]('b'), c[add]('c')];
          }),
          [1, 2, 3]
        );
      });

      it('should return undefined from #.' + take + ' on an empty collection.', function () {
        assert.deepStrictEqual(
          agree(name, function (C) {
            var c = new C();

            return [c[take](), c.size];
          }),
          [undefined, 0]
        );
      });

      it('should render toString and toJSON like upstream.', function () {
        // `Array.prototype.join` renders null and undefined as the empty
        // string, so this is "1,,,x" and not "1,null,undefined,x".
        assert.strictEqual(
          agree(name, function (C) { return C.from([1, null, undefined, 'x']).toString(); }),
          name === 'Stack' ? 'x,,,1' : '1,,,x'
        );
        agree(name, function (C) { return JSON.stringify(C.from([1, 2, 3])); });
      });
    });

    describe('#.from — the five-branch dispatch, through a real host', function () {

      it('should accept a Map, a Set, a string, a plain object and a generator.', function () {
        agree(name, function (C) { return C.from(new Map([['a', 1], ['b', 2]])).toArray(); });
        agree(name, function (C) { return C.from(new Set([1, 2])).toArray(); });
        agree(name, function (C) { return C.from('abc').toArray(); });
        agree(name, function (C) { return C.from({x: 1, y: 2}).toArray(); });
        agree(name, function (C) {
          return C.from((function* () { yield 1; yield 2; })()).toArray();
        });
      });

      it('should throw obliterator\'s own error for a falsy iterable.', function () {
        ['', 0, false, null, undefined, NaN].forEach(function (falsy) {
          assert.throws(function () { pairs[name].port.from(falsy); },
            /obliterator\/forEach: invalid iterable\./);
        });

        agree(name, function (C) {
          try { C.from(0); } catch (error) { return error.message; }
        });
      });

      it('should route #.of through an arguments object.', function () {
        // Upstream's `of` is literally `from(arguments)`, so this is the only
        // place the original suite reaches the `[object Arguments]` clause of
        // the dispatch. The port installs the same one-liner.
        assert.deepStrictEqual(
          agree(name, function (C) { return [C.of(1, 2, 3).toArray(), C.of().toArray()]; }),
          [name === 'Stack' ? [3, 2, 1] : [1, 2, 3], []]
        );
      });

      it('should preserve object identity end to end.', function () {
        var marker = {},
            collection = pairs[name].port.from([marker]);

        assert.strictEqual(collection.toArray()[0], marker);
        assert.strictEqual(collection[take](), marker);
      });

      it('should round-trip primitives no JSON encoding survives.', function () {
        // The port stores primitives by value and rebuilds them, so this is
        // where that would show: NaN, -0, a lone surrogate, big BigInts.
        var odd = [NaN, -0, '', 'a\ud800b', 10n, -(2n ** 70n), true, null, undefined],
            collection = new pairs[name].port();

        odd.forEach(function (value) { collection[add](value); });

        var back = collection.toArray();

        if (name === 'Stack') back.reverse();

        odd.forEach(function (value, i) {
          assert.ok(
            Object.is(value, back[i]) || (typeof value === 'bigint' && value === back[i]),
            String(value) + ' came back as ' + String(back[i])
          );
        });
      });
    });

    describe('#.forEach', function () {

      it('should pass (value, index, collection), with the collection itself.', function () {
        agree(name, function (C) {
          var c = C.from([1, 2, 3]),
              seen = [];

          c.forEach(function (value, i, self) { seen.push([value, i, self === c]); });

          return seen;
        });
      });

      it('should bind `this` to the collection, or to an explicit scope.', function () {
        assert.strictEqual(agree(name, function (C) {
          var c = C.from([1]),
              bound;

          c.forEach(function () { bound = this === c; });

          return bound;
        }), true);

        assert.strictEqual(agree(name, function (C) {
          var c = C.from([1]),
              scope = {},
              bound;

          c.forEach(function () { bound = this === scope; }, scope);

          return bound;
        }), true);
      });

      it('should re-read the backing array on every iteration.', function () {
        // THIS SPEC WAS RED. Upstream freezes the loop bound but not
        // `this.items`, so a callback that rebinds the array -- `clear()` on a
        // Stack, a compacting pair of dequeues on a Queue -- changes what the
        // remaining iterations read. A `&self` on a `Freeze` type is
        // `noalias readonly` to LLVM, which hoisted that read straight out of
        // the loop and made the mutation invisible.
        agree(name, function (C, meta) {
          var c = C.from([1, 2, 3, 4]),
              seen = [];

          c.forEach(function (value, i) {
            seen.push([value, i]);

            if (i !== 0) return;

            if (meta.take === 'pop') c.clear();
            else { c[meta.take](); c[meta.take](); }
          });

          return seen;
        });
      });
    });

    describe('cursors', function () {

      it('should hand out a fresh cursor per spread, but never restart one.', function () {
        agree(name, function (C) {
          var c = C.from([1, 2]);

          return [[].concat(Array.from(c)), [].concat(Array.from(c))];
        });

        agree(name, function (C) {
          var it = C.from([1, 2]).values();

          return [Array.from(it), Array.from(it)];
        });
      });

      it('should survive a clear, because clear rebinds the array.', function () {
        // The cursor captured the array *object*; `clear()` installs a new one.
        // A Vec-backed port would have reported the walk as finished.
        agree(name, function (C) {
          var c = C.from([1, 2, 3]),
              it = c.values(),
              first = it.next();

          c.clear();

          return [c.size, first, it.next(), it.next(), it.next()];
        });
      });

      it('should keep going after a break, because upstream cursors have no #.return.', function () {
        // THIS SPEC WAS RED. `obliterator/iterator` has no `return` method, so
        // `IteratorClose` finds nothing and the cursor keeps its position.
        // napi's `#[napi(iterator)]` installs one that latches a
        // `[[GeneratorState]]` flag, which made every later next() answer
        // {done: true}. The addon deletes the method at load time.
        agree(name, function (C) {
          var it = C.from([1, 2, 3]).values(),
              taken = [];

          for (var value of it) {
            taken.push(value);
            break;
          }

          return [taken, it.next(), it.next()];
        });
      });

      it('should expose the entries cursor with its own counter.', function () {
        agree(name, function (C) { return Array.from(C.from([1, 2, 3]).entries()); });
      });
    });
  });
});

describe('Stack (boundary) — behaviours only a Stack has', function () {

  it('should freeze items.length, so a push during iteration is invisible.', function () {
    agree('Stack', function (S) {
      var s = S.from([1, 2]),
          it = s.values();

      s.push(9);

      return [it.next(), it.next(), it.next()];
    });
  });

  it('should let a pop during iteration open an undefined hole.', function () {
    // `pop()` shortens the very array the cursor captured, and the frozen
    // length still admits the missing slot: {done: false, value: undefined}.
    // This is `docs/DECISIONS.md`'s iteration section's shrink window, on a module whose test file
    // never mutates during iteration.
    var seen = agree('Stack', function (S) {
      var s = S.from([1, 2, 3]),
          it = s.values();

      s.pop();

      return [it.next(), it.next(), it.next(), it.next()];
    });

    assert.deepStrictEqual(seen[0], {value: undefined, done: false});
    assert.strictEqual(seen[3].done, true);
  });

  it('should report the same hole through the entries cursor.', function () {
    agree('Stack', function (S) {
      var s = S.from([1, 2, 3]),
          it = s.entries();

      s.pop();

      return [it.next(), it.next(), it.next(), it.next()];
    });
  });
});

describe('Queue (boundary) — behaviours only a Queue has', function () {

  it('should expose the compaction schedule through offset.', function () {
    // `++offset * 2 >= items.length` rebuilds the array. Nothing upstream
    // reads `offset` or `items.length`, so the entire schedule is untested
    // there even though it is what makes the structure O(1).
    assert.deepStrictEqual(
      agree('Queue', function (Q) {
        var q = Q.from([1, 2, 3, 4]),
            steps = [];

        for (var i = 0; i < 4; i++)
          steps.push([q.dequeue(), q.offset, q.size, q.toArray()]);

        return steps;
      }),
      [
        [1, 1, 3, [2, 3, 4]],
        [2, 0, 2, [3, 4]],
        [3, 0, 1, [4]],
        [4, 0, 0, []]
      ]
    );
  });

  it('should re-read items.length every step, so an enqueue IS visible.', function () {
    // The one line where Queue.prototype.values differs from
    // Stack.prototype.values: `if (i >= items.length)` rather than a frozen
    // `l`. Core expresses it as `Sequence::limit`.
    agree('Queue', function (Q) {
      var q = Q.from([1, 2]),
          it = q.values();

      q.enqueue(3);

      return [it.next(), it.next(), it.next(), it.next()];
    });
  });

  it('should resume a cursor that already reported done.', function () {
    // obliterator's Iterator has no done flag: it just re-runs its closure.
    assert.deepStrictEqual(
      agree('Queue', function (Q) {
        var q = Q.from([1]),
            it = q.values(),
            first = it.next(),
            finished = it.next();

        q.enqueue(2);

        return [first, finished, it.next(), it.next()];
      }),
      [
        {value: 1, done: false},
        // No `value` key at all, on either side: obliterator returns a bare
        // `{done: true}`, and so does the bridge.
        {done: true},
        {value: 2, done: false},
        {done: true}
      ]
    );
  });

  it('should detach an open cursor when a dequeue compacts.', function () {
    // The compaction installs a NEW array; the cursor keeps the old one and
    // goes on yielding elements the queue has already handed out.
    assert.deepStrictEqual(
      agree('Queue', function (Q) {
        var q = Q.from([1, 2, 3, 4]),
            it = q.values();

        q.dequeue();
        q.dequeue();

        return [q.toArray(), it.next().value, it.next().value, it.next().value, it.next().value];
      }),
      [[3, 4], 1, 2, 3, 4]
    );
  });
});

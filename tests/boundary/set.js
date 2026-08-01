/**
 * Set functions — boundary specs (DoD gate 7, at the boundary).
 * ==============================================================
 *
 * `cargo test` covers the fourteen functions over an ordinary Rust set. What
 * it cannot cover is anything about the JavaScript `Set` objects themselves,
 * and for this unit that is where most of the risk lives:
 *
 *   - The four mutating functions must mutate the CALLER'S object. `test/set.js`
 *     checks the contents afterwards, which a port that replaced the object
 *     would also pass if it were somehow able to; more usefully, it never
 *     checks *how* the object was mutated, and clear-and-rebuild is observably
 *     different from add/delete to anything already iterating it.
 *   - The returned values must be real `Set`s, not lookalikes. `Array.from`
 *     works on both.
 *   - Iteration order is asserted by eight of the fourteen upstream blocks, but
 *     always on inputs where the interesting orderings coincide. The two that
 *     do not — `intersection` following its SMALLEST argument, and ties going
 *     to the first — are unreachable from the original file.
 *   - SameValueZero membership (`NaN`, `-0`) is never exercised.
 *
 * Everything about *values* is asserted differentially against the vendored
 * upstream source in `bench/upstream/`, so a passing spec says "the port and
 * upstream agree" rather than "the port matches what I typed".
 */
var assert = require('assert'),
    fs = require('fs'),
    Module = require('module'),
    path = require('path');

process.env.NODE_PATH = path.resolve(__dirname, '..', 'node_modules') +
  (process.env.NODE_PATH ? path.delimiter + process.env.NODE_PATH : '');
Module._initPaths();

function repositoryRoot() {
  var directory = __dirname;

  for (var i = 0; i < 8; i++) {
    if (fs.existsSync(path.join(directory, 'bench', 'upstream', 'set.js')))
      return directory;

    directory = path.dirname(directory);
  }

  throw new Error('cannot locate bench/upstream from ' + __dirname);
}

var upstream = require(path.join(repositoryRoot(), 'bench', 'upstream', 'set.js')),
    port = require('../set.js');

/**
 * Run `scenario` against both implementations and assert they agree.
 * Returns the agreed value so the caller can also say what it should be.
 */
function agree(name, scenario) {
  var mine = scenario(port),
      theirs = scenario(upstream);

  assert.deepStrictEqual(mine, theirs, name + ': port and upstream disagree');

  return mine;
}

describe('set — boundary', function() {

  describe('the returned set', function() {

    it('should be a real Set, not something Array.from happens to accept.', function() {
      var A = new Set([1, 2, 3]),
          B = new Set([2, 3, 4]);

      ['intersection', 'union', 'difference', 'symmetricDifference'].forEach(function(name) {
        var result = port[name](A, B);

        assert.ok(result instanceof Set, name + ' must return a Set');
        assert.strictEqual(result.constructor, Set);
      });
    });

    it('should be a fresh set even when difference short-circuits.', function() {
      // `difference(A, B)` with an empty B returns `new Set(A)` upstream -- a
      // COPY. Returning A itself would pass every assertion in test/set.js and
      // alias two variables the caller believes are independent.
      var A = new Set([1, 2]),
          empty = new Set();

      var mine = port.difference(A, empty),
          theirs = upstream.difference(A, empty);

      assert.notStrictEqual(mine, A);
      assert.notStrictEqual(theirs, A);
      assert.deepStrictEqual(Array.from(mine), Array.from(theirs));
    });
  });

  describe('iteration order', function() {

    it('should follow the SMALLEST argument in intersection.', function() {
      // Unreachable from test/set.js: its two-set case uses equal sizes and
      // its variadic case intersects down to a single member.
      var order = agree('intersection order', function(f) {
        return Array.from(f.intersection(new Set([3, 2, 1]), new Set([1, 2])));
      });

      assert.deepStrictEqual(order, [1, 2]);
    });

    it('should break a size tie in favour of the first argument.', function() {
      var order = agree('intersection tie', function(f) {
        return Array.from(f.intersection(new Set([3, 2, 1]), new Set([1, 2, 3])));
      });

      assert.deepStrictEqual(order, [3, 2, 1]);
    });

    it('should put A\'s half before B\'s in symmetricDifference.', function() {
      var forward = agree('symdiff A,B', function(f) {
        return Array.from(f.symmetricDifference(new Set([1, 2, 3]), new Set([3, 4, 5])));
      });
      var backward = agree('symdiff B,A', function(f) {
        return Array.from(f.symmetricDifference(new Set([3, 4, 5]), new Set([1, 2, 3])));
      });

      assert.deepStrictEqual(forward, [1, 2, 4, 5]);
      assert.deepStrictEqual(backward, [4, 5, 1, 2]);
    });

    it('should add before deleting in disjunct.', function() {
      // {1,2} disjunct {2,3} is [1,3] and not [3,1] only because the addition
      // of B\A happens BEFORE the removal of the intersection.
      var order = agree('disjunct order', function(f) {
        var A = new Set([1, 2]);
        f.disjunct(A, new Set([2, 3]));
        return Array.from(A);
      });

      assert.deepStrictEqual(order, [1, 3]);
    });

    it('should not move a member that is re-added.', function() {
      // Set.add on a present member leaves it in place; only delete-then-add
      // moves it to the end. Both halves, because getting one right and the
      // other wrong is the likely mistake.
      var order = agree('re-add', function(f) {
        var A = new Set([1, 2, 3]);
        f.add(A, new Set([1]));
        return Array.from(A);
      });

      assert.deepStrictEqual(order, [1, 2, 3]);

      var moved = agree('delete then add', function(f) {
        var A = new Set([1, 2, 3]);
        f.subtract(A, new Set([1]));
        f.add(A, new Set([1]));
        return Array.from(A);
      });

      assert.deepStrictEqual(moved, [2, 3, 1]);
    });
  });

  describe('the mutating four', function() {

    ['add', 'subtract', 'intersect', 'disjunct'].forEach(function(name) {

      it('#.' + name + ' should mutate the caller\'s own object and return undefined.', function() {
        var A = new Set([1, 2]),
            before = A;

        var returned = port[name](A, new Set([2, 3]));

        assert.strictEqual(returned, undefined, name + ' returns nothing upstream');
        assert.strictEqual(A, before, name + ' must not replace the object');

        var theirs = new Set([1, 2]);
        upstream[name](theirs, new Set([2, 3]));

        assert.deepStrictEqual(Array.from(A), Array.from(theirs));
      });
    });

    it('should reach a live iterator through add/delete, not clear-and-rebuild.', function() {
      // THE spec of this file. A JS Set iterator is live: entries appended
      // after it was created are visited, and deleted ones are skipped. So a
      // bridge that emptied A and re-inserted every member would be observably
      // different here -- the iterator would see the re-inserted 1 as well --
      // while passing every assertion in test/set.js.
      var seen = agree('live iterator across add', function(f) {
        var A = new Set([1, 2]);
        var iterator = A.values();

        iterator.next();                       // consumes 1
        f.add(A, new Set([2, 3]));

        return Array.from(iterator);
      });

      assert.deepStrictEqual(seen, [2, 3]);
    });

    it('should reach a live iterator through disjunct\'s add-then-delete.', function() {
      var seen = agree('live iterator across disjunct', function(f) {
        var A = new Set([1, 2]);
        var iterator = A.values();

        iterator.next();                       // consumes 1
        f.disjunct(A, new Set([2, 3]));        // adds 3, then deletes 2

        return Array.from(iterator);
      });

      // 2 was deleted before the iterator reached it, so only 3 is left.
      assert.deepStrictEqual(seen, [3]);
    });

    it('should be defined when applied to their own argument.', function() {
      // Never done upstream, and every one of the four has a defined answer.
      // `subtract(A, A)` deletes from the set it is iterating and `disjunct`
      // both adds to and deletes from it.
      ['add', 'subtract', 'intersect', 'disjunct'].forEach(function(name) {
        var members = agree('self ' + name, function(f) {
          var A = new Set([1, 2, 3]);
          f[name](A, A);
          return Array.from(A);
        });

        assert.deepStrictEqual(
          members,
          name === 'add' || name === 'intersect' ? [1, 2, 3] : [],
          name + ' applied to itself');
      });
    });
  });

  describe('membership', function() {

    it('should treat NaN as one member, as SameValueZero does.', function() {
      var shared = agree('NaN membership', function(f) {
        return Array.from(f.intersection(new Set([NaN, 1]), new Set([NaN, 2])));
      });

      assert.strictEqual(shared.length, 1);
      assert.ok(Number.isNaN(shared[0]));
    });

    it('should treat -0 and 0 as one member, and store +0.', function() {
      var members = agree('negative zero', function(f) {
        return Array.from(f.union(new Set([-0]), new Set([0, 1])));
      });

      assert.deepStrictEqual(members, [0, 1]);
      assert.ok(!Object.is(members[0], -0), 'a Set stores -0 as +0');
    });

    it('should distinguish 1 from "1", as a Set does.', function() {
      var members = agree('no coercion', function(f) {
        return Array.from(f.union(new Set([1]), new Set(['1'])));
      });

      assert.deepStrictEqual(members, [1, '1']);
    });

    it('should reject object members with a message naming the limit.', function() {
      // A stated divergence (docs/modules/set.md): Set compares objects by
      // identity, and no identity hash for a JS object is reachable from Rust.
      // Upstream handles them; the port refuses, loudly. Asserted so that
      // silently starting to accept -- and therefore silently conflating two
      // distinct objects -- would be noticed.
      var A = new Set([{}]),
          B = new Set([{}]);

      assert.deepStrictEqual(Array.from(upstream.union(A, B)).length, 2);
      assert.throws(function() { port.union(A, B); }, /object/i);
    });
  });

  describe('arity', function() {

    it('should throw upstream\'s own message for a single argument.', function() {
      [['intersection', 'mnemonist/Set.intersection: needs at least two arguments.'],
       ['union', 'mnemonist/Set.union: needs at least two arguments.']].forEach(function(pair) {
        var name = pair[0], message = pair[1];

        [[new Set([1])], []].forEach(function(args) {
          var theirs = null, mine = null;

          try { upstream[name].apply(null, args); } catch (error) { theirs = error.message; }
          try { port[name].apply(null, args); } catch (error) { mine = error.message; }

          assert.strictEqual(theirs, message, 'upstream ' + name + '/' + args.length);
          assert.strictEqual(mine, message, 'port ' + name + '/' + args.length);
        });
      });
    });

    it('should accept more than four sets.', function() {
      var sets = [];
      for (var i = 0; i < 8; i++) sets.push(new Set([i, i + 1, 99]));

      var shared = agree('8-way intersection', function(f) {
        return Array.from(f.intersection.apply(null, sets));
      });
      var all = agree('8-way union', function(f) {
        return Array.from(f.union.apply(null, sets));
      });

      assert.deepStrictEqual(shared, [99]);
      assert.deepStrictEqual(all, [0, 1, 99, 2, 3, 4, 5, 6, 7, 8]);
    });
  });

  describe('the metrics', function() {

    it('should answer 0 rather than NaN for two empty sets.', function() {
      var answers = agree('empty metrics', function(f) {
        var empty = new Set();
        return [f.jaccard(empty, empty), f.overlap(empty, empty),
                f.intersectionSize(empty, empty), f.unionSize(empty, empty)];
      });

      assert.deepStrictEqual(answers, [0, 0, 0, 0]);
    });

    it('should agree with upstream on a set that is passed as both arguments.', function() {
      var answers = agree('self metrics', function(f) {
        var A = new Set([1, 2, 3]);
        return [f.jaccard(A, A), f.overlap(A, A),
                f.intersectionSize(A, A), f.unionSize(A, A),
                f.isSubset(A, A), f.isSuperset(A, A)];
      });

      assert.deepStrictEqual(answers, [1, 1, 3, 3, true, true]);
    });

    it('should agree with upstream on argument order for intersectionSize.', function() {
      // Upstream swaps so it walks the smaller set; the result must not depend
      // on which way round the caller passed them. test/set.js only ever
      // passes the larger first.
      var answers = agree('swapped intersectionSize', function(f) {
        var A = new Set([1, 2, 3, 4, 5]),
            B = new Set([4, 5, 6]);
        return [f.intersectionSize(A, B), f.intersectionSize(B, A),
                f.unionSize(A, B), f.unionSize(B, A),
                f.overlap(A, B), f.overlap(B, A)];
      });

      assert.deepStrictEqual(answers, [2, 2, 6, 6, 2 / 3, 2 / 3]);
    });
  });
});

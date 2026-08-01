/* eslint no-new: 0 */
//
// Heap / FixedReverseHeap — behaviours the original suite never reaches.
//
// `test/heap.js` and `test/fixed-reverse-heap.js` between them use twenty-one
// assertions, all with a total, side-effect-free comparator over numbers. Every
// case below is reachable through the same public API and none of it is tested
// upstream. Each one was run against the pinned upstream source first
// (`bench/upstream/`, Node 24.18.1) and the expectations here are what upstream
// printed -- so a failure means the port has *diverged*, in either direction.
//
// This file is the JavaScript half of gate 7. The Rust half lives in
// `crates/mnemonist-core/src/structures/{heap,fixed_reverse_heap}.rs`; what can
// only be checked here is anything involving a real JS comparator, a real JS
// array, or a real typed array.
var assert = require('assert'),
    Heap = require('../heap.js'),
    FixedReverseHeap = require('../fixed-reverse-heap.js'),
    comparators = require('../utils/comparators.js');

var MaxHeap = Heap.MaxHeap;

// Upstream's DEFAULT_COMPARATOR, written out, so a test can wrap it.
function ascending(a, b) {
  if (a < b) return -1;
  if (a > b) return 1;
  return 0;
}

function descending(a, b) {
  if (a < b) return 1;
  if (a > b) return -1;
  return 0;
}

describe('Heap (boundary) — the comparator is a callback', function() {

  it('should let a comparator mutate the heap it is comparing.', function() {
    var budget = 2;
    var heap = new Heap(function(a, b) {
      if (budget-- > 0)
        heap.push(99);

      return ascending(a, b);
    });

    heap.push(5);
    heap.push(4);
    heap.push(3);

    // NOTES B-76. Upstream has no defence against this and no error path: the
    // re-entrant pushes land in the same array the outer sift is walking, and
    // whatever is left is the answer. A port whose algorithms held an exclusive
    // borrow could not have produced this at all.
    assert.deepStrictEqual(heap.items, [3, 4, 99, 99, 5]);
    assert.strictEqual(heap.size, 5);
  });

  it('should let a comparator shorten the array, so the sift reads past its end.', function() {
    var budget = 1;
    var heap = new Heap(function(a, b) {
      if (budget-- > 0) {
        heap.items.pop();
        heap.items.pop();
      }

      return ascending(a, b);
    });

    [8, 7, 6, 5, 4, 3, 2, 1].forEach(function(value) {
      heap.push(value);
    });

    assert.deepStrictEqual(heap.items, [1, 2, 3, 5, 6, 7, 4, 8]);
    assert.strictEqual(heap.size, 8);
  });

  it('should let a comparator clear the heap, detaching the sift onto the old array.', function() {
    var cleared = false;
    var heap = new Heap(function(a, b) {
      if (!cleared) {
        cleared = true;
        heap.clear();
      }

      return ascending(a, b);
    });

    heap.push(5);
    heap.push(4);

    // `clear()` installs a NEW array (D-41), so the in-flight sift finished
    // into the detached one and `this.items` is empty -- while `++this.size`
    // still ran, on the zero the clear had just written.
    assert.deepStrictEqual(heap.items, []);
    assert.strictEqual(heap.size, 1);
  });

  it('should leave size one behind items.length when a comparator throws.', function() {
    var armed = false;
    var heap = new Heap(function(a, b) {
      if (armed)
        throw new Error('boom');

      return ascending(a, b);
    });

    heap.push(1);
    armed = true;

    // NOTES B-70. `push` grows the array BEFORE it sifts, and `++this.size`
    // never runs, so the two disagree permanently. There is no try/finally
    // anywhere in `heap.js`.
    assert.throws(function() { heap.push(2); }, /boom/);
    assert.strictEqual(heap.size, 1);
    assert.strictEqual(heap.items.length, 2);

    armed = false;
    assert.strictEqual(heap.pop(), 1);
    assert.strictEqual(heap.size, 0);
    assert.deepStrictEqual(heap.items, [2]);
  });

  it('should propagate the comparator\'s own error object, not a wrapper.', function() {
    var sentinel = new TypeError('the original');
    var heap = new Heap(function() { throw sentinel; });

    heap.push(1);

    assert.throws(function() { heap.push(2); }, function(error) {
      return error === sentinel;
    });
  });

  it('should strand the items when a comparator throws inside consume().', function() {
    var armed = false;
    var heap = new Heap(function(a, b) {
      if (armed)
        throw new Error('boom');

      return ascending(a, b);
    });

    heap.push(3);
    heap.push(1);
    heap.push(2);
    armed = true;

    // NOTES B-77: `this.size = 0` is the FIRST statement of `#.consume`, so a
    // comparator that throws leaves a heap reporting empty and holding two.
    assert.throws(function() { heap.consume(); }, /boom/);
    assert.strictEqual(heap.size, 0);
    assert.deepStrictEqual(heap.items, [3, 2]);
  });

  it('should coerce a non-numeric comparator result rather than reject it.', function() {
    // NOTES B-78. `< 0`, `> 0` and `>= 0` are all false for NaN, so a
    // comparator returning a string reports "equal" for everything.
    var nonsense = new Heap(function() { return 'x'; });

    nonsense.push(3);
    nonsense.push(1);
    nonsense.push(2);
    assert.deepStrictEqual(nonsense.toArray(), [3, 1, 2]);

    var fractional = new Heap(function() { return 0.5; });

    fractional.push(3);
    fractional.push(1);
    fractional.push(2);
    assert.deepStrictEqual(fractional.toArray(), [3, 1, 2]);
  });

  it('should accept a BigInt comparator result, which ToNumber alone would reject.', function() {
    // `Number(-1n)` works but `ToNumber(-1n)` in the relational operators does
    // not -- they use ToNumeric. So `-1n < 0` is true and the heap sorts.
    var heap = new Heap(function() { return -1n; });

    heap.push(3);
    heap.push(1);
    heap.push(2);

    assert.deepStrictEqual(heap.toArray(), [2, 1, 3]);
  });

  it('should take the default comparator for any falsy argument.', function() {
    // NOTES B-79: the guard is `comparator || DEFAULT_COMPARATOR` followed by a
    // typeof test, so `0` and `''` are accepted silently while `'test'` throws.
    assert.strictEqual(new Heap(0).size, 0);
    assert.strictEqual(new Heap('').size, 0);
    assert.throws(function() { new Heap('test'); }, /function/);
    assert.throws(function() { new Heap({}); }, /function/);
    assert.throws(function() { new MaxHeap([]); }, /function/);
  });

  it('should reverse by swapping arguments, not by negating.', function() {
    // `reverseComparator` is `comparator(b, a)`. For a comparator that is not
    // antisymmetric the two differ, and MaxHeap is built on this one.
    var constant = function() { return 1; };
    var reversed = comparators.reverseComparator(constant);

    assert.strictEqual(reversed(1, 2), 1);
    assert.strictEqual(constant(1, 2), 1);
  });
});

describe('Heap (boundary) — the raw-array statics', function() {

  it('should expose all eight statics next to the prototype methods of the same name.', function() {
    ['siftUp', 'siftDown', 'push', 'pop', 'replace', 'pushpop', 'heapify', 'consume']
      .forEach(function(name) {
        assert.strictEqual(typeof Heap[name], 'function', 'Heap.' + name);
      });

    ['push', 'pop', 'replace', 'pushpop', 'consume']
      .forEach(function(name) {
        assert.strictEqual(
          typeof Heap.prototype[name], 'function', 'Heap.prototype.' + name);
      });

    // The two are genuinely different functions with different arities.
    assert.notStrictEqual(Heap.push, Heap.prototype.push);
  });

  it('should keep the bridge\'s scaffolding off the enumerable surface.', function() {
    // The statics live on a `HeapStatics` class that the addon deletes from its
    // own exports once they have been copied across, so nothing extra is
    // reachable through `require('@port/addon')`.
    assert.strictEqual(require('@port/addon').HeapStatics, undefined);

    // `Heap.__max` and `Heap.__maxFrom` DO survive, and cannot be removed: they
    // are `#[napi(factory)]`s, and napi defines a class's own properties as
    // non-configurable, so `delete` is a no-op. This is the bridge's only
    // addition to upstream's surface. They are invisible to every enumeration
    // JavaScript offers, which is the property that matters.
    assert.strictEqual(typeof Heap.__max, 'function');
    assert.ok(Object.keys(Heap).indexOf('__max') === -1);
    assert.ok(Object.keys(Heap).indexOf('__maxFrom') === -1);
    assert.strictEqual(
      Object.getOwnPropertyDescriptor(Heap, '__max').enumerable, false);

    // The ten copied statics ARE enumerable, exactly as upstream's assignments
    // leave them.
    ['siftUp', 'siftDown', 'push', 'pop', 'replace', 'pushpop', 'heapify',
     'consume', 'nsmallest', 'nlargest', 'MinHeap', 'MaxHeap']
      .forEach(function(name) {
        assert.ok(Object.keys(Heap).indexOf(name) !== -1, name);
      });
  });

  it('should mutate the caller\'s own array in place.', function() {
    var array = [3, 5, 1, 56, 0, 13, 4];

    Heap.heapify(comparators.DEFAULT_COMPARATOR, array);

    assert.strictEqual(array[0], 0);

    Heap.push(comparators.DEFAULT_COMPARATOR, array, -7);

    assert.strictEqual(array.length, 8);
    assert.strictEqual(array[0], -7);
    assert.strictEqual(Heap.pop(comparators.DEFAULT_COMPARATOR, array), -7);

    var popped = Heap.replace(comparators.DEFAULT_COMPARATOR, array, 100);

    assert.strictEqual(popped, 0);
    assert.strictEqual(Heap.pushpop(comparators.DEFAULT_COMPARATOR, array, -1), -1);
  });

  it('should throw upstream\'s message when replacing on an empty array.', function() {
    assert.throws(function() {
      Heap.replace(comparators.DEFAULT_COMPARATOR, [], 1);
    }, /mnemonist\/heap\.replace: cannot pop an empty heap\./);
  });
});

describe('Heap (boundary) — MaxHeap shares Heap\'s prototype', function() {

  it('should make every Heap an instanceof MaxHeap, and vice versa.', function() {
    // NOTES B-75. `MaxHeap.prototype = Heap.prototype` upstream, so the two
    // constructors are indistinguishable by `instanceof`. Modelling MaxHeap as
    // its own native class would have silently corrected this.
    assert.strictEqual(MaxHeap.prototype, Heap.prototype);
    assert.ok(new Heap() instanceof MaxHeap);
    assert.ok(new MaxHeap() instanceof Heap);
    assert.strictEqual(new MaxHeap().constructor.name, 'Heap');
    assert.strictEqual(Heap.MinHeap, Heap);
  });

  it('should reverse a custom comparator rather than replacing it.', function() {
    var heap = new MaxHeap(descending);

    [3, 34, 1, 2].forEach(function(value) { heap.push(value); });

    // descending reversed is ascending.
    assert.deepStrictEqual(heap.toArray(), [1, 2, 3, 34]);
  });
});

describe('Heap (boundary) — nsmallest / nlargest', function() {

  it('should answer with the Infinity sentinel itself for an empty source.', function() {
    // NOTES B-71: `var min = Infinity` is never replaced, and the sentinel is
    // returned as though it were an element.
    assert.deepStrictEqual(Heap.nsmallest(1, []), [Infinity]);
    assert.deepStrictEqual(Heap.nlargest(1, []), [-Infinity]);
    assert.deepStrictEqual(Heap.nsmallest(1, new Set()), [Infinity]);

    // …and through a typed array the sentinel is stored, so it narrows to 0.
    var typed = Heap.nsmallest(1, new Uint8Array(0));

    assert.ok(typed instanceof Uint8Array);
    assert.deepStrictEqual(typed, new Uint8Array([0]));

    // Any other `n` takes the heap path, which has no sentinel and is correct.
    assert.deepStrictEqual(Heap.nsmallest(2, []), []);
  });

  it('should let a real Infinity element reset the sentinel.', function() {
    // NOTES B-72. Under a descending comparator the smallest of
    // `[Infinity, 5]` is `Infinity`, and the n === 1 path answers `5` because
    // `min === Infinity` is still true after `min` was set to the element.
    assert.deepStrictEqual(Heap.nsmallest(descending, 1, [Infinity, 5]), [5]);
    // The general path, one `n` up, disagrees with it.
    assert.deepStrictEqual(Heap.nsmallest(descending, 2, [Infinity, 5]), [Infinity, 5]);

    assert.deepStrictEqual(Heap.nlargest(descending, 1, [-Infinity, -5]), [-5]);
  });

  it('should preserve the source\'s class for n === 1 over an array-like.', function() {
    // `new iterable.constructor(1)`, so a typed array in is a typed array out.
    var result = Heap.nsmallest(1, new Uint8Array([9, 4, 7]));

    assert.ok(result instanceof Uint8Array);
    assert.deepStrictEqual(result, new Uint8Array([4]));
  });

  it('should not mutate the source array.', function() {
    var array = [5, 2, 4, 8, 9, 1];
    var copy = array.slice();

    Heap.nsmallest(3, array);
    Heap.nlargest(3, array);
    Heap.nsmallest(99, array);

    assert.deepStrictEqual(array, copy);
  });

  it('should correct n downwards when the iterable reports a size.', function() {
    // `guessLength` reads `.length` then `.size`; a Set has the latter.
    var set = new Set([3, 1, 2]);

    assert.deepStrictEqual(Heap.nsmallest(10, set), [1, 2, 3]);
    assert.deepStrictEqual(Heap.nlargest(10, set), [3, 2, 1]);
  });

  it('should keep the two-argument form working with a comparator first.', function() {
    var array = [2, 3, 1, 6, 4, 10, 8, 9, 7];

    assert.deepStrictEqual(Heap.nsmallest(3, array), [1, 2, 3]);
    assert.deepStrictEqual(Heap.nsmallest(ascending, 3, array), [1, 2, 3]);
    assert.deepStrictEqual(Heap.nsmallest(descending, 3, array), [10, 9, 8]);
  });
});

describe('Heap (boundary) — the default comparator on non-numbers', function() {

  it('should order mixed types the way `<` and `>` do.', function() {
    // The port answers number-vs-number and string-vs-string natively and
    // defers everything else to the engine (D-72). This is the assertion that
    // the deferral is exact: `<` on these pairs runs ToPrimitive, and the
    // resulting order is not one anybody would reproduce by hand.
    var heap = new Heap();

    [3, 'apple', true, null, undefined, {}, [2]].forEach(function(value) {
      heap.push(value);
    });

    assert.deepStrictEqual(heap.toArray(), [true, [2], {}, 3, undefined, 'apple', null]);
  });

  it('should call valueOf and toString on object elements.', function() {
    var byValueOf = new Heap();

    byValueOf.push({valueOf: function() { return 5; }});
    byValueOf.push({valueOf: function() { return 1; }});
    assert.deepStrictEqual(byValueOf.toArray().map(Number), [1, 5]);

    // Both operands ToPrimitive to strings, so `<` compares them AS strings —
    // the case a Rust ToNumber fast path would have got wrong.
    var byToString = new Heap();

    byToString.push({toString: function() { return 'b'; }});
    byToString.push({toString: function() { return 'a'; }});
    assert.deepStrictEqual(byToString.toArray().map(String), ['a', 'b']);
  });

  it('should order strings by UTF-16 code unit, and BigInts numerically.', function() {
    var strings = new Heap();

    ['pear', 'apple', 'fig'].forEach(function(value) { strings.push(value); });
    assert.deepStrictEqual(strings.toArray(), ['apple', 'fig', 'pear']);

    var bigints = new Heap();

    [3n, 1n, 2n].forEach(function(value) { bigints.push(value); });
    assert.deepStrictEqual(bigints.toArray().map(String), ['1', '2', '3']);
  });

  it('should throw when a comparator returns a Symbol, as `< 0` would.', function() {
    var heap = new Heap(function() { return Symbol('x'); });

    heap.push(1);
    assert.throws(function() { heap.push(2); }, TypeError);
  });

  it('should build from a plain object, and fail on a typed array.', function() {
    // `Heap.from` uses `iterables.isArrayLike`, which accepts a typed array —
    // and then `toArray()` calls `heap.pop()` on it, which does not exist.
    // Upstream is broken here too; both throw.
    var fromObject = Heap.from({a: 1, b: 2});

    assert.strictEqual(fromObject.size, 2);
    assert.deepStrictEqual(fromObject.toArray(), [1, 2]);

    assert.throws(function() {
      Heap.from(new Uint8Array([5, 3, 9])).toArray();
    }, /pop is not a function/);
  });
});

describe('FixedReverseHeap (boundary)', function() {

  it('should accept a capacity of 0 and then discard everything.', function() {
    // NOTES B-73: the guard is `typeof capacity !== 'number' && capacity <= 0`,
    // where `||` was meant, so it short-circuits to false for every number.
    var heap = new FixedReverseHeap(Array, 0);

    assert.strictEqual(heap.capacity, 0);
    assert.strictEqual(heap.push(1), 0);
    assert.strictEqual(heap.push(2), 0);
    assert.strictEqual(heap.size, 0);
    assert.deepStrictEqual(heap.consume(), []);
  });

  it('should die in the ArrayClass before its own guard can run.', function() {
    // `this.items = new ArrayClass(capacity)` precedes both guards.
    assert.throws(function() { new FixedReverseHeap(Array, -1); }, RangeError);
    assert.throws(function() { new FixedReverseHeap(Uint8Array, -1); }, RangeError);
  });

  it('should fire the capacity guard only for a non-number.', function() {
    assert.throws(
      function() { new FixedReverseHeap(Array, null); },
      /capacity should be a number > 0/
    );
  });

  it('should answer peek() with a discarded item after clear().', function() {
    // NOTES B-74: `clear()` is `this.size = 0` and nothing else.
    var heap = new FixedReverseHeap(Array, 3);

    heap.push(45);
    heap.push(12);
    heap.push(46);

    var stale = heap.peek();

    heap.clear();

    assert.strictEqual(heap.size, 0);
    assert.strictEqual(heap.peek(), stale);
    assert.deepStrictEqual(heap.consume(), []);
  });

  it('should peek the WORST kept item, not the best.', function() {
    var heap = new FixedReverseHeap(Array, 3);

    [4, 1, 8].forEach(function(value) { heap.push(value); });

    assert.strictEqual(heap.peek(), 8);
    assert.deepStrictEqual(heap.toArray(), [1, 4, 8]);
  });

  it('should apply typed-array store semantics to pushed values.', function() {
    var heap = new FixedReverseHeap(Uint8Array, 3);

    heap.push(300);
    heap.push(-1);
    heap.push(2.7);

    // ToUint32 then narrow: 300 -> 44, -1 -> 255, 2.7 -> 2.
    assert.deepStrictEqual(heap.consume(), new Uint8Array([2, 44, 255]));
  });

  it('should let a comparator re-enter and grow the backing array.', function() {
    var budget = 3;
    var heap = new FixedReverseHeap(Array, function(a, b) {
      if (budget-- > 0)
        heap.items.push(-1);

      return ascending(a, b);
    }, 3);

    [5, 4, 3, 2, 1].forEach(function(value) { heap.push(value); });

    assert.ok(heap.items.length > 3);
    assert.strictEqual(heap.size, 3);
  });
});

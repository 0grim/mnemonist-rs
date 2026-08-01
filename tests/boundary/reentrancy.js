/**
 * Re-entrant callbacks — boundary specs (DoD gate 7, at the boundary).
 * ====================================================================
 *
 * B-31. A `#[napi]` method taking `&self` on a type with no interior
 * mutability compiles to a `noalias readonly` pointer, so LLVM is entitled to
 * hoist reads out of the loop that calls a JS callback — and it did. A
 * `forEach` callback that mutated the collection was invisible to the walk it
 * was running inside, while the *same* object reported its new state through a
 * separate call one line later.
 *
 * Why these specs live here and not in `crates/difffuzz`
 * -----------------------------------------------------
 * The differential fuzzer compares `mnemonist-core` against upstream JS. The
 * napi bridge is not in that loop at all, so no op alphabet, however complete,
 * can reach this class of bug: the hoist happens in the layer the fuzzer skips.
 * A grammar op for a mutating callback (added alongside these specs) pins the
 * *loop shape* — live re-read versus frozen bound — which is a real and
 * separate thing to get wrong. Catching the aliasing itself needs the real
 * addon and a real JS callback, which is what this file is.
 *
 * Every case is asserted differentially against the vendored upstream source
 * in `bench/upstream/`, then again explicitly, so a failure names the
 * behaviour rather than diffing two arrays.
 *
 * The `-C opt-level` caveat: this reproduces only in an optimised build, which
 * is what `tests/run.sh` builds (`cargo build --release`). A debug addon would
 * pass these specs while still being wrong. That is a property of the bug, not
 * of the specs, and it is the reason the fix is a *type* (`RefCell`, which is
 * not `Freeze`) rather than a rearrangement of the loop.
 */
var assert = require('assert'),
    fs = require('fs'),
    Module = require('module'),
    path = require('path');

// `bench/upstream/` is vendored source with no node_modules of its own, and
// several of these files require obliterator at load time. Same fix as
// `tests/boundary/stack-queue.js` and `fuzz/oracle.js`.
process.env.NODE_PATH = path.resolve(__dirname, '..', 'node_modules') +
  (process.env.NODE_PATH ? path.delimiter + process.env.NODE_PATH : '');
Module._initPaths();

function repositoryRoot() {
  var directory = __dirname;

  for (var i = 0; i < 8; i++) {
    if (fs.existsSync(path.join(directory, 'bench', 'upstream', 'sparse-set.js')))
      return directory;

    directory = path.dirname(directory);
  }

  throw new Error('cannot locate bench/upstream from ' + __dirname);
}

var UPSTREAM = path.join(repositoryRoot(), 'bench', 'upstream'),
    port = require('@port/addon');

function upstream(name) {
  return require(path.join(UPSTREAM, name + '.js'));
}

/**
 * Run `scenario` against both implementations of one module and assert they
 * agree. Returns what they agreed on, so the caller can additionally say what
 * it should have been — a scenario that is wrong on BOTH sides would otherwise
 * pass silently.
 */
function agree(name, scenario) {
  var expected = scenario(upstream(name)),
      actual = scenario(port[constructorName(name)]);

  assert.deepStrictEqual(actual, expected,
    name + ': the port and upstream disagree under a mutating callback');

  return expected;
}

function constructorName(name) {
  return name.split('-').map(function (word) {
    return word.charAt(0).toUpperCase() + word.slice(1);
  }).join('');
}

describe('re-entrant forEach callbacks (B-31)', function () {

  describe('SparseSet', function () {

    it('should shorten the walk when the callback deletes, because the bound is live.', function () {
      var seen = agree('sparse-set', function (SparseSet) {
        var s = new SparseSet(8), out = [];

        [1, 2, 3, 4].forEach(function (m) { s.add(m); });
        s.forEach(function (m) { out.push(m); s.delete(m); });

        return out;
      });

      // Upstream's `for (i = 0; i < this.size; i++)` re-reads `size`, and
      // `delete` swaps the last member into the freed slot -- so deleting
      // every visited member visits half of them, not all four.
      assert.deepStrictEqual(seen, [1, 2]);
    });

    it('should stop immediately when the callback clears.', function () {
      var seen = agree('sparse-set', function (SparseSet) {
        var s = new SparseSet(8), out = [];

        [0, 1, 2, 3, 4].forEach(function (m) { s.add(m); });
        s.forEach(function (m) { out.push(m); s.clear(); });

        return out;
      });

      assert.deepStrictEqual(seen, [0]);
    });

    it('should visit members the callback adds, because the bound grows too.', function () {
      var seen = agree('sparse-set', function (SparseSet) {
        var s = new SparseSet(8), out = [];

        s.add(0);
        s.forEach(function (m) { out.push(m); if (m < 3) s.add(m + 1); });

        return out;
      });

      assert.deepStrictEqual(seen, [0, 1, 2, 3]);
    });

    it('should let a cursor see writes the callback made between steps.', function () {
      var seen = agree('sparse-set', function (SparseSet) {
        var s = new SparseSet(8), out = [];

        [1, 2, 3].forEach(function (m) { s.add(m); });

        var it = s.values();

        out.push(it.next().value);
        s.delete(2);
        out.push(it.next().value);
        out.push(it.next().value);

        return out;
      });

      // `delete(2)` swaps the last member into slot 1, so the second step
      // yields 3 -- and the frozen size still admits a third step, which reads
      // the stale slot 3 was swapped out of and yields it a second time.
      assert.deepStrictEqual(seen, [1, 3, 3]);
    });
  });

  describe('SparseQueueSet', function () {

    it('should NOT shorten the walk when the callback dequeues, because the bound is frozen.', function () {
      var seen = agree('sparse-queue-set', function (SparseQueueSet) {
        var q = new SparseQueueSet(8), out = [];

        [1, 2, 3, 4].forEach(function (m) { q.enqueue(m); });
        q.forEach(function (m) { out.push(m); q.dequeue(); });

        return out;
      });

      // Upstream captures `l = this.size` before the loop, so unlike
      // SparseSet all four steps run -- reading through a ring whose `start`
      // the callback has been advancing.
      assert.strictEqual(seen.length, 4);
    });

    it('should see members the callback enqueues, through the live dense array.', function () {
      var seen = agree('sparse-queue-set', function (SparseQueueSet) {
        var q = new SparseQueueSet(8), out = [];

        [1, 2].forEach(function (m) { q.enqueue(m); });
        q.forEach(function (m) { out.push(m); if (m === 1) q.enqueue(5); });

        return out;
      });

      assert.strictEqual(seen.length, 2);
    });
  });

  describe('SparseMap', function () {

    it('should shorten the walk when the callback deletes, because the bound is live.', function () {
      var seen = agree('sparse-map', function (SparseMap) {
        var m = new SparseMap(8), out = [];

        [1, 2, 3, 4].forEach(function (k) { m.set(k, k * 10); });
        m.forEach(function (value, key) { out.push([value, key]); m.delete(key); });

        return out;
      });

      // `delete` swaps the last entry into the freed slot, so deleting every
      // visited entry visits half of them -- and the second visit sees the
      // pair that was originally at index 1, not the one swapped in.
      assert.deepStrictEqual(seen, [[10, 1], [20, 2]]);
    });

    it('should stop immediately when the callback clears.', function () {
      var seen = agree('sparse-map', function (SparseMap) {
        var m = new SparseMap(8), out = [];

        [1, 2, 3].forEach(function (k) { m.set(k, k); });
        m.forEach(function (value, key) { out.push(key); m.clear(); });

        return out;
      });

      assert.deepStrictEqual(seen, [1]);
    });
  });

  describe('BitSet', function () {

    it('should snapshot the word, so a write to the word being walked is invisible.', function () {
      var seen = agree('bit-set', function (BitSet) {
        var b = new BitSet(8), out = [];

        b.set(0);
        b.forEach(function (bit, i) { out.push(bit); if (i === 0) b.set(4); });

        return out;
      });

      // Upstream lifts `byte = this.array[i]` out of the inner loop, so the
      // callback's `set(4)` does not appear in this walk.
      assert.deepStrictEqual(seen, [1, 0, 0, 0, 0, 0, 0, 0]);
    });

    it('should still report the write through the object afterwards.', function () {
      var after = agree('bit-set', function (BitSet) {
        var b = new BitSet(8);

        b.set(0);
        b.forEach(function (bit, i) { if (i === 0) b.set(4); });

        return [b.get(4), b.size];
      });

      // The half that made B-31 visible: the walk did not see the write, but
      // the object must -- and a hoisted read would have hidden it here too.
      assert.deepStrictEqual(after, [1, 2]);
    });

    it('should observe a clear from inside its own callback.', function () {
      var after = agree('bit-set', function (BitSet) {
        var b = new BitSet(40), seenSize = [];

        b.set(0);
        b.set(35);
        b.forEach(function (bit, i) {
          if (i === 0) b.clear();
          if (i === 32) seenSize.push(b.get(35));
        });

        return [seenSize, b.size];
      });

      // Word 1 is re-read after the callback cleared it, so bit 35 is gone by
      // the time the walk reaches it.
      assert.deepStrictEqual(after, [[0], 0]);
    });
  });

  describe('BitVector', function () {

    it('should snapshot the word, so a write to the word being walked is invisible.', function () {
      var seen = agree('bit-vector', function (BitVector) {
        var v = new BitVector(8), out = [];

        v.set(0);
        v.forEach(function (bit, i) { out.push(bit); if (i === 0) v.set(4); });

        return out;
      });

      assert.deepStrictEqual(seen, [1, 0, 0, 0, 0, 0, 0, 0]);
    });

    it('should still report the write through the object afterwards.', function () {
      var after = agree('bit-vector', function (BitVector) {
        var v = new BitVector(8);

        v.set(0);
        v.forEach(function (bit, i) { if (i === 0) v.set(4); });

        return [v.get(4), v.size, v.length];
      });

      assert.deepStrictEqual(after, [1, 2, 8]);
    });

    it('should observe a push from inside its own callback.', function () {
      var after = agree('bit-vector', function (BitVector) {
        var v = new BitVector(4), lengths = [];

        v.set(0);
        v.forEach(function (bit, i) { if (i === 0) v.push(1); lengths.push(v.length); });

        return lengths;
      });

      // `length` is read through the object on every step. A hoisted read
      // would report 4 forever.
      assert.deepStrictEqual(after, [5, 5, 5, 5]);
    });
  });

  describe('DefaultMap', function () {

    it('should visit an entry the callback adds, because a Map walk is live.', function () {
      var seen = agree('default-map', function (DefaultMap) {
        var m = new DefaultMap(function () { return 0; }), out = [];

        m.set('a', 1);
        m.forEach(function (value, key) {
          out.push([key, value]);
          if (key === 'a') m.set('b', 2);
        });

        return out;
      });

      assert.deepStrictEqual(seen, [['a', 1], ['b', 2]]);
    });

    it('should skip an entry the callback deletes ahead of the cursor.', function () {
      var seen = agree('default-map', function (DefaultMap) {
        var m = new DefaultMap(function () { return 0; }), out = [];

        m.set('a', 1);
        m.set('b', 2);
        m.set('c', 3);
        m.forEach(function (value, key) {
          out.push(key);
          if (key === 'a') m.delete('b');
        });

        return out;
      });

      assert.deepStrictEqual(seen, ['a', 'c']);
    });

    it('should report the size the callback changed, on the next step.', function () {
      var seen = agree('default-map', function (DefaultMap) {
        var m = new DefaultMap(function () { return 0; }), sizes = [];

        m.set('a', 1);
        m.set('b', 2);
        m.forEach(function (value, key) {
          if (key === 'a') m.set('c', 3);
          sizes.push(m.size);
        });

        return sizes;
      });

      assert.deepStrictEqual(seen, [3, 3, 3]);
    });
  });

  describe('re-entry through something other than a forEach callback', function () {

    // These two are the cases a forEach-shaped fix misses, and both were
    // found the same way: by asking "what ELSE can run JavaScript while a
    // `&self` method is on the stack?" A `RefCell` borrow alive across such a
    // call does not degrade — a RefCell panic inside a `#[napi]` method
    // ABORTS THE PROCESS, because napi 3.12 does not catch_unwind a sync call
    // and a panic unwinding out of an `extern "C"` frame is an abort.
    // Both aborted before the fix that followed.

    it('should let a DefaultMap factory read the map it is inserting into.', function () {
      var seen = agree('default-map', function (DefaultMap) {
        var sizes = [];
        var m = new DefaultMap(function () { return m.size; });

        m.get('a');
        m.get('b');
        sizes.push(m.peek('a'), m.peek('b'), m.size);

        return sizes;
      });

      // Upstream's factory runs BETWEEN the read and the write, so the map is
      // in its pre-insert state and `this.size` is the count before the bump.
      assert.deepStrictEqual(seen, [0, 1, 2]);
    });

    it('should let a DefaultMap factory write to the map it is inserting into.', function () {
      var seen = agree('default-map', function (DefaultMap) {
        var m = new DefaultMap(function (key) {
          if (key === 'a') m.set('side', 'effect');
          return key;
        });

        m.get('a');

        return [m.peek('a'), m.peek('side'), m.size];
      });

      assert.deepStrictEqual(seen, ['a', 'effect', 2]);
    });

    it('should refuse — catchably, not fatally — a BitVector policy that re-enters.', function () {
      // The one place the port cannot follow upstream, and it is stated in
      // `crates/mnemonist-napi/src/bit_vector.rs` and in the module doc: the
      // growth policy is JavaScript called from INSIDE mnemonist-core's
      // `grow`, so the vector is genuinely mid-operation and the borrow
      // cannot be released around it.
      //
      // What this spec pins is that refusing is a catchable `Error` and not a
      // process abort. It is deliberately NOT differential: upstream succeeds
      // here and the port does not.
      var BitVector = port.BitVector,
          vector;

      vector = new BitVector({
        initialLength: 4,
        policy: function (capacity) { return capacity * 2 + vector.length * 0; }
      });

      assert.throws(function () {
        for (var i = 0; i < 200; i++) vector.push(1);
      }, /growth policy called back into the vector/);

      // Still usable afterwards: the borrow was released, not poisoned.
      assert.strictEqual(typeof vector.length, 'number');
    });

    it('should run a non-re-entrant BitVector policy exactly as upstream does.', function () {
      var grown = agree('bit-vector', function (BitVector) {
        var v = new BitVector({
          initialLength: 4,
          policy: function (capacity) { return capacity + 64; }
        });

        for (var i = 0; i < 200; i++) v.push(1);

        return [v.length, v.capacity, v.size];
      });

      assert.strictEqual(grown[0], 204);
    });
  });

  describe('Stack and Queue — already correct, kept as the control', function () {

    it('should keep Stack and Queue behaving as they did before the fix.', function () {
      var stack = agree('stack', function (Stack) {
        var s = new Stack(), out = [];

        [1, 2, 3, 4].forEach(function (v) { s.push(v); });
        s.forEach(function (v) { out.push(v); s.pop(); });

        return out;
      });

      // These two bridges already held a `RefCell`; the specs are here so a
      // future edit that removes it fails in this file rather than silently.
      assert.strictEqual(stack.length, 4);

      var queue = agree('queue', function (Queue) {
        var q = new Queue(), out = [];

        [1, 2, 3, 4].forEach(function (v) { q.enqueue(v); });
        q.forEach(function (v, i) { out.push(v); if (i === 0) { q.dequeue(); q.dequeue(); } });

        return out;
      });

      assert.strictEqual(queue.length, 4);
    });
  });
});

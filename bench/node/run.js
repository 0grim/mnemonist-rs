#!/usr/bin/env node
//
// Node half of the matched benchmark harness (DESIGN.md 5.1-5.2).
//
// This file is a deliberate mirror of bench/runner/src/*.rs, line of reasoning
// for line of reasoning: same PRNG, same three-draws-per-op discipline, same
// materialise-before-timing rule, same K = 1000 batching, same warmup and
// measured counts, same JSON envelope. DESIGN.md 5.2 Problem 1 is the reason -
// two different measurement methodologies in one comparison table invalidate
// every row of it, so neither side gets a tool the other cannot have.
//
// It benchmarks bench/upstream/, the JS vendored from the same clone and
// commit as the hashed tests (12c.2 point 1). Not `npm install mnemonist`:
// the tests were hashed from master, which may sit ahead of the released
// tarball, and comparing against a different codebase than the one the parity
// tests pin would be quietly wrong.
//
// Modes:
//   node run.js --module static-disjoint-set --warmup 3 --measured 1
//   node run.js --module sparse-set --kind drain --passes 100
//   node run.js --baseline        # no-op run, reports peak RSS only
//   node run.js --noop            # startup floor, for hyperfine
//   node run.js --dump-prng 1000  # matched-PRNG proof
'use strict';

const Module = require('module');
const fs = require('fs');
const path = require('path');

// From sparse-set.js onwards the vendored upstream files require obliterator at
// load time, and bench/upstream/ is vendored source with no node_modules of its
// own. Same fix, and same reasoning, as fuzz/oracle.js.
const HARNESS_MODULES = path.resolve(__dirname, '..', '..', 'tests', '.work', 'node_modules');

if (fs.existsSync(HARNESS_MODULES)) {
  process.env.NODE_PATH = process.env.NODE_PATH
    ? HARNESS_MODULES + path.delimiter + process.env.NODE_PATH
    : HARNESS_MODULES;
  Module._initPaths();
}

const BATCH_K = 1000;
const DEFAULT_SIZE = 1000000;
const DEFAULT_OPS = 1000000;
const DEFAULT_SEED = 42;

// One `kind % 4` stream serves every module; each names the four values for
// its own alphabet. Twin of bench/runner/src/workload.rs.
const UNION_A = 0;
const UNION_B = 1;
const FIND = 2;

const ADD_A = 0;
const ADD_B = 1;
const HAS = 2;

// Twin of DEFAULT_PASSES in bench/runner/src/main.rs.
const DEFAULT_PASSES = 100;

const MODULES = {
  'static-disjoint-set': ['mixed'],
  'sparse-set': ['mixed', 'drain'],
  'bit-set': ['mixed'],
  'lru-cache': ['mixed'],
  'heap': ['mixed'],
  'trie': ['mixed'],
  'vector': ['mixed'],
  // Appended for the sequence-backed batch, never inserted: twin of
  // harness.rs::MODULES, which documents why a new module is always an
  // appended entry rather than a reordered table.
  'stack': ['mixed'],
  'queue': ['mixed'],
  'fixed-stack': ['mixed'],
  'fixed-deque': ['mixed'],
  'circular-buffer': ['mixed'],
  'hashed-array-tree': ['mixed'],
  'sparse-map': ['mixed'],
  'sparse-queue-set': ['mixed'],
  'bit-vector': ['mixed'],
  // No `mixed` kind: see bench/runner/src/sort.rs and suffix_array.rs for why
  // both reuse `drain`'s one-sample-per-operation shape instead.
  'suffix-array': ['drain'],
  'sort': ['drain'],
  // Appended for the map-like/multi-container Gate 10 batch, never inserted:
  // twin of harness.rs::MODULES, which documents why a new module is always
  // an appended entry rather than a reordered table.
  'default-map': ['mixed'],
  'bi-map': ['mixed'],
  'multi-map': ['mixed'],
  'multi-set': ['mixed'],
  'multi-array': ['mixed'],
  'fuzzy-map': ['mixed'],
  'fuzzy-multi-map': ['mixed'],
  'inverted-index': ['mixed'],
  // No `mixed` kind: see bench/runner/src/set_ops.rs for why this reuses
  // `drain`'s one-sample-per-call shape instead.
  'set': ['drain'],
  // Appended for the final Gate 10 batch (the last fourteen units), never
  // inserted: twin of harness.rs::MODULES.
  'trie-map': ['mixed'],
  'critbit-tree-map': ['mixed'],
  'fixed-critbit-tree-map': ['mixed'],
  'bk-tree': ['mixed'],
  'vp-tree': ['mixed'],
  'kd-tree': ['mixed'],
  'static-interval-tree': ['mixed'],
  'fibonacci-heap': ['mixed'],
  'fixed-reverse-heap': ['mixed'],
  'bloom-filter': ['mixed'],
  'linked-list': ['mixed']
};

const argv = process.argv.slice(2);

// --- matched PRNG -----------------------------------------------------------
//
// Twin of bench/runner/src/xorshift.rs. The `>>> 0` after each step is what
// makes JS agree with a Rust u32: JS bitwise operators produce *signed* 32-bit
// results, so without it `x` goes negative and the streams part company within
// a handful of draws.
function XorShift32(seed) {
  if (seed === 0) throw new Error('xorshift32 cannot be seeded with zero');
  this.state = seed >>> 0;
}

XorShift32.prototype.next = function () {
  let x = this.state;

  x ^= (x << 13);
  x >>>= 0;
  x ^= (x >>> 17);
  x ^= (x << 5);
  x >>>= 0;

  this.state = x;

  return x;
};

// Plain modulo, bias included, because the Rust side does the same and
// rejection sampling would consume a data-dependent number of draws.
XorShift32.prototype.below = function (bound) {
  return this.next() % bound;
};

// --- workload ---------------------------------------------------------------

// Three draws per op regardless of whether the op uses the second operand.
// A conditional third draw desynchronises the two sides at the first `find`.
function generate(size, ops, seed) {
  const rng = new XorShift32(seed);
  const kind = new Uint8Array(ops);
  const a = new Uint32Array(ops);
  const b = new Uint32Array(ops);

  for (let i = 0; i < ops; i++) {
    kind[i] = rng.next() % 4;
    a[i] = rng.below(size);
    b[i] = rng.below(size);
  }

  return {size: size, kind: kind, a: a, b: b};
}

function runOnce(StaticDisjointSet, workload, k) {
  // Fresh set per pass, outside the timed region: reusing one would make pass
  // 2 onwards measure an already-merged forest.
  const set = new StaticDisjointSet(workload.size);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const x = workload.a[i];
      const y = workload.b[i];
      const op = workload.kind[i];

      if (op === UNION_A || op === UNION_B) {
        set.union(x, y);
      } else if (op === FIND) {
        checksum += set.find(x);
      } else {
        checksum += set.connected(x, y) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: set};
}

// Twin of bench/runner/src/sparse_set.rs `run_mixed`: 50% add, 25% has,
// 25% delete, members drawn in range.
function runMixedSparse(SparseSet, workload, k) {
  const set = new SparseSet(workload.size);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const member = workload.a[i];
      const op = workload.kind[i];

      if (op === ADD_A || op === ADD_B) {
        set.add(member);
      } else if (op === HAS) {
        checksum += set.has(member) ? 1 : 0;
      } else {
        checksum += set.delete(member) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: set};
}

// Twin of bench/runner/src/bit_set.rs. 50% set/reset (mutating), 25% get,
// 25% test (both pure O(1) reads). `rank` was tried and pulled -- see that
// file's module docs: it has no rank/select index on either side, so a
// single call costs O(i / 32) words, and a 25%-weighted mix over a 1e6 domain
// made the harness spend ten-plus minutes computing six of ten reps instead
// of measuring a representative bit-set workload.
function runMixedBitSet(BitSet, workload, k) {
  const set = new BitSet(workload.size);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const index = workload.a[i];
      const op = workload.kind[i];

      if (op === 0) {
        set.set(index);
      } else if (op === 1) {
        set.reset(index);
      } else if (op === 2) {
        checksum += set.get(index);
      } else {
        checksum += set.test(index) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: set};
}

// Twin of bench/runner/src/lru_cache.rs::capacity_for. Capacity is a fixed
// fraction (20%) of the key domain `workload.size` provides, never the domain
// itself -- see that file's module docs for why capacity == domain (a 100%
// hit rate once warmed) and a tiny fixed capacity are both the wrong answer.
function capacityForLru(domain) {
  return Math.max(1, Math.floor(domain / 5));
}

// Twin of bench/runner/src/lru_cache.rs. 50% set (mutating), 25% get
// (mutating -- splays to front -- and read), 25% has (pure read).
function runMixedLru(LRUCache, workload, k) {
  const cache = new LRUCache(capacityForLru(workload.size));
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const key = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        cache.set(key, key);
      } else if (op === 2) {
        const value = cache.get(key);
        checksum += value === undefined ? 0 : value;
      } else {
        checksum += cache.has(key) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: cache};
}

// Twin of bench/runner/src/heap.rs. Default (numeric) comparator, matching
// core's DefaultComparator. 50% push (mutating), 25% pop (mutating and read,
// the same shape as static-disjoint-set's `find`), 25% peek (pure read).
// pop/peek on an empty heap return `undefined`, contributing 0 -- no guard
// needed, since 50% push against 25% pop keeps the heap non-empty almost
// throughout.
function runMixedHeap(Heap, workload, k) {
  const heap = new Heap();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const value = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        heap.push(value);
      } else if (op === 2) {
        const popped = heap.pop();
        checksum += popped === undefined ? 0 : popped;
      } else {
        const peeked = heap.peek();
        checksum += peeked === undefined ? 0 : peeked;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: heap};
}

// Twin of bench/runner/src/trie.rs. Keys are `value.toString(16)` -- lowercase
// hex, no leading zeros, byte-identical to Rust's `format!("{value:x}")` for
// the same u32, so no second matched generator is needed and prefix-sharing
// among nearby values comes for free. Same 50/25/25 add/has/delete shape as
// sparse-set.
function runMixedTrie(Trie, workload, k) {
  const trie = new Trie();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const word = workload.a[i].toString(16);
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        trie.add(word);
      } else if (op === 2) {
        checksum += trie.has(word) ? 1 : 0;
      } else {
        checksum += trie.delete(word) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: trie};
}

// Twin of bench/runner/src/vector.rs. 50% push (mutating growth), 25% get at
// a uniformly random *existing* index (pure read, modulo the current length
// so it never lands past it), 25% pop (mutating and read). Both sides derive
// the same push/pop counts from the same matched stream, so the vector's
// length trajectory -- and what `get`'s modulo lands on -- is identical; the
// checksum proves it.
function runMixedVector(Vector, workload, k) {
  const vector = new Vector(Float64Array, 0);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        vector.push(workload.a[i]);
      } else if (op === 2) {
        const len = vector.length;

        if (len > 0) {
          const index = workload.a[i] % len;
          checksum += vector.get(index);
        }
      } else {
        const popped = vector.pop();
        checksum += popped === undefined ? 0 : popped;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: vector};
}

// Twin of bench/runner/src/stack.rs. 50% push (mutating growth), 25% peek
// (pure read), 25% pop (mutating and a read) -- `vector`'s shape, with `peek`
// standing in for `get` because a stack exposes no random access.
function runMixedStack(Stack, workload, k) {
  const stack = new Stack();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        stack.push(workload.a[i]);
      } else if (op === 2) {
        const peeked = stack.peek();
        checksum += peeked === undefined ? 0 : peeked;
      } else {
        const popped = stack.pop();
        checksum += popped === undefined ? 0 : popped;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: stack};
}

// Twin of bench/runner/src/queue.rs. `stack`'s exact shape with FIFO names.
function runMixedQueue(Queue, workload, k) {
  const queue = new Queue();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        queue.enqueue(workload.a[i]);
      } else if (op === 2) {
        const peeked = queue.peek();
        checksum += peeked === undefined ? 0 : peeked;
      } else {
        const popped = queue.dequeue();
        checksum += popped === undefined ? 0 : popped;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: queue};
}

// Twin of bench/runner/src/fixed_stack.rs. `workload.size` is the capacity
// directly. `push` is guarded by `size < capacity` so the timed loop never
// calls the fallible path -- an unguarded push into a full stack would
// benchmark V8's `Error` construction (stack-trace capture) rather than the
// stack; see that file's module docs for the full account.
function runMixedFixedStack(FixedStack, workload, k) {
  const stack = new FixedStack(Float64Array, workload.size);
  const capacity = workload.size;
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        if (stack.size < capacity) stack.push(workload.a[i]);
      } else if (op === 2) {
        const peeked = stack.peek();
        checksum += peeked === undefined ? 0 : peeked;
      } else {
        const popped = stack.pop();
        checksum += popped === undefined ? 0 : popped;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: stack};
}

// Twin of bench/runner/src/fixed_deque.rs. Back-end ops only (push/peekLast/
// pop), mirroring `fixed-stack`'s shape; same capacity guard, same reason.
function runMixedFixedDeque(FixedDeque, workload, k) {
  const deque = new FixedDeque(Float64Array, workload.size);
  const capacity = workload.size;
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        if (deque.size < capacity) deque.push(workload.a[i]);
      } else if (op === 2) {
        const peeked = deque.peekLast();
        checksum += peeked === undefined ? 0 : peeked;
      } else {
        const popped = deque.pop();
        checksum += popped === undefined ? 0 : popped;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: deque};
}

// Twin of bench/runner/src/circular_buffer.rs. Same shape as `fixed-deque`,
// but `push` is never guarded: it cannot fail, which is this module's whole
// reason to exist.
function runMixedCircularBuffer(CircularBuffer, workload, k) {
  const buffer = new CircularBuffer(Float64Array, workload.size);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        buffer.push(workload.a[i]);
      } else if (op === 2) {
        const peeked = buffer.peekLast();
        checksum += peeked === undefined ? 0 : peeked;
      } else {
        const popped = buffer.pop();
        checksum += popped === undefined ? 0 : popped;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: buffer};
}

// Twin of bench/runner/src/hashed_array_tree.rs. `vector`'s shape: 50% push
// (growth), 25% get at a random existing index (modulo the current length),
// 25% pop. No options object: the default block size, matching
// `Options::default()`.
function runMixedHashedArrayTree(HashedArrayTree, workload, k) {
  const tree = new HashedArrayTree(Uint32Array);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        tree.push(workload.a[i]);
      } else if (op === 2) {
        const len = tree.length;

        if (len > 0) {
          const index = workload.a[i] % len;
          checksum += tree.get(index);
        }
      } else {
        const popped = tree.pop();
        checksum += popped === undefined ? 0 : popped;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: tree};
}

// Twin of bench/runner/src/sparse_map.rs. `sparse-set`'s add/has/delete
// shape, with `set` taking `workload.b[i]` as the value.
function runMixedSparseMap(SparseMap, workload, k) {
  const map = new SparseMap(workload.size);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const member = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        map.set(member, workload.b[i]);
      } else if (op === 2) {
        const value = map.get(member);
        checksum += value === undefined ? 0 : value;
      } else {
        checksum += map.delete(member) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: map};
}

// Twin of bench/runner/src/sparse_queue_set.rs. `sparse-set`'s shape with
// FIFO names; `dequeue` takes no operand.
function runMixedSparseQueueSet(SparseQueueSet, workload, k) {
  const queue = new SparseQueueSet(workload.size);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const member = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        queue.enqueue(member);
      } else if (op === 2) {
        checksum += queue.has(member) ? 1 : 0;
      } else {
        const dequeued = queue.dequeue();
        checksum += dequeued === undefined ? 0 : dequeued;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: queue};
}

// Twin of bench/runner/src/bit_vector.rs. `vector`/`hashed-array-tree`'s
// shape: 50% push (growth), 25% get at a random existing index, 25% pop.
// `rank`/`select` excluded -- see `bit_vector.rs`'s module docs, which
// inherits `bit-set`'s `rank` lesson.
function runMixedBitVector(BitVector, workload, k) {
  const vector = new BitVector(0);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        vector.push(workload.a[i] % 2 === 1);
      } else if (op === 2) {
        const len = vector.length;

        if (len > 0) {
          const index = workload.a[i] % len;
          checksum += vector.get(index);
        }
      } else {
        const popped = vector.pop();
        checksum += popped === undefined ? 0 : popped;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: vector};
}

// Twin of bench/runner/src/default_map.rs. 50% set (mutating), 25%
// get-or-insert (mutating and a read -- the factory always returns a
// defined value, so this never triggers B-40's `size` drift), 25% delete.
function runMixedDefaultMap(DefaultMap, workload, k) {
  const map = new DefaultMap(function(key) { return key; });
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const key = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        map.set(key, workload.b[i]);
      } else if (op === 2) {
        checksum += map.get(key);
      } else {
        checksum += map.delete(key) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: map};
}

// Twin of bench/runner/src/bi_map.rs. 50% set (mutating -- the four-branch
// constraint resolution), 25% get (pure read), 25% delete. Key and value
// share the same `0..size` domain, deliberately -- see that file's own
// module docs for why.
function runMixedBiMap(BiMap, workload, k) {
  const map = new BiMap();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const key = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        map.set(key, workload.b[i]);
      } else if (op === 2) {
        const value = map.get(key);
        checksum += value === undefined ? 0 : value;
      } else {
        checksum += map.delete(key) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: map};
}

// Twin of bench/runner/src/multi_map.rs. 50% set (mutating append, default
// Array container), 25% get (pure read, contributing the bucket's length),
// 25% remove (mutating). `workload.size` is the KEY DOMAIN, deliberately far
// smaller than the op count -- see that file's own module docs for the
// values-per-key reasoning.
function runMixedMultiMap(MultiMap, workload, k) {
  const map = new MultiMap();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const key = workload.a[i];
      const value = workload.b[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        map.set(key, value);
      } else if (op === 2) {
        const container = map.get(key);
        checksum += container === undefined ? 0 : container.length;
      } else {
        checksum += map.remove(key, value) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: map};
}

// Twin of bench/runner/src/multi_set.rs. 50% add (mutating), 25%
// multiplicity (pure read), 25% remove (mutating, no contribution --
// `MultiSet#.remove` returns nothing, matching `#.set`'s convention
// elsewhere in this batch). `delete`/`set` are excluded on purpose -- see
// that file's own module docs on B-160/B-161.
function runMixedMultiSet(MultiSet, workload, k) {
  const set = new MultiSet();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const item = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        set.add(item, 1);
      } else if (op === 2) {
        checksum += set.multiplicity(item);
      } else {
        set.remove(item, 1);
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: set};
}

// Twin of bench/runner/src/multi_array.rs. 50% set (mutating append,
// dynamic/unbounded mode -- `test/multi-array.js` never builds a
// fixed-capacity `Array` container), 25% get (a read that materialises the
// whole bucket, contributing its length), 25% multiplicity (a pure O(1)
// read). No delete: this module has none, upstream or here.
function runMixedMultiArray(MultiArray, workload, k) {
  const array = new MultiArray();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const index = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        array.set(index, workload.b[i]);
      } else if (op === 2) {
        const bucket = array.get(index);
        checksum += bucket === undefined ? 0 : bucket.length;
      } else {
        checksum += array.multiplicity(index);
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: array};
}

// `hash(x) = x >>> 4`, twin of bench/runner/src/fuzzy_map.rs::hash and
// fuzzy_multi_map.rs::hash -- has to do the IDENTICAL work on both sides
// (see fuzzy_map.rs's own module docs), which is exactly why this is a bare
// arithmetic shift rather than anything floating-point-based.
function fuzzyHash(x) {
  return x >>> 4;
}

// Twin of bench/runner/src/fuzzy_map.rs. 50% set (mutating, hashed
// internally by the class), 25% get (pure read), 25% has (pure read --
// stands in for `sparse-map`'s delete slot, since this module has no
// delete; see that file's own module docs).
function runMixedFuzzyMap(FuzzyMap, workload, k) {
  const map = new FuzzyMap(fuzzyHash);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const key = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        map.set(key, workload.b[i]);
      } else if (op === 2) {
        const value = map.get(key);
        checksum += value === undefined ? 0 : value;
      } else {
        checksum += map.has(key) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: map};
}

// Twin of bench/runner/src/fuzzy_multi_map.rs. Same hash as `fuzzy-map`; 50%
// set (mutating append, default Array container), 25% get (pure read,
// bucket length), 25% has (pure read). No delete/remove: this module has
// none.
function runMixedFuzzyMultiMap(FuzzyMultiMap, workload, k) {
  const map = new FuzzyMultiMap(fuzzyHash);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const key = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        map.set(key, workload.b[i]);
      } else if (op === 2) {
        const container = map.get(key);
        checksum += container === undefined ? 0 : container.length;
      } else {
        checksum += map.has(key) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: map};
}

// Twin of bench/runner/src/inverted_index.rs. An identity tokenizer, so a
// "document" IS its own token array -- the port's `Doc` (a bare document
// id, never read back by this workload) and its tokens are two different
// values on the Rust side and the same array reference here, which is an
// equivalent design choice: neither side's checksum ever reads a document's
// *content* back, only a query's result COUNT, so what the "document" value
// contains is inert either way. 50% add (mutating, two tokens), 25% a
// single-token get (pure read, contributing the match count), 25% a
// two-token get (pure read, additionally exercising the AND intersection).
// `workload.size` is the token VOCABULARY -- see that file's own module docs
// for why it is far smaller than the op count.
function runMixedInvertedIndex(InvertedIndex, workload, k) {
  const index = new InvertedIndex(function(x) { return x; });
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const t1 = workload.a[i];
      const t2 = workload.b[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        index.add([t1, t2]);
      } else if (op === 2) {
        checksum += index.get([t1]).length;
      } else {
        checksum += index.get([t1, t2]).length;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: index};
}

// Twin of bench/runner/src/trie_map.rs. Same hex-key generator and prefix-
// sharing shape as `runMixedTrie`, reused rather than re-derived. 50% `set`
// (mutating), 25% `get` (pure read, contributing the stored value), 25%
// `delete` (mutating, contributing upstream's own plain boolean return --
// see trie_map.rs's module docs for why the Rust side matches that shape
// rather than checksumming the displaced value core's richer API exposes).
function runMixedTrieMap(TrieMap, workload, k) {
  const trie = new TrieMap();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const word = workload.a[i].toString(16);
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        trie.set(word, workload.a[i]);
      } else if (op === 2) {
        const value = trie.get(word);
        checksum += value === undefined ? 0 : value;
      } else {
        checksum += trie.delete(word) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: trie};
}

// Twin of bench/runner/src/critbit_tree_map.rs::key. Zero-padded to six
// decimal digits, not bare `String(value)` -- see that file's own module
// docs for why fixed width is what makes most pairs diverge deep in the key
// rather than at byte 0.
function critbitKey(value) {
  return String(value).padStart(6, '0');
}

// Twin of bench/runner/src/critbit_tree_map.rs. 50% `set` (mutating), 25%
// `get` (pure read), 25% `delete` (mutating, upstream's own plain boolean).
function runMixedCritbitTreeMap(CritBitTreeMap, workload, k) {
  const map = new CritBitTreeMap();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const key = critbitKey(workload.a[i]);
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        map.set(key, workload.a[i]);
      } else if (op === 2) {
        const value = map.get(key);
        checksum += value === undefined ? 0 : value;
      } else {
        checksum += map.delete(key) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: map};
}

// Twin of bench/runner/src/fixed_critbit_tree_map.rs. No `delete` (see that
// file's own module docs -- upstream has none), so this is `fuzzy-map`'s
// set/get/has shape instead. `workload.size` is BOTH the capacity and the
// full key domain -- load-bearing, not a style choice: upstream's `set` has
// no capacity guard at all, and inserting a distinct key past capacity
// silently corrupts the tree and later THROWS from the walk that next
// passes through the corrupted node. Capping the domain at capacity is what
// lets this workload fill the tree without ever reaching that crash.
function runMixedFixedCritbitTreeMap(FixedCritBitTreeMap, workload, k) {
  const map = new FixedCritBitTreeMap(workload.size);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const key = critbitKey(workload.a[i]);
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        map.set(key, workload.a[i]);
      } else if (op === 2) {
        const value = map.get(key);
        checksum += value === undefined ? 0 : value;
      } else {
        checksum += map.has(key) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: map};
}

// Twin of bench/runner/src/bk_tree.rs::distance. `|a - b|`, not upstream's
// own Levenshtein -- a BK-tree is metric-agnostic, and a numeric metric both
// sides compute identically carries zero risk of a Levenshtein port
// drifting apart in an edge case. See that file's own module docs for the
// full account, including the two failure modes (domain too small, domain
// too large) found and rejected before committing to this workload's
// parameters.
function bkDistance(a, b) {
  return Math.abs(a - b);
}

// Twin of bench/runner/src/bk_tree.rs. 50% `add` (mutating), 25% `search` at
// SMALL_RADIUS (mostly pruned), 25% `search` at LARGE_RADIUS (reliably real
// matches) -- both contributing upstream's own `found.length`.
function runMixedBkTree(BKTree, workload, k) {
  const SMALL_RADIUS = 2;
  const LARGE_RADIUS = 20;
  const tree = new BKTree(bkDistance);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const item = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        tree.add(item);
      } else if (op === 2) {
        checksum += tree.search(SMALL_RADIUS, item).length;
      } else {
        checksum += tree.search(LARGE_RADIUS, item).length;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: tree};
}

// Twin of bench/runner/src/vp_tree.rs::shuffled_items. A SEPARATE
// fixed-seed (1) XorShift32 instance, independent of the workload's own
// stream -- see that file's own module docs for why an in-order `0..size`
// domain is quicksort's worst case for VPTree's construction step.
function vpShuffledItems(size) {
  const items = new Array(size);
  for (let i = 0; i < size; i++) items[i] = i;

  const rng = new XorShift32(1);

  for (let i = items.length - 1; i > 0; i--) {
    const j = rng.next() % (i + 1);
    const tmp = items[i];
    items[i] = items[j];
    items[j] = tmp;
  }

  return items;
}

function vpDistance(a, b) {
  return Math.abs(a - b);
}

// Twin of bench/runner/src/vp_tree.rs. No mutating op -- the tree is built
// once (untimed) from a shuffled 0..size, then every op is a query: 40%
// `neighbors` at SMALL_RADIUS (mostly pruned), 40% at LARGE_RADIUS (real
// matches, still a small fraction of the tree), 20% `nearestNeighbors(K)`
// (the heap-based query, its own pruning bound). Position-weighted
// checksum, not a plain sum -- `nearestNeighbors` pins an exact tie-break
// order in upstream's own test suite.
function runMixedVpTree(VPTree, workload, k) {
  const SMALL_RADIUS = 10;
  const LARGE_RADIUS = 400;
  const K = 10;
  const items = vpShuffledItems(workload.size);
  const tree = new VPTree(vpDistance, items);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const query = workload.a[i];
      const op = workload.kind[i];

      let hits;
      if (op === 0) {
        hits = tree.neighbors(SMALL_RADIUS, query);
      } else if (op === 1 || op === 2) {
        hits = tree.neighbors(LARGE_RADIUS, query);
      } else {
        hits = tree.nearestNeighbors(K, query);
      }

      for (let position = 0; position < hits.length; position++) {
        checksum += (position + 1) * hits[position].item;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: tree};
}

// Twin of bench/runner/src/kd_tree.rs::scattered_points. Two independent
// coordinates per point from a fixed-seed (1) XorShift32, same convention
// vp-tree's own shuffle uses -- an unsorted, uncorrelated domain matters
// here too (construction sorts each level's window by one axis's raw
// value).
function kdScatteredPoints(size) {
  const rng = new XorShift32(1);
  const axes = [new Array(size), new Array(size)];
  const labels = new Array(size);
  const domain = Math.max(size, 1);

  for (let i = 0; i < size; i++) {
    axes[0][i] = rng.below(domain);
    axes[1][i] = rng.below(domain);
    labels[i] = i;
  }

  return {axes: axes, labels: labels};
}

// Twin of bench/runner/src/kd_tree.rs. No mutating op -- the tree is built
// once (untimed) from scattered 2-D points, then every op is a query: 75%
// nearestNeighbor (pure read), 25% kNearestNeighbors(K) (pure read,
// position-weighted -- the heap pins an exact tie-break order).
function runMixedKdTree(KDTree, workload, k) {
  const K = 10;
  const built = kdScatteredPoints(workload.size);
  const tree = KDTree.fromAxes(built.axes, built.labels);
  const domain = Math.max(workload.size, 1);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const query = [workload.a[i] % domain, workload.b[i] % domain];
      const op = workload.kind[i];

      if (op < 3) {
        const label = tree.nearestNeighbor(query);
        checksum += label === null ? 0 : label;
      } else {
        const hits = tree.kNearestNeighbors(K, query);
        for (let position = 0; position < hits.length; position++) {
          checksum += (position + 1) * hits[position];
        }
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: tree};
}

// Twin of bench/runner/src/static_interval_tree.rs::overlapping_intervals
// and LENGTH_FRACTION. 0.1% of the domain, not the 10% first tried -- see
// that file's own module docs for the measurement (22 seconds for a
// 200,000-op pass) that ruled the larger fraction out.
const SIT_LENGTH_FRACTION = 0.001;

function sitOverlappingIntervals(size) {
  const rng = new XorShift32(1);
  const domain = Math.max(size, 1);
  const length = domain * SIT_LENGTH_FRACTION;
  const intervals = new Array(size);

  for (let i = 0; i < size; i++) {
    const start = rng.below(domain);
    intervals[i] = [start, start + length];
  }

  return intervals;
}

// Twin of bench/runner/src/static_interval_tree.rs. No mutating op -- the
// tree is built once (untimed), then every op is a query: 50%
// intervalsContainingPoint, 50% intervalsOverlappingInterval. Both
// contribute a position-weighted checksum, since neither query method sorts
// its output.
function runMixedStaticIntervalTree(StaticIntervalTree, workload, k) {
  const domain = Math.max(workload.size, 1);
  const length = domain * SIT_LENGTH_FRACTION;
  const intervals = sitOverlappingIntervals(workload.size);
  const tree = new StaticIntervalTree(intervals);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const op = workload.kind[i];

      let hits;
      if (op < 2) {
        const point = workload.a[i] % domain;
        hits = tree.intervalsContainingPoint(point);
      } else {
        const queryStart = workload.a[i] % domain;
        hits = tree.intervalsOverlappingInterval([queryStart, queryStart + length]);
      }

      for (let position = 0; position < hits.length; position++) {
        checksum += (position + 1) * (hits[position][0] + 1);
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: tree};
}

// Twin of bench/runner/src/fibonacci_heap.rs. Same shape as `heap.rs`: 50%
// push (mutating), 25% pop (mutating and a read -- where consolidation
// happens), 25% peek (pure read).
function runMixedFibonacciHeap(FibonacciHeap, workload, k) {
  const heap = new FibonacciHeap();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const value = workload.a[i];
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        heap.push(value);
      } else if (op === 2) {
        const popped = heap.pop();
        checksum += popped === undefined ? 0 : popped;
      } else {
        const peeked = heap.peek();
        checksum += peeked === undefined ? 0 : peeked;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: heap};
}

// Twin of bench/runner/src/fixed_reverse_heap.rs. Capacity is HALF the
// value domain, not a tiny slice of the op count -- see that file's own
// module docs for why a tiny capacity would fill once and then rarely
// displace anything again. 75% push (never guarded -- it cannot fail),
// 25% peek. No `consume`: draining resets size to zero, undoing the
// "capacity filled" state this workload exists to hold.
function runMixedFixedReverseHeap(FixedReverseHeap, workload, k) {
  const capacity = Math.max(Math.floor(workload.size / 2), 1);
  const heap = new FixedReverseHeap(Float64Array, capacity);
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const value = workload.a[i];
      const op = workload.kind[i];

      if (op < 3) {
        heap.push(value);
      } else {
        const peeked = heap.peek();
        checksum += peeked === undefined ? 0 : peeked;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: heap};
}

// Twin of bench/runner/src/bloom_filter.rs. Filter is prefilled to half its
// stated capacity (untimed) -- see that file's own module docs for why an
// empty/near-empty filter proves nothing. 50% add (mutating, same 0..size
// domain the prefill drew from -- fill ratio climbs toward ~1.0 over the
// run), 25% test on the hit pool (0..size), 25% test on the miss pool
// (size..2*size, never added).
function runMixedBloomFilter(BloomFilter, workload, k) {
  const domain = Math.max(workload.size, 1);
  const filter = new BloomFilter(domain);
  const prefill = Math.floor(workload.size / 2);

  for (let i = 0; i < prefill; i++) filter.add(i.toString(16));

  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        const member = workload.a[i] % domain;
        filter.add(member.toString(16));
      } else if (op === 2) {
        const member = workload.a[i] % domain;
        checksum += filter.test(member.toString(16)) ? 1 : 0;
      } else {
        const member = domain + (workload.a[i] % domain);
        checksum += filter.test(member.toString(16)) ? 1 : 0;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: filter};
}

// Twin of bench/runner/src/linked_list.rs::WALK_STEPS.
const LINKED_LIST_WALK_STEPS = 20;

// Twin of bench/runner/src/linked_list.rs. 50% push (tail, mutating), 25%
// shift (head removal, mutating and a read), 25% walk (a fresh cursor from
// the head, stepped LINKED_LIST_WALK_STEPS times -- the one op that
// genuinely chases node-to-node pointers rather than touching an end
// push/shift already hold a reference to).
function runMixedLinkedList(LinkedList, workload, k) {
  const list = new LinkedList();
  const ops = workload.kind.length;
  const batches = [];
  let checksum = 0;

  for (let start = 0; start < ops; start += k) {
    const end = Math.min(start + k, ops);
    const clock = process.hrtime.bigint();

    for (let i = start; i < end; i++) {
      const op = workload.kind[i];

      if (op === 0 || op === 1) {
        list.push(workload.a[i]);
      } else if (op === 2) {
        const shifted = list.shift();
        checksum += shifted === undefined ? 0 : shifted;
      } else {
        const iterator = list.values();
        let last;

        for (let step = 0; step < LINKED_LIST_WALK_STEPS; step++) {
          const result = iterator.next();
          if (result.done) break;
          last = result.value;
        }

        checksum += last === undefined ? 0 : last;
      }
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, set: list};
}

// Dispatch table, twin of harness.rs::MODULES's `mixed` field. Replaces what
// was a two-armed ternary before five more modules made that the wrong shape.
const MIXED_RUNNERS = {
  'static-disjoint-set': runOnce,
  'sparse-set': runMixedSparse,
  'bit-set': runMixedBitSet,
  'lru-cache': runMixedLru,
  'heap': runMixedHeap,
  'trie': runMixedTrie,
  'vector': runMixedVector,
  // Appended for the sequence-backed batch, never inserted (twin of
  // harness.rs::MODULES).
  'stack': runMixedStack,
  'queue': runMixedQueue,
  'fixed-stack': runMixedFixedStack,
  'fixed-deque': runMixedFixedDeque,
  'circular-buffer': runMixedCircularBuffer,
  'hashed-array-tree': runMixedHashedArrayTree,
  'sparse-map': runMixedSparseMap,
  'sparse-queue-set': runMixedSparseQueueSet,
  'bit-vector': runMixedBitVector,
  // Appended for the map-like/multi-container batch, never inserted (twin of
  // harness.rs::MODULES).
  'default-map': runMixedDefaultMap,
  'bi-map': runMixedBiMap,
  'multi-map': runMixedMultiMap,
  'multi-set': runMixedMultiSet,
  'multi-array': runMixedMultiArray,
  'fuzzy-map': runMixedFuzzyMap,
  'fuzzy-multi-map': runMixedFuzzyMultiMap,
  'inverted-index': runMixedInvertedIndex,
  // Appended for the final Gate 10 batch (the last fourteen units), never
  // inserted: twin of harness.rs::MODULES.
  'trie-map': runMixedTrieMap,
  'critbit-tree-map': runMixedCritbitTreeMap,
  'fixed-critbit-tree-map': runMixedFixedCritbitTreeMap,
  'bk-tree': runMixedBkTree,
  'vp-tree': runMixedVpTree,
  'kd-tree': runMixedKdTree,
  'static-interval-tree': runMixedStaticIntervalTree,
  'fibonacci-heap': runMixedFibonacciHeap,
  'fixed-reverse-heap': runMixedFixedReverseHeap,
  'bloom-filter': runMixedBloomFilter,
  'linked-list': runMixedLinkedList
};

// Twin of harness.rs::MODULES's `structure` field: build the structure at
// `size` and return one element to read, so nothing can be deferred or
// elided. `bit-set`/`lru-cache` preallocate to a fixed capacity like
// `sparse-set`/`static-disjoint-set` (bit-set's capacity is `size` directly;
// lru-cache's is `capacityForLru(size)`, matching runMixedLru). `heap`/
// `trie`/`vector` have no capacity distinct from occupied size, so "size"
// means "prefilled with `size` elements" for those three instead -- see each
// bench/runner/src/*.rs file's own `build_structure` doc for why.
const STRUCTURE_BUILDERS = {
  'static-disjoint-set': function (StaticDisjointSet, size) {
    const set = new StaticDisjointSet(size);
    return set.parents[size - 1];
  },
  'sparse-set': function (SparseSet, size) {
    const set = new SparseSet(size);
    return set.dense[size - 1];
  },
  'bit-set': function (BitSet, size) {
    const set = new BitSet(size);
    return set.get(size - 1);
  },
  'lru-cache': function (LRUCache, size) {
    const cache = new LRUCache(capacityForLru(size));
    return cache.has(0);
  },
  'heap': function (Heap, size) {
    const heap = new Heap();

    for (let i = 0; i < size; i++) heap.push(i);

    return heap.peek();
  },
  'trie': function (Trie, size) {
    const trie = new Trie();

    for (let i = 0; i < size; i++) trie.add(i.toString(16));

    return trie.has((size - 1).toString(16));
  },
  'vector': function (Vector, size) {
    const vector = new Vector(Float64Array, 0);

    for (let i = 0; i < size; i++) vector.push(i);

    return vector.get(vector.length - 1);
  },
  // Appended for the sequence-backed batch, never inserted.
  'stack': function (Stack, size) {
    const stack = new Stack();

    for (let i = 0; i < size; i++) stack.push(i);

    return stack.peek();
  },
  'queue': function (Queue, size) {
    const queue = new Queue();

    for (let i = 0; i < size; i++) queue.enqueue(i);

    return queue.peek();
  },
  'fixed-stack': function (FixedStack, size) {
    const stack = new FixedStack(Float64Array, size);
    return stack.peek();
  },
  'fixed-deque': function (FixedDeque, size) {
    const deque = new FixedDeque(Float64Array, size);
    return deque.peekLast();
  },
  'circular-buffer': function (CircularBuffer, size) {
    const buffer = new CircularBuffer(Float64Array, size);
    return buffer.peekLast();
  },
  'hashed-array-tree': function (HashedArrayTree, size) {
    const tree = new HashedArrayTree(Uint32Array);

    for (let i = 0; i < size; i++) tree.push(i);

    return tree.get(size - 1);
  },
  'sparse-map': function (SparseMap, size) {
    const map = new SparseMap(size);
    return map.has(0);
  },
  'sparse-queue-set': function (SparseQueueSet, size) {
    const queue = new SparseQueueSet(size);
    return queue.has(0);
  },
  'bit-vector': function (BitVector, size) {
    const vector = new BitVector(0);

    for (let i = 0; i < size; i++) vector.push(i % 2 === 1);

    return vector.get(vector.length - 1);
  },
  // `sort`/`suffix-array` have no persistent structure -- see each
  // bench/runner/src/*.rs file's own `build_structure` doc for what this
  // number means instead (transient footprint of one call, fixed seed 42
  // regardless of the run's own `--seed`, matching the Rust side).
  'sort': function (Sort, size) {
    const rng = new XorShift32(42);
    const buffer = new Array(size);

    for (let i = 0; i < size; i++) buffer[i] = rng.below(1000000);

    Sort.inplaceQuickSort(buffer, 0, buffer.length);

    return buffer[0];
  },
  'suffix-array': function (SuffixArray, size) {
    const rng = new XorShift32(42);
    const codes = new Array(size);

    for (let i = 0; i < size; i++) codes[i] = String.fromCharCode(65 + rng.below(4));

    const array = new SuffixArray(codes.join(''));

    return array.array[size - 1];
  },
  // Appended for the map-like/multi-container batch, never inserted (twin of
  // harness.rs::MODULES). Each "one value per key" fill mirrors its own
  // bench/runner/src/*.rs::build_structure -- see those files' own docs.
  'default-map': function (DefaultMap, size) {
    const map = new DefaultMap(function(key) { return key; });

    for (let i = 0; i < size; i++) map.set(i, i);

    return map.has(size - 1);
  },
  'bi-map': function (BiMap, size) {
    const map = new BiMap();

    for (let i = 0; i < size; i++) map.set(i, i);

    return map.has(size - 1);
  },
  'multi-map': function (MultiMap, size) {
    const map = new MultiMap();

    for (let i = 0; i < size; i++) map.set(i, i);

    return map.has(size - 1);
  },
  'multi-set': function (MultiSet, size) {
    const set = new MultiSet();

    for (let i = 0; i < size; i++) set.add(i, 1);

    return set.has(size - 1);
  },
  'multi-array': function (MultiArray, size) {
    const array = new MultiArray();

    for (let i = 0; i < size; i++) array.push(i);

    return array.has(size - 1);
  },
  'fuzzy-map': function (FuzzyMap, size) {
    const map = new FuzzyMap(fuzzyHash);

    for (let i = 0; i < size; i++) map.set(i, i);

    return map.has(fuzzyHash(size - 1));
  },
  'fuzzy-multi-map': function (FuzzyMultiMap, size) {
    const map = new FuzzyMultiMap(fuzzyHash);

    for (let i = 0; i < size; i++) map.set(i, i);

    return map.has(fuzzyHash(size - 1));
  },
  // `size` is the token VOCABULARY here (matching runMixedInvertedIndex's own
  // meaning of it), so this fills `size * DOCS_PER_WORD` documents rather
  // than `size` -- twin of bench/runner/src/inverted_index.rs::DOCS_PER_WORD.
  'inverted-index': function (InvertedIndex, size) {
    const DOCS_PER_WORD = 100;
    const index = new InvertedIndex(function(x) { return x; });
    const docCount = Math.max(1, size * DOCS_PER_WORD);

    for (let i = 0; i < docCount; i++) index.add([i % size, Math.floor(i / size) % size]);

    return index.get([0]).length;
  },
  // `set.js` has no persistent structure of its own (see set_ops.rs's own
  // module docs) -- a plain native `Set`, not `SetOps` itself, is what is
  // actually built and measured.
  'set': function (SetOps, size) {
    const set = new Set();

    for (let i = 0; i < size; i++) set.add(i);

    return set.has(size - 1);
  },
  // Appended for the final Gate 10 batch (the last fourteen units), never
  // inserted: twin of harness.rs::MODULES. Same convention as `trie`: "size"
  // means "prefilled with `size` distinct hex keys".
  'trie-map': function (TrieMap, size) {
    const trie = new TrieMap();

    for (let i = 0; i < size; i++) trie.set(i.toString(16), i);

    return trie.get((size - 1).toString(16));
  },
  'critbit-tree-map': function (CritBitTreeMap, size) {
    const map = new CritBitTreeMap();

    for (let i = 0; i < size; i++) map.set(critbitKey(i), i);

    return map.get(critbitKey(size - 1));
  },
  // Capacity IS the domain here too -- see runMixedFixedCritbitTreeMap's own
  // docs for why that is load-bearing rather than a style choice.
  'fixed-critbit-tree-map': function (FixedCritBitTreeMap, size) {
    const map = new FixedCritBitTreeMap(size);

    for (let i = 0; i < size; i++) map.set(critbitKey(i), i);

    return map.get(critbitKey(size - 1));
  },
  // Twin of bench/runner/src/bk_tree.rs::build_structure: "size" means
  // "after `size` `add` calls over the `0..size` domain".
  'bk-tree': function (BKTree, size) {
    const tree = new BKTree(bkDistance);
    const domain = Math.max(size, 1);

    for (let i = 0; i < size; i++) tree.add(i % domain);

    return tree.size;
  },
  // Twin of bench/runner/src/vp_tree.rs::build_structure: "size" means
  // "built from a shuffled 0..size".
  'vp-tree': function (VPTree, size) {
    const items = vpShuffledItems(size);
    const tree = new VPTree(vpDistance, items);

    return tree.size;
  },
  // Twin of bench/runner/src/kd_tree.rs::build_structure: "size" means
  // "built from `size` scattered points".
  'kd-tree': function (KDTree, size) {
    const built = kdScatteredPoints(size);
    const tree = KDTree.fromAxes(built.axes, built.labels);

    return tree.size;
  },
  // Twin of bench/runner/src/static_interval_tree.rs::build_structure:
  // "size" means "built from `size` overlapping intervals".
  'static-interval-tree': function (StaticIntervalTree, size) {
    const intervals = sitOverlappingIntervals(size);
    const tree = new StaticIntervalTree(intervals);

    return tree.size;
  },
  'fibonacci-heap': function (FibonacciHeap, size) {
    const heap = new FibonacciHeap();

    for (let i = 0; i < size; i++) heap.push(i);

    return heap.peek();
  },
  // Twin of bench/runner/src/fixed_reverse_heap.rs::build_structure:
  // capacity size/2, prefilled with `size` pushes.
  'fixed-reverse-heap': function (FixedReverseHeap, size) {
    const capacity = Math.max(Math.floor(size / 2), 1);
    const heap = new FixedReverseHeap(Float64Array, capacity);

    for (let i = 0; i < size; i++) heap.push(i);

    return heap.peek();
  },
  // Twin of bench/runner/src/bloom_filter.rs::build_structure: "size" means
  // "capacity `size`, prefilled with `size` items" -- full, not the
  // workload's own half-capacity starting point.
  'bloom-filter': function (BloomFilter, size) {
    const domain = Math.max(size, 1);
    const filter = new BloomFilter(domain);

    for (let i = 0; i < size; i++) filter.add(i.toString(16));

    return filter.test((size - 1).toString(16));
  },
  'linked-list': function (LinkedList, size) {
    const list = new LinkedList();

    for (let i = 0; i < size; i++) list.push(i);

    return list.last();
  }
};

// Twin of bench/runner/src/sparse_set.rs `prefilled`.
function prefilled(SparseSet, size, seed) {
  const set = new SparseSet(size);
  const rng = new XorShift32(seed);

  for (let i = 0; i < size; i++) set.add(rng.below(size));

  return set;
}

// Twin of `run_drain`. One timed sample per full walk, because a cursor costs
// something per walk as well as per element and splitting a walk across
// samples would hide the creation cost in whichever sample contained it.
function runDrain(SparseSet, size, seed, passes) {
  const set = prefilled(SparseSet, size, seed);
  const perPass = set.size;
  const batches = [];
  let checksum = 0;

  for (let pass = 0; pass < passes; pass++) {
    const clock = process.hrtime.bigint();

    // A fresh iterator per pass: the collection's Symbol.iterator is a factory
    // (D-07), and reusing one would measure an exhausted cursor from pass 2 on.
    const iterator = set.values();
    let step = iterator.next();

    while (!step.done) {
      checksum += step.value;
      step = iterator.next();
    }

    batches.push(Number(process.hrtime.bigint() - clock));
  }

  return {batches: batches, checksum: checksum, perPass: perPass, set: set};
}

// Twin of bench/runner/src/sort.rs. `sort` has no instance and no per-element
// op stream, so it reuses the `drain` shape: one measured sample per SORT,
// not per element. The whole `size * passes` buffer of random values is drawn
// from the matched xorshift stream once, before any timing -- re-sorting the
// PREVIOUS pass's (now-sorted) output would feed upstream's fixed-pivot
// quicksort its worst case on every pass after the first, silently turning an
// O(n log n) benchmark into an O(n^2) one. `size` elements are viewed via
// `subarray`, a zero-copy view, so JS sorts in place exactly as the Rust side
// does over its own disjoint slice -- no side gets credited for skipping a
// copy the other one pays for.
function runDrainSort(Sort, size, seed, passes) {
  const rng = new XorShift32(seed);
  const buffer = new Float64Array(size * passes);

  for (let i = 0; i < buffer.length; i++) buffer[i] = rng.below(1000000);

  const batches = [];
  let checksum = 0;

  for (let pass = 0; pass < passes; pass++) {
    const chunk = buffer.subarray(pass * size, (pass + 1) * size);

    const clock = process.hrtime.bigint();
    Sort.inplaceQuickSort(chunk, 0, chunk.length);
    batches.push(Number(process.hrtime.bigint() - clock));

    // Outside the timed region: a verification read, not part of what the
    // sort itself costs. Position-weighted rather than a sum, because a sum
    // cannot tell a correctly sorted array from an unsorted one of the same
    // multiset -- see sort.rs's own docs on why this checksum is shaped this
    // way.
    for (let i = 0; i < chunk.length; i++) checksum += (i + 1) * chunk[i];
  }

  return {batches: batches, checksum: checksum, perPass: size, set: buffer};
}

// Twin of bench/runner/src/suffix_array.rs. Same drain shape as `sort`: one
// measured sample per CONSTRUCTION. A four-symbol alphabet, generated fresh
// per pass from the matched stream so the recursive case (repeated triples)
// is exercised rather than avoided -- see that file's own docs on B-91.
function runDrainSuffixArray(SuffixArray, size, seed, passes) {
  const rng = new XorShift32(seed);
  const codes = new Array(size * passes);

  for (let i = 0; i < codes.length; i++) codes[i] = String.fromCharCode(65 + rng.below(4));

  const text = codes.join('');
  const batches = [];
  let checksum = 0;

  for (let pass = 0; pass < passes; pass++) {
    const clock = process.hrtime.bigint();

    // The slice (a copy: JS strings are immutable) is inside the timed
    // region, matching the Rust side's `.to_vec()` -- both sides pay an
    // equivalent copy, so this stays symmetric.
    const slice = text.slice(pass * size, (pass + 1) * size);
    const array = new SuffixArray(slice);

    batches.push(Number(process.hrtime.bigint() - clock));

    const positions = array.array;
    for (let i = 0; i < positions.length; i++) checksum += (i + 1) * positions[i];
  }

  return {batches: batches, checksum: checksum, perPass: size, set: text};
}

// Twin of bench/runner/src/set_ops.rs. One measured sample per `union` call
// -- `union` is the representative choice out of set.js's fourteen free
// functions; see that file's own module docs for why. Both `A` and `B` are
// `size` elements drawn from the SAME `0..size` domain, guaranteeing real
// overlap and internal duplicates by the birthday bound -- see that file's
// docs. `perPass` is `2 * size` (the number of source elements `union`
// visits per call), constant across passes.
function runDrainSet(SetOps, size, seed, passes) {
  const rng = new XorShift32(seed);
  const buffer = new Uint32Array(2 * size * passes);

  for (let i = 0; i < buffer.length; i++) buffer[i] = rng.below(size);

  const batches = [];
  let checksum = 0;

  for (let pass = 0; pass < passes; pass++) {
    const base = pass * 2 * size;

    const a = new Set();
    for (let i = base; i < base + size; i++) a.add(buffer[i]);

    const b = new Set();
    for (let i = base + size; i < base + 2 * size; i++) b.add(buffer[i]);

    const clock = process.hrtime.bigint();
    const result = SetOps.union(a, b);
    batches.push(Number(process.hrtime.bigint() - clock));

    // Outside the timed region: a verification read, not part of what
    // `union` itself costs. Position-weighted, not a sum -- see that file's
    // own docs on why order has to be part of the checksum.
    let index = 0;
    for (const member of result) {
      checksum += (index + 1) * member;
      index++;
    }
  }

  return {batches: batches, checksum: checksum, perPass: 2 * size, set: null};
}

// Dispatch table, twin of harness.rs::MODULES's `drain` field. `sparse-set`
// was the only drain-shaped module before this batch, so its loop used to be
// called directly by name; two more modules with genuinely different drain
// bodies made that the wrong shape, the same lesson `MIXED_RUNNERS` already
// learned.
const DRAIN_RUNNERS = {
  'sparse-set': runDrain,
  'sort': runDrainSort,
  'suffix-array': runDrainSuffixArray,
  // Appended for the map-like/multi-container batch, never inserted (twin of
  // harness.rs::MODULES).
  'set': runDrainSet
};

// --- CLI --------------------------------------------------------------------

function value(name, fallback) {
  const at = argv.indexOf(name);

  return at === -1 || at + 1 >= argv.length ? fallback : argv[at + 1];
}

function number(name, fallback) {
  const raw = value(name, null);
  const parsed = raw === null ? NaN : Number(raw);

  return Number.isFinite(parsed) ? parsed : fallback;
}

// Twin of size_flag() in bench/runner/src/main.rs. Rejecting size 0 matters
// specifically here: `x % 0` is NaN in JS, and NaN coerced into a typed array
// becomes 0, so this side would happily produce an all-zero workload and
// report a plausible-looking measurement of nothing while the Rust side
// panicked. Upper bound mirrors the Rust u32 parse.
function sizeArg() {
  const size = number('--size', DEFAULT_SIZE);

  if (!Number.isInteger(size) || size < 1 || size > 4294967295) {
    process.stderr.write('run.js: `--size` must be an integer in 1..=4294967295\n');
    process.exitCode = 2;
    return null;
  }

  return size;
}

// `maxRSS` is kilobytes on Linux, matching getrusage on the Rust side.
function peakRssKb() {
  return process.resourceUsage().maxRSS;
}

function main() {
  if (argv.includes('--noop')) return;

  const dump = value('--dump-prng', null);

  if (dump !== null) {
    const rng = new XorShift32(DEFAULT_SEED);
    const out = [];

    for (let i = 0; i < Number(dump); i++) out.push(rng.next());

    process.stdout.write(out.join('\n') + '\n');
    return;
  }

  if (argv.includes('--baseline')) {
    // Node carries ~40 MB of V8 before a single element exists (5.2 Problem 3).
    // Subtracting this is what turns peak RSS from a runtime comparison into a
    // data-structure comparison.
    process.stdout.write(JSON.stringify({side: 'original', mode: 'baseline', rss_kb: peakRssKb()}) + '\n');
    return;
  }

  const module = value('--module', 'static-disjoint-set');
  const kind = value('--kind', 'mixed');

  if (!MODULES[module]) {
    process.stderr.write('run.js: unknown module `' + module + '`\n');
    process.exitCode = 2;
    return;
  }

  // `sort` has no `sort.js` at the upstream root -- it is a directory of two
  // files (`sort/quick.js`, `sort/insertion.js`), unlike every other module
  // here, which is one file matching its own name. Special-cased rather than
  // adding a stub `sort.js`, which would mean maintaining a second copy of
  // something `bench/upstream/` vendors from a pinned upstream commit.
  const Structure = module === 'sort'
    ? require(path.join(__dirname, '..', 'upstream', 'sort', 'quick.js'))
    : require(path.join(__dirname, '..', 'upstream', module + '.js'));

  if (argv.includes('--structure')) {
    // Twin of bench-runner --structure: build the structure, touch nothing
    // else, report peak RSS. Isolates the structure from the ~9 MB of
    // materialised workload arrays that dominate the mixed run's RSS delta.
    //
    // Deliberately BEFORE the `--kind` check below: `--structure` never
    // consults `kind` at all, and gating it on the same check meant a
    // drain-only module (no `mixed` kind: `suffix-array`, `sort`) could never
    // reach this branch, since the default `--kind` is `mixed` and nothing
    // here supplies `--kind drain` on its own. Invisible until a module
    // existed with no `mixed` kind -- every prior module has one. Twin of the
    // same reordering in bench/runner/src/main.rs.
    const size = sizeArg();

    if (size === null) return;

    // Twin of harness.rs::ModuleEntry::structure. Reads one element back so
    // nothing can be deferred or elided.
    global.__keepAlive = STRUCTURE_BUILDERS[module](Structure, size);

    process.stdout.write(JSON.stringify({
      side: 'original', mode: 'structure', size: size, rss_kb: peakRssKb()
    }) + '\n');
    return;
  }

  if (MODULES[module].indexOf(kind) === -1) {
    process.stderr.write('run.js: module `' + module + '` has no `' + kind + '` workload\n');
    process.exitCode = 2;
    return;
  }

  const warmup = number('--warmup', 3);
  const measured = number('--measured', 1);
  const size = sizeArg();

  if (size === null) return;

  const ops = number('--ops', DEFAULT_OPS);
  const seed = number('--seed', DEFAULT_SEED);
  const passes = number('--passes', DEFAULT_PASSES);

  if (kind === 'drain') {
    drain(Structure, module, size, seed, passes, warmup, measured);
    return;
  }

  const workload = generate(size, ops, seed);

  // Mandatory, and stated in methodology.md: measuring cold JS against
  // optimised Rust is a dishonest win.
  const run = function (w) { return MIXED_RUNNERS[module](Structure, w, BATCH_K); };

  let checksum = 0;

  for (let i = 0; i < warmup; i++) {
    checksum = run(workload).checksum;
  }

  let batches = [];

  for (let i = 0; i < measured; i++) {
    const pass = run(workload);

    if (warmup > 0 && pass.checksum !== checksum) {
      process.stderr.write('run.js: checksum changed between passes\n');
      process.exitCode = 2;
      return;
    }

    checksum = pass.checksum;
    batches = batches.concat(pass.batches);
  }

  process.stdout.write(JSON.stringify({
    side: 'original',
    module: module,
    kind: kind,
    size: size,
    ops: ops,
    seed: seed,
    batch_k: BATCH_K,
    warmup: warmup,
    measured: measured,
    checksum: checksum,
    batch_ns: batches,
    rss_kb: peakRssKb()
  }) + '\n');
}

// Twin of `drain()` in bench/runner/src/main.rs. `batch_k` carries
// members-per-pass rather than a fixed 1000, so the driver's `ns / batch_k`
// still means nanoseconds per element.
function drain(Structure, module, size, seed, passes, warmup, measured) {
  if (!Number.isInteger(passes) || passes < 1) {
    process.stderr.write('run.js: `--passes` must be at least 1\n');
    process.exitCode = 2;
    return;
  }

  // Twin of harness.rs::ModuleEntry::drain: a table rather than a single
  // hardcoded loop, once `sort`/`suffix-array` needed genuinely different
  // drain bodies from `sparse-set`'s prefill-and-walk.
  const runDrainFor = DRAIN_RUNNERS[module];

  let checksum = 0;
  let perPass = 0;

  for (let i = 0; i < warmup; i++) {
    const pass = runDrainFor(Structure, size, seed, passes);
    checksum = pass.checksum;
    perPass = pass.perPass;
  }

  let batches = [];

  for (let i = 0; i < measured; i++) {
    const pass = runDrainFor(Structure, size, seed, passes);

    if (warmup > 0 && pass.checksum !== checksum) {
      process.stderr.write('run.js: checksum changed between passes\n');
      process.exitCode = 2;
      return;
    }

    checksum = pass.checksum;
    perPass = pass.perPass;
    batches = batches.concat(pass.batches);
  }

  if (perPass === 0) {
    process.stderr.write('run.js: nothing was produced to drain (empty per-pass count)\n');
    process.exitCode = 2;
    return;
  }

  process.stdout.write(JSON.stringify({
    side: 'original',
    module: module,
    kind: 'drain',
    size: size,
    ops: perPass * passes * measured,
    seed: seed,
    batch_k: perPass,
    passes: passes,
    warmup: warmup,
    measured: measured,
    checksum: checksum,
    batch_ns: batches,
    rss_kb: peakRssKb()
  }) + '\n');
}

main();

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
  'sparse-set': ['mixed', 'drain']
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

  if (MODULES[module].indexOf(kind) === -1) {
    process.stderr.write('run.js: module `' + module + '` has no `' + kind + '` workload\n');
    process.exitCode = 2;
    return;
  }

  const Structure = require(path.join(__dirname, '..', 'upstream', module + '.js'));

  if (argv.includes('--structure')) {
    // Twin of bench-runner --structure: build the structure, touch nothing
    // else, report peak RSS. Isolates the structure from the ~9 MB of
    // materialised workload arrays that dominate the mixed run's RSS delta.
    const size = sizeArg();

    if (size === null) return;

    const set = new Structure(size);

    // Read one element so nothing can be deferred or elided.
    global.__keepAlive = module === 'sparse-set' ? set.dense[size - 1] : set.parents[size - 1];

    process.stdout.write(JSON.stringify({
      side: 'original', mode: 'structure', size: size, rss_kb: peakRssKb()
    }) + '\n');
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
  const run = module === 'sparse-set'
    ? function (w) { return runMixedSparse(Structure, w, BATCH_K); }
    : function (w) { return runOnce(Structure, w, BATCH_K); };

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
function drain(SparseSet, module, size, seed, passes, warmup, measured) {
  if (!Number.isInteger(passes) || passes < 1) {
    process.stderr.write('run.js: `--passes` must be at least 1\n');
    process.exitCode = 2;
    return;
  }

  let checksum = 0;
  let perPass = 0;

  for (let i = 0; i < warmup; i++) {
    const pass = runDrain(SparseSet, size, seed, passes);
    checksum = pass.checksum;
    perPass = pass.perPass;
  }

  let batches = [];

  for (let i = 0; i < measured; i++) {
    const pass = runDrain(SparseSet, size, seed, passes);

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
    process.stderr.write('run.js: the prefilled set is empty, so there is nothing to drain\n');
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

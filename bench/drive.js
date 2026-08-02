#!/usr/bin/env node
//
// Benchmark driver: runs both sides, interleaved, and writes one keyed entry
// into bench/results.json.
//
// Why the driver rather than each runner computes the percentiles: DESIGN.md
// 5.2 Problem 1 asks for "same percentile maths" on both sides. Implementing
// the maths twice and asserting the implementations agree is weaker than
// implementing it once over samples from both, so the runners emit raw
// per-batch nanoseconds and nothing else. The runners themselves stay strictly
// symmetric; the shared part is shared.
//
// Four protocol rules from 5.1-5.2, all enforced here:
//
//   1. Matched PRNG, verified. The first 1000 values from each side are
//      diffed before anything is measured, and a mismatch aborts.
//   2. Batched timing at K = 1000, so p99 is a property of the code and of
//      V8's GC rather than of clock_gettime.
//   3. RSS in-process on both sides, reported as total AND as a delta over a
//      no-op baseline of the same runtime.
//   4. Interleaved A/B/A/B. Thermal drift and background load are monotonic
//      over a run; interleaving cancels them, and running all of one side then
//      all of the other bakes them into the result.
//
// A fifth check is not in the spec but falls out for free: both runners emit a
// checksum over the results of every non-mutating op. If the port and upstream
// disagree, the checksums differ and the driver refuses to record anything.
// That makes "same workload" a verified claim rather than an assertion.
'use strict';

const {execFileSync} = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

const ROOT = path.resolve(__dirname, '..');
const RESULTS = path.join(ROOT, 'bench', 'results.json');
const RAW = path.join(ROOT, 'bench', 'raw');
const RUST = path.join(ROOT, 'target', 'release', 'bench-runner');
const NODE_RUNNER = path.join(ROOT, 'bench', 'node', 'run.js');

const REPS = 10;      // measured passes per side (5.1)
const WARMUP = 3;     // warmup passes before each measured pass (5.1)

const module_ = process.argv[2] || 'static-disjoint-set';

// Pin at the process boundary on bare metal. In Docker this is --cpuset-cpus
// instead (12c.2 point 4) and PIN=0 turns it off.
const PIN = process.env.BENCH_PIN === '0' ? [] : pinPrefix();

function pinPrefix() {
  try {
    execFileSync('taskset', ['--version'], {stdio: 'ignore'});
    return ['taskset', '-c', process.env.BENCH_CPUS || '2,3'];
  } catch (error) {
    return [];
  }
}

function run(command, args, options) {
  const full = PIN.concat([command], args);

  return execFileSync(full[0], full.slice(1), Object.assign({
    encoding: 'utf8',
    maxBuffer: 1 << 28
  }, options || {}));
}

// --- rule 1: prove the two PRNGs are the same generator ---------------------

function verifyPrng() {
  const rust = run(RUST, ['--dump-prng', '1000']);
  const node = run(process.execPath, [NODE_RUNNER, '--dump-prng', '1000']);

  if (rust !== node) {
    const rustLines = rust.trim().split('\n');
    const nodeLines = node.trim().split('\n');
    const at = rustLines.findIndex((line, i) => line !== nodeLines[i]);

    throw new Error(
      'PRNG streams diverge at value ' + at + ': rust=' + rustLines[at] + ' node=' + nodeLines[at] +
      '\nThe workload is not matched; no number produced here would be meaningful.'
    );
  }

  return rust.trim().split('\n').length;
}

// --- statistics -------------------------------------------------------------

// Nearest-rank percentile. Applied once, to both sides, so the choice of
// definition cannot advantage either.
function percentile(sorted, q) {
  const rank = Math.ceil(q * sorted.length);

  return sorted[Math.min(Math.max(rank, 1), sorted.length) - 1];
}

function summarise(batches, k, rssKb, baselineKb, startupMs, structureKb) {
  const sorted = batches.slice().sort((x, y) => x - y);

  return {
    p50_ns_per_op: round(percentile(sorted, 0.50) / k, 3),
    p99_ns_per_op: round(percentile(sorted, 0.99) / k, 3),
    min_ns_per_op: round(sorted[0] / k, 3),
    rss_total_mb: round(rssKb / 1024, 1),
    rss_delta_mb: round((rssKb - baselineKb) / 1024, 1),
    structure_rss_delta_mb: round((structureKb - baselineKb) / 1024, 1),
    startup_ms: round(startupMs, 1),
    samples: batches.length
  };
}

function round(value, places) {
  const scale = Math.pow(10, places);

  return Math.round(value * scale) / scale;
}

function si(value) {
  return value >= 1000000 ? (value / 1000000) + 'e6' : String(value);
}

// Every metric here is lower-is-better, so a regression is mechanical to find.
// It is computed rather than written down on purpose: DESIGN.md 5.1 says
// hiding a regression scores worse than disclosing one, and a field nobody has
// to remember to fill in cannot be quietly left out on a bad day.
function regressions(entry) {
  const metrics = [
    'p50_ns_per_op', 'p99_ns_per_op', 'min_ns_per_op',
    'rss_total_mb', 'rss_delta_mb', 'structure_rss_delta_mb', 'startup_ms'
  ];

  const worse = [];

  for (const metric of metrics) {
    const port = entry.port[metric];
    const original = entry.original[metric];

    if (port > original) {
      worse.push({
        metric: metric,
        port: port,
        original: original,
        ratio: round(port / original, 2)
      });
    }
  }

  return worse;
}

// --- rule 3: baselines ------------------------------------------------------

function baseline(side) {
  const out = side === 'port'
    ? run(RUST, ['--baseline'])
    : run(process.execPath, [NODE_RUNNER, '--baseline']);

  return JSON.parse(out).rss_kb;
}

// The structure on its own, with no workload arrays in the picture. This is
// where the port's PointerVec design shows up honestly: it backs every logical
// width with a Vec<u32>, so a Uint8Array upstream costs four times as much
// here. The mixed run's rss_delta cannot see that, because ~9 MB of identical
// op arrays swamp it.
function structureRss(side, size) {
  const out = side === 'port'
    ? run(RUST, ['--structure', '--size', String(size)])
    : run(process.execPath, [NODE_RUNNER, '--structure', '--size', String(size)]);

  return JSON.parse(out).rss_kb;
}

// --- startup, measured with the one tool that is fair to both ---------------

function startup() {
  fs.mkdirSync(RAW, {recursive: true});

  const out = path.join(RAW, 'startup.json');

  execFileSync('hyperfine', [
    '--warmup', '5',
    '--runs', '30',
    '-N',
    '--export-json', out,
    '--command-name', 'port', RUST + ' --noop',
    '--command-name', 'original', process.execPath + ' ' + NODE_RUNNER + ' --noop'
  ], {stdio: 'inherit'});

  const parsed = JSON.parse(fs.readFileSync(out, 'utf8'));
  const byName = {};

  for (const result of parsed.results) byName[result.command] = result.mean * 1000;

  return byName;
}

// --- host -------------------------------------------------------------------

function host() {
  let governor = 'unknown';

  try {
    governor = fs.readFileSync(
      '/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor', 'utf8'
    ).trim();
  } catch (error) {
    // WSL2 exposes no cpufreq node. Recorded honestly rather than guessed.
    governor = 'unavailable';
  }

  return {
    cpu: (os.cpus()[0] || {}).model || 'unknown',
    cores: os.cpus().length,
    governor: governor,
    ram_gb: round(os.totalmem() / (1024 * 1024 * 1024), 1),
    rustc: run('rustc', ['--version']).trim(),
    node: process.version,
    kernel: os.release(),
    in_docker: fs.existsSync('/.dockerenv'),
    pinned_to: PIN.length ? PIN.slice(1).join(' ') : 'not pinned'
  };
}

// --- main -------------------------------------------------------------------

// Workloads, per module. Each entry is one row of the published table.
//
// `static-disjoint-set` has two, and the second exists because of what the
// first one hid: `mixed-1e6` is the headline and the port wins every metric on
// it, which is a suspiciously clean result for a library that is already
// typed-array-backed. `mixed-4e6` is the same op mix at four times the size,
// found by probing for the boundary, and it is where a representation choice
// in the port once turned a p50 win into a p99 loss. Keeping only the size
// that flatters the port would have been the easiest possible way to publish a
// dishonest table.
//
// `sparse-set` adds `drain`, and that one is not symmetry for its own sake:
// iteration is the whole reason this module was ported now, and the drain
// workload is the only benchmark in the repo that puts the cursor machinery of
// DESIGN.md 3.4 against the JS closure it was ported from. Its batch is a
// whole walk rather than 1000 elements, because a cursor costs something per
// walk (it freezes state at creation) as well as per element.
const WORKLOADS = {
  'static-disjoint-set': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed union/find/connected (50/25/25)'
    },
    {
      name: 'mixed-4e6', kind: 'mixed', size: 4000000, ops: 1000000,
      label: 'mixed union/find/connected (50/25/25)'
    }
  ],
  'sparse-set': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed add/has/delete (50/25/25)'
    },
    {
      name: 'mixed-4e6', kind: 'mixed', size: 4000000, ops: 1000000,
      label: 'mixed add/has/delete (50/25/25)'
    },
    {
      name: 'drain-1e5', kind: 'drain', size: 100000, passes: 100,
      label: 'full iteration of a prefilled set, one timed sample per walk'
    }
  ],

  // The five modules added for the Gate 10 extension past the first two.
  // Each is one workload rather than a size sweep -- `static-disjoint-set`'s
  // second row exists because a boundary was found by probing; nothing here
  // has (yet) shown the same signal, and adding a second row per module on
  // spec would cost 5 more single-process passes each without a documented
  // reason, which is the shape of padding a table rather than reporting one.
  //
  // `bit-set`: capacity IS the domain, same as sparse-set/static-disjoint-set
  // -- a fixed-length structure has no separate "key space" to rig.
  'bit-set': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed set/reset/get/test (50/25/25); rank excluded -- see ' +
        'bench/runner/src/bit_set.rs for why a 25%-weighted rank blew up wall-clock time'
    }
  ],

  // `lru-cache`: `size` is the KEY DOMAIN, not the capacity -- capacity is
  // derived inside bench/runner/src/lru_cache.rs and bench/node/run.js's
  // `capacityForLru` as a fixed 20% of it. 1e6 keys / 200,000-entry cache is
  // large enough that the cache is genuinely under eviction pressure for the
  // whole run rather than filling once and idling; see that file's module
  // docs for why 100% (capacity == domain) and single-digit-percent
  // capacities are both the wrong answer, and for the honest limit of what
  // "hit rate" means under this benchmark's uniform access pattern.
  'lru-cache': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed set/get/has (50/25/25), capacity 20% of the 1e6 key domain'
    }
  ],

  // `heap`: `size` bounds the numeric range pushed values are drawn from, not
  // a capacity -- a heap grows under push like `vector`/`trie` do. 1e6 keeps
  // most pushed values distinct (see bench/runner/src/heap.rs), so ties --
  // which would let the comparator return early without moving anything --
  // stay rare.
  'heap': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed push/pop/peek (50/25/25), default numeric comparator'
    }
  ],

  // `trie`: `size` is the domain the hex-encoded keys are drawn from, kept an
  // order of magnitude below the other modules' 1e6 on purpose. Every
  // distinct value here is a multi-node walk with a hash map fan-out per
  // node, not a flat array index, so an equal domain would make this module
  // by far the slowest wall-clock component of a Gate 10 pass for a
  // comparison that does not need the extra size to be representative --
  // 200,000 keys already exercises deep sharing (values under 0x1000 alone
  // number in the thousands) without turning one workload into most of the
  // batch's runtime.
  'trie': [
    {
      name: 'mixed-2e5', kind: 'mixed', size: 200000, ops: 1000000,
      label: 'mixed add/has/delete (50/25/25) over hex-encoded keys'
    }
  ],

  // `vector`: `size` only bounds the magnitude of pushed values -- a growable
  // array has no capacity distinct from its (self-managed) length, so the
  // number itself is arbitrary and kept at 1e6 for consistency with the other
  // modules rather than for any effect on the measurement.
  'vector': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed push/get/pop (50/25/25)'
    }
  ],

  // The eleven modules added for the sequence-backed Gate 10 batch. Each is
  // one workload, same reasoning as the first extension past the original
  // two: a size sweep costs one more single-process pass per module without
  // a documented signal to justify it, which is the shape of padding a table
  // rather than reporting one.
  //
  // `stack`/`queue`: no capacity distinct from pushed/enqueued length, same
  // as `vector` -- `size` only bounds magnitude.
  'stack': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed push/peek/pop (50/25/25)'
    }
  ],
  'queue': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed enqueue/peek/dequeue (50/25/25)'
    }
  ],

  // `fixed-stack`/`fixed-deque`/`circular-buffer`: `size` IS the capacity.
  // 10,000 against 1e6 ops is a 100:1 ratio -- the structure fills within the
  // first ~2% of the run (50% push, guarded by size<capacity for the two that
  // can refuse) and spends the remaining ~98% oscillating at or near
  // capacity, which is "reached and sustained" rather than "reached once at
  // the very end". See each bench/runner/src/*.rs file's own module docs for
  // why `push` is guarded on the two that can refuse, and why it is not
  // guarded on `circular-buffer`.
  'fixed-stack': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 10000, ops: 1000000,
      label: 'mixed push/peek/pop (50/25/25), capacity reached and held'
    }
  ],
  'fixed-deque': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 10000, ops: 1000000,
      label: 'mixed push/peekLast/pop (50/25/25), capacity reached and held'
    }
  ],
  'circular-buffer': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 10000, ops: 1000000,
      label: 'mixed push/peekLast/pop (50/25/25), capacity reached and overwriting'
    }
  ],

  // `hashed-array-tree`/`bit-vector`: no capacity distinct from pushed
  // length, same reasoning as `vector`/`hashed-array-tree` share.
  'hashed-array-tree': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed push/get/pop (50/25/25)'
    }
  ],
  'bit-vector': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed push/get/pop (50/25/25); rank/select excluded, same reason as bit-set'
    }
  ],

  // `sparse-map`/`sparse-queue-set`: `size` is the domain, same as
  // `sparse-set`/`bit-set` -- a fixed-length structure has no separate key
  // space to rig.
  'sparse-map': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed set/get/delete (50/25/25)'
    }
  ],
  'sparse-queue-set': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed enqueue/has/dequeue (50/25/25)'
    }
  ],

  // `sort`/`suffix-array`: functions/one-shot construction, not op streams --
  // see each file's own bench/runner/src/*.rs module docs. Both reuse the
  // `drain` kind, one measured sample per sort/construction rather than per
  // element. 20,000 elements/characters times 50 passes keeps the total work
  // per measured sample at the same 1e6 order of magnitude as the mixed
  // workloads above, for a comparable per-workload wall-clock cost -- sanity
  // checked (DESIGN.md 5.1's own lesson, repeated in bit_set.rs's `rank`
  // account): a single process invocation at these parameters completes in
  // well under three seconds on both sides before this was committed to.
  'sort': [
    {
      name: 'sort-2e4x50', kind: 'drain', size: 20000, passes: 50,
      label: 'quicksort of a freshly-generated random array, one timed sample per sort'
    }
  ],
  'suffix-array': [
    {
      name: 'build-2e4x50', kind: 'drain', size: 20000, passes: 50,
      label: 'DC3 construction over a freshly-generated 4-symbol random text, one timed sample per build'
    }
  ],

  // The ten map-like/multi-container modules added for this batch. Each is
  // one workload, same reasoning as both prior extensions: a size sweep
  // costs one more single-process pass per module without a documented
  // signal to justify it. `default-weak-map` is deliberately absent -- its
  // keys must be objects and entries vanish at the GC's discretion, so
  // timing would be dominated by allocation and GC rather than by the
  // structure; see planning/NOTES.md and docs/modules/default-weak-map.md
  // for the same call made about GC timing elsewhere in this project.
  //
  // `default-map`/`bi-map`: `size` IS the full key domain, same reasoning as
  // `sparse-map`/`bit-set` -- a hash map (or a pair of them, for `bi-map`)
  // has no separate index to rig.
  'default-map': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed set/get-or-insert/delete (50/25/25); factory always defined, so B-40\'s size drift never fires here'
    },
    // `mixed-1e6` lost on p50/p99 -- the sharpest margin in this batch (§5.1:
    // "expect to lose somewhere and report it," and a clean sweep invites the
    // question of what was left out). Probed at 4x domain, `static-disjoint-
    // set`'s own convention for telling a real boundary from noise, before
    // publishing either figure as the final word.
    {
      name: 'mixed-4e6', kind: 'mixed', size: 4000000, ops: 1000000,
      label: 'mixed set/get-or-insert/delete (50/25/25); factory always defined, so B-40\'s size drift never fires here'
    }
  ],
  'bi-map': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed set/get/delete (50/25/25); key and value share one domain so the bijection\'s rebinding path fires under load'
    }
  ],

  // `multi-map`/`multi-set`/`multi-array`: the load-bearing parameter for
  // every multi-container is how many VALUES sit under one key, so `size`
  // here is the key/item/index domain, deliberately far smaller than the
  // 1e6 op count -- see each bench/runner/src/*.rs file's own module docs
  // for the exact values-per-key figure this reaches.
  'multi-map': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 20000, ops: 1000000,
      label: 'mixed set/get/remove (50/25/25) over a 20,000-key domain; ~25 values/key by the run\'s end'
    }
  ],
  'multi-set': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 20000, ops: 1000000,
      label: 'mixed add/multiplicity/remove (50/25/25) over a 20,000-item domain; ~12.5 net multiplicity/item by the run\'s end'
    }
  ],
  'multi-array': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 20000, ops: 1000000,
      label: 'mixed set/get/multiplicity (50/25/25) over a 20,000-index domain; ~25 values/bucket by the run\'s end'
    }
  ],

  // `fuzzy-map`/`fuzzy-multi-map`: both hash `x >> 4` (16:1 collapse) on
  // both sides -- see each bench/runner/src/*.rs file's own module docs for
  // why the hash must do identical work on both sides to avoid measuring the
  // hash instead of the structure. `fuzzy-map`'s domain is the full 1e6 (the
  // hash's own collapse is what produces the collision-heavy access
  // pattern); `fuzzy-multi-map`'s is smaller (200,000) so its post-hash
  // domain (12,500) reaches a representative values-per-key figure.
  'fuzzy-map': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed set/get/has (50/25/25), hash(x) = x >> 4, 16:1 key collapse'
    }
  ],
  'fuzzy-multi-map': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 200000, ops: 1000000,
      label: 'mixed set/get/has (50/25/25), hash(x) = x >> 4; ~40 values/key over the ~12,500-key post-hash domain'
    }
  ],

  // `inverted-index`: `size` is the token VOCABULARY, not the doc count, and
  // `ops` is 200,000 rather than this batch's usual 1e6 -- both deliberately
  // smaller, sanity-checked before committing to them (the `bit-set.rs`
  // `rank` lesson `methodology.md` documents): at this batch's usual 1e6/1e6
  // shape, posting lists would average ~1,000 documents and a two-token
  // query would cost ~25x what it does here, for no additional signal.
  'inverted-index': [
    {
      name: 'mixed-2e5', kind: 'mixed', size: 1000, ops: 200000,
      label: 'mixed add(2-token doc)/get(1-token query)/get(2-token AND query) (50/25/25) over a 1,000-word vocabulary; ~200 docs/token by the run\'s end'
    }
  ],

  // `set`: no instance and no per-element op stream (see
  // bench/runner/src/set_ops.rs's own module docs), so this reuses the
  // `drain` shape -- one measured sample per `union` call, the representative
  // choice out of set.js's fourteen free functions. `size * passes` at the
  // same ~1e6 order of magnitude as `sort`/`suffix-array` above, for a
  // comparable per-workload wall-clock cost.
  'set': [
    {
      name: 'union-2e4x50', kind: 'drain', size: 20000, passes: 50,
      label: 'union(A, B) of two 20,000-element sets drawn from a shared domain (real overlap and duplicates by the birthday bound), one timed sample per call'
    }
  ],

  // The final fourteen units. Appended, not inserted -- same reasoning as
  // every prior batch.
  //
  // `trie-map`: same domain as `trie`'s own workload and the same reasoning
  // -- `size` is the hex-key domain, kept an order of magnitude below the
  // flat-structure modules' 1e6 so the prefix-sharing shape (every value
  // under 0x1000 shares its leading digits with thousands of others) stays
  // the dominant cost rather than sheer key count.
  'trie-map': [
    {
      name: 'mixed-2e5', kind: 'mixed', size: 200000, ops: 1000000,
      label: 'mixed set/get/delete (50/25/25) over hex-encoded keys'
    }
  ],

  // `critbit-tree-map`: zero-padded 6-digit decimal keys over a 200,000-key
  // domain -- same order-of-magnitude reasoning as `trie-map`'s own domain,
  // and the padding is what forces most key pairs to diverge deep in the
  // key rather than at the first byte; see bench/runner/src/
  // critbit_tree_map.rs's own module docs for the full account.
  'critbit-tree-map': [
    {
      name: 'mixed-2e5', kind: 'mixed', size: 200000, ops: 1000000,
      label: 'mixed set/get/delete (50/25/25) over zero-padded decimal keys, forcing deep critical-bit positions'
    }
  ],

  // `fixed-critbit-tree-map`: no `delete` (upstream has none), so this is
  // `fuzzy-map`'s set/get/has shape. `size` is BOTH the capacity and the
  // full key domain -- load-bearing, not a style choice: upstream's `set`
  // has no capacity guard, and a distinct key past capacity silently
  // corrupts the tree and later throws. See bench/runner/src/
  // fixed_critbit_tree_map.rs's own module docs.
  'fixed-critbit-tree-map': [
    {
      name: 'mixed-2e5', kind: 'mixed', size: 200000, ops: 1000000,
      label: 'mixed set/get/has (50/25/25) over zero-padded decimal keys, capacity reached and held'
    }
  ],

  // `bk-tree`: `size` 300,000 against 1e6 ops, found by sanity-checking two
  // failure modes first (a too-small domain that goes superlinear via
  // duplicate-chain growth, a too-large one that collapses the tree to
  // depth 1) -- see bench/runner/src/bk_tree.rs's own module docs for the
  // measurements that ruled each out.
  'bk-tree': [
    {
      name: 'mixed-3e5', kind: 'mixed', size: 300000, ops: 1000000,
      label: 'mixed add/search-small-radius/search-large-radius (50/25/25), metric |a - b|, over a 300,000-item domain'
    }
  ],

  // `vp-tree`: no `add` at all -- the tree is built once (untimed) from a
  // shuffled 0..size, then every op is a query. `size` 50,000, well below
  // `bk-tree`'s domain: construction sorts by distance from a vantage point
  // at every level, and that cost was measured to be superlinear even after
  // fixing the "sequential input" trap (a standalone probe against
  // upstream's own vp-tree.js confirmed the remainder is a genuine property
  // of the ported algorithm, not a Rust-only regression) -- see
  // bench/runner/src/vp_tree.rs's own module docs for the full account.
  // `ops` 200,000, not this batch's usual 1e6, for the same reason.
  'vp-tree': [
    {
      name: 'mixed-5e4', kind: 'mixed', size: 50000, ops: 200000,
      label: 'mixed neighbors-small-radius/neighbors-large-radius/nearestNeighbors (40/40/20), metric |a - b|, over a shuffled 50,000-item domain'
    }
  ],

  // `kd-tree`: no `add` -- the tree is built once (untimed) from `size`
  // scattered 2-D points, then every op is a query. Unlike `bk-tree`/
  // `vp-tree`, a single query shape already exercises both outcomes of the
  // cross-plane backtrack for genuine 2-D data, so no second radius is
  // needed -- see bench/runner/src/kd_tree.rs's own module docs.
  'kd-tree': [
    {
      name: 'mixed-1e5', kind: 'mixed', size: 100000, ops: 1000000,
      label: 'mixed nearestNeighbor/kNearestNeighbors (75/25) over 100,000 scattered 2-D points'
    }
  ],

  // `static-interval-tree`: no `add` -- the tree is built once (untimed)
  // from 100,000 overlapping intervals, then every op is a query. Interval
  // width is 0.1% of the domain, not the 10% first tried -- see
  // bench/runner/src/static_interval_tree.rs's own module docs for the
  // measurement (22 seconds for a 200,000-op pass) that ruled the larger
  // fraction out.
  'static-interval-tree': [
    {
      name: 'mixed-1e5', kind: 'mixed', size: 100000, ops: 1000000,
      label: 'mixed intervalsContainingPoint/intervalsOverlappingInterval (50/50) over 100,000 overlapping intervals'
    }
  ],

  // `fibonacci-heap`: same shape as `heap`'s own workload -- 50/25/25
  // push/pop/peek, `size` bounding the pushed range so values stay mostly
  // distinct. `size`/`ops` are 200,000, not this batch's usual 1e6: a 1e6-op
  // pass was timed by hand first and upstream took over 2 minutes, dominated
  // by system time rather than user CPU (heavy memory churn, not algorithmic
  // cost) -- see bench/runner/src/fibonacci_heap.rs's own module docs. The
  // load-bearing check is `FibonacciHeap::merges`: measured at 195,920 merges
  // over 50,000 pops for this exact op mix at 200,000 ops, confirming
  // consolidation fires repeatedly rather than degenerating to "pop one
  // thing, link nothing".
  'fibonacci-heap': [
    {
      name: 'mixed-2e5', kind: 'mixed', size: 200000, ops: 200000,
      label: 'mixed push/pop/peek (50/25/25), default numeric comparator'
    }
  ],

  // `fixed-reverse-heap`: capacity is HALF the value domain, not a tiny
  // slice of the op count -- see bench/runner/src/fixed_reverse_heap.rs's
  // own module docs for why that is load-bearing (a tiny capacity fills
  // once and then rarely displaces anything again). Measured: 60.3%
  // displacement rate over full-heap pushes.
  'fixed-reverse-heap': [
    {
      name: 'mixed-1e6', kind: 'mixed', size: 1000000, ops: 1000000,
      label: 'mixed push/peek (75/25), capacity = size/2, default numeric comparator'
    }
  ],

  // `bloom-filter`: prefilled to a stated 50% fill ratio (untimed), climbing
  // toward ~1.0 over the run via the `add` stream. Measured: 61.1% hit rate
  // on the hit pool, 0.028% false-positive rate on the miss pool -- see
  // bench/runner/src/bloom_filter.rs's own module docs.
  'bloom-filter': [
    {
      name: 'mixed-2e5', kind: 'mixed', size: 200000, ops: 200000,
      label: 'mixed add/test-hit/test-miss (50/25/25), prefilled to 50% fill ratio'
    }
  ]
};

function measure(workload, baselines, startups) {
  const collected = {port: [], original: []};
  const checksums = new Set();
  let meta = null;

  // Rule 4: strictly A/B/A/B. Each measured pass is its own process, so each
  // gets its own WARMUP passes -- V8's JIT state does not survive a process,
  // so warming once and measuring ten times would only be honest for Rust.
  for (let rep = 1; rep <= REPS; rep++) {
    for (const side of ['port', 'original']) {
      const args = [
        '--module', module_,
        '--kind', workload.kind,
        '--size', String(workload.size),
        '--warmup', String(WARMUP),
        '--measured', '1'
      ].concat(workload.kind === 'drain'
        ? ['--passes', String(workload.passes)]
        : ['--ops', String(workload.ops)]);

      const raw = side === 'port'
        ? run(RUST, args)
        : run(process.execPath, [NODE_RUNNER].concat(args));

      const result = JSON.parse(raw);

      collected[side].push(result);
      checksums.add(result.checksum);
      meta = meta || result;
    }

    process.stdout.write('  ' + workload.name + ': rep ' + rep + '/' + REPS + '\r');
  }

  console.log('');

  // Same ops AND same answers on both sides. If this trips, no timing number
  // below it means anything, so it is a hard stop rather than a warning.
  if (checksums.size !== 1) {
    throw new Error(
      'checksums differ across runs/sides: ' + Array.from(checksums).join(', ') +
      '\nThe two implementations did not compute the same answers, so a timing ' +
      'comparison between them is meaningless. Fix the divergence first.'
    );
  }

  const structures = {
    port: structureRss('port', workload.size),
    original: structureRss('original', workload.size)
  };

  // `sparse-set`'s drain narrative is specific to what it measures (a
  // prefilled set, walked): "elements yielded", "distinct members" and
  // "walks" are all its vocabulary, not the shape `sort`/`suffix-array` added
  // later share (one sort, or one construction, per measured sample -- see
  // each bench/runner/src/*.rs file's own module docs). Rather than stretch
  // sparse-set's sentence to cover operations it does not describe, drain
  // workloads other than sparse-set's own get a description built from
  // `workload.label` (which already says what is being measured) plus the
  // mechanical facts every drain shares: total ops, passes, size, seed.
  const drainWorkload = module_ === 'sparse-set'
    ? si(meta.ops) + ' elements yielded: ' + workload.label + '. Set of length ' +
      si(meta.size) + ' prefilled by ' + si(meta.size) +
      ' random adds (xorshift32 seed ' + meta.seed + '), leaving ' +
      meta.batch_k + ' distinct members; ' + workload.passes +
      ' walks per measured pass, one timed sample each'
    : si(meta.ops) + ' total (' + workload.passes + ' passes × ' + meta.batch_k +
      ' per pass): ' + workload.label + ', size ' + si(meta.size) +
      ', xorshift32 seed ' + meta.seed;

  const entry = {
    workload: workload.kind === 'drain'
      ? drainWorkload
      : si(meta.ops) + ' ' + workload.label + ' over size ' + si(meta.size) +
        ', xorshift32 seed ' + meta.seed +
        ', ops materialised before the timed region',
    checksum: meta.checksum,
    checksum_note: 'identical on both sides: same ops, same answers, upstream bugs included',
    structure_note: 'structure_rss_delta_mb constructs size ' + si(meta.size) +
      ' and nothing else, isolating the structure from the op arrays both sides materialise'
  };

  for (const side of ['port', 'original']) {
    const batches = [].concat.apply([], collected[side].map((r) => r.batch_ns));
    // Peak RSS is a high-water mark, so the max across passes is the figure.
    const rss = Math.max.apply(null, collected[side].map((r) => r.rss_kb));

    entry[side] = summarise(
      batches, meta.batch_k, rss, baselines[side], startups[side], structures[side]
    );
  }

  entry.regressions = regressions(entry);

  return entry;
}

function main() {
  const verified = verifyPrng();
  console.log('matched PRNG verified: first ' + verified + ' values identical');

  const baselines = {port: baseline('port'), original: baseline('original')};
  console.log('rss baselines (kb): port=' + baselines.port + ' original=' + baselines.original);

  const startups = startup();
  const planned = WORKLOADS[module_];

  if (!planned) {
    throw new Error('no workloads defined for module `' + module_ + '`');
  }

  const workloads = {};

  for (const workload of planned) {
    workloads[workload.name] = measure(workload, baselines, startups);
  }

  const results = fs.existsSync(RESULTS)
    ? JSON.parse(fs.readFileSync(RESULTS, 'utf8'))
    : {};

  results.methodology = 'bench/methodology.md';
  results.host = host();
  results.protocol = {
    warmup: WARMUP,
    measured: REPS,
    batch_k: 1000,
    interleaved: true,
    percentile: 'nearest-rank, computed once in bench/drive.js over both sides',
    criterion: false,
    through_napi: false,
    rss: 'in-process: getrusage(RUSAGE_SELF) in Rust, process.resourceUsage().maxRSS in Node'
  };
  results.baseline_rss_mb = {
    node: round(baselines.original / 1024, 1),
    rust: round(baselines.port / 1024, 1)
  };
  results.modules = results.modules || {};
  results.modules[module_] = {workloads: workloads};
  results.generated_utc = new Date().toISOString();

  fs.writeFileSync(RESULTS, JSON.stringify(results, null, 2) + '\n');

  console.log(JSON.stringify(workloads, null, 2));

  for (const name of Object.keys(workloads)) {
    const worse = workloads[name].regressions;

    console.log(name + ': ' + (worse.length === 0
      ? 'no regressions'
      : 'REGRESSIONS -> ' + worse.map((r) => r.metric + ' ' + r.ratio + 'x').join(', ')));
  }

  console.log('written to bench/results.json');
}

main();

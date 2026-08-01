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

  const entry = {
    workload: workload.kind === 'drain'
      ? si(meta.ops) + ' elements yielded: ' + workload.label + '. Set of length ' +
        si(meta.size) + ' prefilled by ' + si(meta.size) +
        ' random adds (xorshift32 seed ' + meta.seed + '), leaving ' +
        meta.batch_k + ' distinct members; ' + workload.passes +
        ' walks per measured pass, one timed sample each'
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

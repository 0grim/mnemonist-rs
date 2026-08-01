#!/usr/bin/env node
//
// Persistent differential-fuzzing oracle.
//
// Speaks line-delimited JSON on stdin/stdout, one response per request, and
// holds a single live upstream instance between requests. DESIGN.md 4 is
// explicit about why: spawning `node` per operation turns a 60-second fuzz run
// into an hour of process startup. One process, one pipe, no per-op cost
// beyond a round trip.
//
// The oracle is module-agnostic. Everything it needs — which upstream file to
// require, the constructor arguments, and which properties/nullary methods
// make up the observable state — arrives in the `init` request, so adding a
// module means adding a `ModuleSpec` on the Rust side and nothing here.
//
// Protocol
// --------
//   -> {"cmd":"init","module":"static-disjoint-set","ctor":[10],
//       "observe":["size","dimension","mapping","compile"]}
//   <- {"ok":true,"state":{...}}
//
//   -> {"cmd":"op","name":"union","args":[0,1]}
//   <- {"ok":true,"result":{"$self":true},"state":{...}}
//
//   -> {"cmd":"ping"}          <- {"ok":true}
//   -> {"cmd":"quit"}          (no response; process exits)
//
// Cursor lifecycle ops (DESIGN.md 3.4/3.7, D-21). An op name starting with `$`
// is not a method on the instance; it drives the ONE cursor the oracle keeps
// alongside it. This is what lets a generated program interleave iteration
// with mutation, which is the only way D-06/D-08/D-09 are reachable at all.
//
//   -> {"cmd":"op","name":"$iter","args":["values"]}
//   <- {"ok":true,"result":{"$iterator":true},"state":{...}}
//
//   -> {"cmd":"op","name":"$next","args":[]}
//   <- {"ok":true,"result":{"done":false,"value":3},"state":{...}}
//
//   -> {"cmd":"op","name":"$spread","args":[]}
//   <- {"ok":true,"result":[3,6,9],"state":{...}}
//
// `$next` normalises the step, because the two sides shape it differently and
// neither difference is meaningful: obliterator returns `{value: x}` with no
// `done` key for an item and `{done: true}` with no `value` for the end, while
// a napi generator always sets both. `{done: <bool>, value: <encoded>}` is
// what both actually mean.
//
// `$spread` is `Array.from(instance)`, which goes through the COLLECTION's
// Symbol.iterator rather than a stored cursor — the factory half of D-07. It
// is a separate op precisely because it must construct a fresh cursor every
// time while `$next` must not.
//
// Any thrown error is reported as {"ok":false,"error":"..."} rather than
// killing the process, so a divergence in error behaviour is comparable data
// rather than a dead oracle.
'use strict';

const fs = require('fs');
const Module = require('module');
const path = require('path');
const readline = require('readline');

const UPSTREAM = path.resolve(__dirname, '..', 'bench', 'upstream');

// `bench/upstream/` is vendored source, not an installed package, so it has no
// `node_modules` of its own — and from `sparse-set.js` onwards the upstream
// files `require('obliterator/...')` at load time. Point Node's global
// resolution at the harness's installed dependencies, which `tests/run.sh`
// creates. Pinning it here rather than in the Rust side keeps the oracle
// runnable by hand.
const HARNESS_MODULES = path.resolve(__dirname, '..', 'tests', '.work', 'node_modules');

if (fs.existsSync(HARNESS_MODULES)) {
  process.env.NODE_PATH = process.env.NODE_PATH
    ? HARNESS_MODULES + path.delimiter + process.env.NODE_PATH
    : HARNESS_MODULES;
  Module._initPaths();
}

let instance = null;
let observations = [];
let cursor = null;

// JSON has no typed arrays, no `undefined`, and no NaN, and all three are
// observably distinct in JS. Encode them so the Rust side can reproduce the
// exact same document instead of comparing lossily.
function encode(value) {
  if (value === instance && instance !== null) return {$self: true};
  if (value === undefined) return {$undefined: true};
  if (value === null) return null;

  if (ArrayBuffer.isView(value)) {
    return {$typed: value.constructor.name, values: Array.from(value)};
  }

  if (Array.isArray(value)) return value.map(encode);

  if (typeof value === 'number') {
    if (Number.isNaN(value)) return {$nan: true};
    if (!Number.isFinite(value)) return {$infinity: Math.sign(value)};
    return value;
  }

  if (typeof value === 'object') {
    const out = {};
    for (const key of Object.keys(value)) out[key] = encode(value[key]);
    return out;
  }

  return value;
}

// Some constructors take another CONSTRUCTOR as an argument: `SparseMap` is
// `new SparseMap(Values, length)`, where `Values` is `Array` or a typed-array
// constructor. JSON cannot carry a function, so the Rust side sends
// `{"$global": "Uint8Array"}` and it is resolved here, against the real global
// rather than by name-matching a lookalike.
//
// Deliberately narrow: only `init`'s ctor arguments go through this, never an
// op's arguments. An op that could smuggle in an arbitrary global would make
// the generated programs unbounded in a way the Rust side cannot mirror.
function decodeCtorArg(arg) {
  if (arg === null || typeof arg !== 'object' || typeof arg.$global !== 'string') {
    return arg;
  }

  const resolved = globalThis[arg.$global];

  if (typeof resolved === 'undefined') {
    throw new Error('unknown global in ctor argument: ' + arg.$global);
  }

  return resolved;
}

function observe() {
  const state = {};

  for (const name of observations) {
    const member = instance[name];
    state[name] = encode(typeof member === 'function' ? member.call(instance) : member);
  }

  return state;
}

// The `$` ops. `cursor` is deliberately singular: one stored iterator is
// enough to express create/step/mutate interleavings, and the Rust side has to
// mirror whatever this holds.
function cursorOp(request) {
  switch (request.name) {
    case '$iter':
      // Replaces any previous cursor, so a program can re-open mid-walk.
      cursor = instance[request.args[0]]();
      return {$iterator: true};

    case '$next': {
      // Stepping before `$iter` is legal in the grammar and must be reported
      // rather than thrown, so that both sides agree on the same non-event.
      if (cursor === null) return {$noIterator: true};

      const step = cursor.next();

      return {done: step.done === true, value: encode(step.value)};
    }

    case '$spread':
      return Array.from(instance).map(encode);

    default:
      throw new Error('unknown cursor op: ' + request.name);
  }
}

function handle(request) {
  switch (request.cmd) {
    case 'ping':
      return {ok: true};

    case 'init': {
      const Ctor = require(path.join(UPSTREAM, request.module + '.js'));
      instance = new Ctor(...request.ctor.map(decodeCtorArg));
      observations = request.observe;
      cursor = null;
      return {ok: true, state: observe()};
    }

    case 'op': {
      if (instance === null) throw new Error('op before init');

      if (request.name.charAt(0) === '$')
        return {ok: true, result: cursorOp(request), state: observe()};

      let result;

      // An exception thrown BY AN OPERATION is a comparable result, not
      // apparatus failure. Reporting it as {ok:false} would reach the Rust
      // side as OracleError and ABORT the campaign, when "upstream threw and
      // the port did not" is precisely the divergence worth catching. See the
      // "Trap for the next module" note on spec::CheckFailure, written before
      // there was a module that throws; hashed-array-tree is that module.
      try {
        result = encode(instance[request.name](...request.args));
      } catch (error) {
        result = {$throw: String(error && error.message ? error.message : error)};
      }

      return {ok: true, result: result, state: observe()};
    }

    default:
      throw new Error('unknown command: ' + request.cmd);
  }
}

const lines = readline.createInterface({input: process.stdin});

lines.on('line', (line) => {
  if (line.length === 0) return;

  let request;

  try {
    request = JSON.parse(line);
  } catch (error) {
    process.stdout.write(JSON.stringify({ok: false, error: 'bad request: ' + error.message}) + '\n');
    return;
  }

  if (request.cmd === 'quit') {
    lines.close();
    return;
  }

  let response;

  try {
    response = handle(request);
  } catch (error) {
    response = {ok: false, error: String(error && error.message ? error.message : error)};
  }

  process.stdout.write(JSON.stringify(response) + '\n');
});

lines.on('close', () => process.exit(0));

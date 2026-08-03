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
// Cursor lifecycle ops (DESIGN.md 3.4/3.7, DIV-PROJ-21). An op name starting with `$`
// is not a method on the instance; it drives the ONE cursor the oracle keeps
// alongside it. This is what lets a generated program interleave iteration
// with mutation, which is the only way DIV-STACK-1/DIV-PROJ-10/DIV-SPARSE-SET-1 are reachable at all.
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
// Symbol.iterator rather than a stored cursor — the factory half of DIV-STACK-2. It
// is a separate op precisely because it must construct a fresh cursor every
// time while `$next` must not.
//
//   -> {"cmd":"op","name":"$forEach","args":["delete","arg0",1]}
//   <- {"ok":true,"result":{"seen":[[1,1],[2,2]]},"state":{...}}
//
// `$forEach` walks the instance and, from inside the callback, calls one of
// its own mutating methods. `args` is `[method, rule, limit]`: the method to
// call (or `null` for a plain walk), how to build its arguments out of the
// callback's own, and how many times it may fire. `result` is the list of
// callback argument pairs, so the walk's SHAPE is compared and not only the
// state it leaves behind.
//
// The third callback argument -- the collection itself, where a module passes
// one -- is deliberately not recorded: it encodes as `{"$self": true}` on
// every step and can never disagree.
// Free-function modules (DESIGN.md 1.1's `sort` and `set` units). `init` may
// name a LIST OF FILES instead of one constructor; their exports are merged
// and `instance` becomes that object, so `instance[name](...)` still
// dispatches and nothing else in the protocol changes.
//
//   -> {"cmd":"init","module":"sort","observe":[],
//       "functions":["sort/insertion","sort/quick","utils/typed-arrays"]}
//   <- {"ok":true,"state":{}}
//
// Such a module has NO observable state, so `state` is `{}` forever and the
// whole comparison would otherwise rest on the return value. That is not
// enough — `set.js`'s `add`/`subtract`/`intersect`/`disjunct` return
// `undefined` and mutate their first argument, and `sort/*.js` sorts in place
// — so an op's result is wrapped and the arguments are echoed back after the
// call:
//
//   -> {"cmd":"op","name":"inplaceQuickSort","args":[[3,1,2],0,3]}
//   <- {"ok":true,"result":{"$return":[1,2,3],"$args":[[1,2,3],0,3]},"state":{}}
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
// True when `instance` is a module object of free functions rather than a
// constructed structure. See the `init` and `op` handlers.
let functionsModule = false;

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

  // Indexed rather than `value.map(encode)`, because `map` SKIPS holes and
  // leaves them holes, which `JSON.stringify` then writes as `null` -- while an
  // explicitly assigned `undefined` in the same array becomes `{$undefined}`.
  // The two are indistinguishable through every API the heaps expose (`a[i]` is
  // `undefined` either way), so encoding them differently would be a false
  // divergence waiting to happen. `heap` is the first module that can produce a
  // hole at all: a comparator that shrinks the array mid-sift makes the sift
  // write past the new end. No earlier module has a sparse observation, so this
  // changes nothing for them.
  if (Array.isArray(value)) {
    const out = new Array(value.length);
    for (let i = 0; i < value.length; i++) out[i] = encode(value[i]);
    return out;
  }

  // A `Map` has no own enumerable properties, so the generic object branch
  // below would encode every T3 module's entire state as `{}` -- an
  // observation that can never disagree. Entry order is part of what is being
  // compared, so entries go out as a list, not as an object.
  if (value instanceof Map) {
    return {$map: Array.from(value.entries()).map(([k, v]) => [encode(k), encode(v)])};
  }

  // A `Set` has no own enumerable properties either, and `set.js`'s functions
  // both take and return them. Insertion order is asserted by the original
  // test file in eight of its fourteen blocks, so members go out as a list.
  if (value instanceof Set) {
    return {$set: Array.from(value).map(encode)};
  }

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

    case '$forEach':
      return forEachOp(request);
    default:
      throw new Error('unknown cursor op: ' + request.name);
  }
}

// How a `$forEach` mutation's arguments come out of the callback's own.
// Mirrored by `spec::for_each_args` on the Rust side; the two must agree
// exactly or every campaign is a false divergence.
//
// SELECTION IS SEPARATE FROM ARITHMETIC, and the order is load-bearing. The
// first version of this table folded `+ 1` into the selection, so an
// `undefined` argument became `NaN` -- which is not `undefined`, sailed
// through the skip below, and reached `SparseSet.add(NaN)`, where upstream's
// `sparse[NaN]` comparison falls through and increments `size`. Caught by the
// first 20-second campaign after the op was added; the minimised seed is in
// crates/difffuzz/proptest-regressions/sparse-set.txt and is kept, so the
// ordering stays pinned.
const FOR_EACH_RULES = {
  'none': function () { return []; },
  'arg0': function (a) { return [a[0]]; },
  'arg0+1': function (a) { return [a[0]]; },
  'arg1': function (a) { return [a[1]]; },
  'arg1,arg0': function (a) { return [a[1], a[0]]; },
};

// Applied AFTER the undefined skip, never before.
function forEachArithmetic(rule, args) {
  return rule === 'arg0+1' ? [args[0] + 1] : args;
}

// `forEach`, with a callback that calls back into the collection it is
// walking.
//
// Two deliberate narrowings, both mirrored on the Rust side and both recorded
// in fuzz/log.txt:
//
//   1. A selected argument that is `undefined` skips the mutation. Passing it
//      on is legal JS and reaches upstream's NaN-indexed swap, which
//      `mnemonist-core` does not model -- see `spec::for_each_args`.
//   2. An exception thrown by the walk is reported as `$throw` ALONGSIDE the
//      steps taken before it, not instead of them. `BitVector.push` can throw
//      from a growth policy, and losing the prefix would make the two sides
//      agree on strictly less than they know.
function forEachOp(request) {
  const method = request.args[0];
  const name = request.args[1] || 'none';
  const rule = FOR_EACH_RULES[name];
  const limit = request.args[2] || 0;

  if (!rule) throw new Error('unknown $forEach rule: ' + name);

  const seen = [];
  let fired = 0;

  try {
    instance.forEach(function () {
      // Only the first two: the third is the collection itself.
      const received = Array.prototype.slice.call(arguments, 0, 2);

      seen.push(received.map(encode));

      if (method === null || method === undefined || fired >= limit) return;

      const selected = rule(received);

      if (selected.some(function (value) { return value === undefined; })) return;

      fired++;
      instance[method].apply(instance, forEachArithmetic(name, selected));
    });
  }
  catch (error) {
    return {seen: seen, $throw: String(error && error.message ? error.message : error)};
  }

  return {seen: seen};
}

// Arguments travel as JSON, which has no `undefined`, no `-0`, no NaN and no
// functions. All four are ordinary inputs to a T3 module -- `undefined` is the
// value that reaches DefaultMap's size drift, `-0` and NaN are the two places
// SameValueZero differs from `===`, and a factory IS a function. So the same
// envelopes `encode` produces are recognised on the way in.
//
// Factories are named rather than transmitted as source: a generated program
// has to be reproducible from its seed, and `eval` of a generated string is
// neither reproducible nor readable in a repro.
// Builders, not functions: `autoIncrement` is stateful, and one shared counter
// would make a program's result depend on the programs that ran before it.
// Every `init` gets a fresh one.
const FACTORIES = {
  undefined: () => () => undefined,
  null: () => () => null,
  autoIncrement: () => {
    let next = 0;
    return () => next++;
  },
  key: () => (key) => key,
  size: () => (key, size) => size,
  // Appended at the end, never inserted: this table is a shared registry
  // (CLAUDE.md, Git) and a new key anywhere else is a merge conflict inside
  // an object literal. `fuzzy*` names are prefixed so `fuzzy-map`'s campaign
  // cannot collide with `default-map`'s factory names above.
  fuzzyIdentity: () => (x) => x,
  fuzzyLower: () => (x) => String(x).toLowerCase(),
  // `bk-tree`'s distance function: a real metric, cheap to mirror exactly on
  // the Rust side (`(a - b).abs()`), and dense over a narrow item range so
  // repeated `add`s collide on distance constantly -- see
  // `difffuzz::modules::bk_tree`'s docs.
  bkAbsDiff: () => (a, b) => Math.abs(a - b),
  // ---- T2: comparator callbacks -----------------------------------------
  //
  // Appended at the end of the table, never in the middle.
  //
  // A comparator is called FROM INSIDE a sift, once per comparison, so these
  // are the only factories whose behaviour depends on *when* they run. Each
  // carries a budget counted in comparisons, which makes them deterministic
  // only if the port performs exactly the same comparisons in exactly the same
  // order -- so the budget is itself part of what is being compared.
  //
  // `instance` is read lazily on purpose: it is assigned after `new Ctor(...)`
  // returns, and no constructor in this tier invokes its comparator.
  ascending: () => heapAscending,
  descending: () => (a, b) => {
    if (a < b) return 1;
    if (a > b) return -1;
    return 0;
  },
  // Grows the array the sift is walking. The sift wrote `heap[i] = item` at an
  // index it chose before the array changed length.
  pushy: () => {
    let budget = 3;
    return (a, b) => {
      if (budget-- > 0) instance.items.push(99);
      return heapAscending(a, b);
    };
  },
  // Shrinks it, so the walk reads past its own frozen `endIndex` and gets
  // `undefined` -- and then writes it back somewhere else.
  popper: () => {
    let budget = 2;
    return (a, b) => {
      if (budget-- > 0) instance.items.pop();
      return heapAscending(a, b);
    };
  },
  // REBINDS it. `Heap.prototype.clear` installs a new array, so the sift
  // finishes into one nothing can reach. This is DIV-STACK-3 reached from inside a
  // comparator rather than from a cursor.
  clearer: () => {
    let budget = 1;
    return (a, b) => {
      if (budget-- > 0) instance.clear();
      return heapAscending(a, b);
    };
  },
  // Throws mid-sift. Upstream has no try/finally, so `push` has already grown
  // the array while `++this.size` never runs.
  boom: () => {
    let budget = 5;
    return (a, b) => {
      if (budget-- <= 0) throw new Error('boom');
      return heapAscending(a, b);
    };
  },
  // Appended at the end, never inserted (CLAUDE.md, Git): a new key anywhere
  // else is a merge conflict inside an object literal. `trie-map.update`'s
  // and `trie.update`'s factories -- both trivial, deterministic, and named
  // rather than transmitted as source for the same reason every factory
  // above is (see `FACTORIES`' own docs).
  trieIncrement: () => (old) => (typeof old === 'number' ? old : 0) + 1,
  // `has` is presence-based (`SENTINEL in node`), not a truthiness check, so
  // toggling a `Trie` node's value to `false` still leaves it a stored word
  // -- this factory is what lets `difffuzz::modules::trie` reach that state
  // on both sides.
  trieToggle: () => (old) => !old,
  // Appended at the end, never inserted (CLAUDE.md, Git): a new key anywhere
  // else is a merge conflict inside an object literal.
  //
  // `fibonacci-heap` reuses `ascending`/`descending`/`boom` above verbatim --
  // all three are already generic over any instance, since none of them
  // touch `.items`. It needs its OWN mutating factories because
  // `FibonacciHeap` has no public backing array at all: `push`, `peek`,
  // `pop` and `clear` are its entire surface, so the re-entrant comparators
  // reach through those instead of `instance.items.push`/`.pop()`.
  //
  // Grows the tree count under a sift that has already chosen which nodes to
  // compare.
  fibPushy: () => {
    let budget = 3;
    return (a, b) => {
      if (budget-- > 0) instance.push(99);
      return heapAscending(a, b);
    };
  },
  // A NESTED pop from inside another pop's `consolidate` -- legitimate
  // re-entrancy `FibonacciHeap` (unlike `Heap`) has no backing array to
  // shrink instead, so this is the shape that exercises it here.
  fibPopper: () => {
    let budget = 2;
    return (a, b) => {
      if (budget-- > 0) instance.pop();
      return heapAscending(a, b);
    };
  },
  // Resets `root`/`min`/`size` out from under the `consolidate` call
  // currently comparing against them -- NOTES.md BUG-FIBONACCI-HEAP-1's trigger.
  fibClearer: () => {
    let budget = 1;
    return (a, b) => {
      if (budget-- > 0) instance.clear();
      return heapAscending(a, b);
    };
  },
  // Appended at the end, never inserted (CLAUDE.md, Git): a new key anywhere
  // else is a merge conflict inside an object literal.
  //
  // `passjoin-index`'s `levenshtein` ctor argument. The real `leven` package
  // (a harness dependency, `tests/harness-package.json`) rather than a
  // hand-rolled metric: `PassjoinIndex` is only ever fuzzed together with the
  // exact distance function `test/passjoin-index.js` itself uses, so there
  // is no simplification to make here the way `bkAbsDiff` makes for
  // `bk-tree`'s arbitrary caller-supplied metric.
  pjLeven: () => require('leven'),
};

// Upstream's DEFAULT_COMPARATOR, written out so the factories above can wrap it.
function heapAscending(a, b) {
  if (a < b) return -1;
  if (a > b) return 1;
  return 0;
}

// A fixed pool of real objects, created ONCE and held here for the whole
// life of this process -- `default-weak-map`'s key identity. A JSON value
// cannot carry object identity, so a key travels as `{"$weakKey": n}` and
// `decode` resolves it against pool slot `n`. Held by a plain module-level
// array (a strong reference) so that none of these objects is EVER eligible
// for collection during a campaign: `default-weak-map`'s own module docs
// (`mnemonist_core::structures::default_weak_map`, and
// `crates/difffuzz/src/modules/default_weak_map.rs`) are explicit that GC
// timing is not observable through this module's API at all, so a campaign
// that let a key die mid-run would be measuring an apparatus accident, not
// the port. Eight slots, matching the Rust side's `KEY_POOL`.
const WEAK_KEY_POOL = Array.from({length: 8}, () => ({}));

function decode(value) {
  if (Array.isArray(value)) return value.map(decode);
  if (value === null || typeof value !== 'object') return value;

  if (value.$undefined) return undefined;
  if (value.$nan) return NaN;
  if (value.$negativeZero) return -0;
  if (typeof value.$weakKey === 'number') return WEAK_KEY_POOL[value.$weakKey];

  // The inverses of `encode`'s `$set` and `$typed`, needed because a
  // free-function module takes these as ARGUMENTS rather than building them
  // itself: `set.js` is handed real Sets and `sort/*.js` real typed arrays.
  if (Array.isArray(value.$set)) return new Set(value.$set.map(decode));

  if (typeof value.$typed === 'string') {
    const Ctor = globalThis[value.$typed];

    if (typeof Ctor !== 'function') {
      throw new Error('unknown typed array: ' + value.$typed);
    }

    return new Ctor(value.values);
  }

  if (typeof value.$factory === 'string') {
    const name = value.$factory;
    if (!Object.prototype.hasOwnProperty.call(FACTORIES, name)) {
      throw new Error('unknown factory: ' + name);
    }
    return FACTORIES[name]();
  }

  return value;
}

function handle(request) {
  switch (request.cmd) {
    case 'ping':
      return {ok: true};

    case 'init': {
      observations = request.observe;
      cursor = null;

      // Free-function modules (DESIGN.md 1.1's `sort` and `set` units). They
      // have no constructor at all -- `set.js` and `sort/*.js` export bare
      // functions -- so there is nothing to `new`, and `instance` becomes the
      // module object itself so that `instance[name](...)` still dispatches.
      //
      // A list rather than a name because a unit can span several files:
      // test/sort.js's require-closure is three of them, and the log key for
      // the whole unit is `sort`, which is not a file. Merging their exports is
      // safe because upstream's own names do not collide.
      if (Array.isArray(request.functions)) {
        functionsModule = true;
        instance = Object.assign(
          {},
          ...request.functions.map((file) => require(path.join(UPSTREAM, file + '.js')))
        );

        return {ok: true, state: observe()};
      }

      functionsModule = false;

      const Ctor = require(path.join(UPSTREAM, request.module + '.js'));
      // Two decoders, composed, because they solve different problems and
      // neither subsumes the other: `decodeCtorArg` resolves {"$global": …} to
      // a real constructor (SparseMap/HashedArrayTree take one), while `decode`
      // rebuilds the values JSON cannot carry -- NaN, -0, undefined, factories.
      // `decodeCtorArg` passes non-$global values through untouched and `decode`
      // returns a function unchanged, so the order is safe either way.
      const ctorArgs = request.ctor.map((arg) => decode(decodeCtorArg(arg)));
      // `kd-tree.js`'s own raw constructor, `function KDTree(dimensions,
      // build)`, takes an already-built internal shape only `.from`/
      // `.fromAxes` themselves ever produce -- there is no directly usable
      // `new KDTree(...)` the way every other fuzzed module has one.
      // `staticFactory` names one of the constructor's own static methods to
      // call instead of `new Ctor(...)`, so this module's real entry point is
      // exercised rather than a synthetic re-derivation of `build`'s shape.
      // Optional and additive: every other module's `init` omits it and gets
      // the same `new Ctor(...)` behaviour as before.
      instance = request.staticFactory
        ? Ctor[request.staticFactory](...ctorArgs)
        : new Ctor(...ctorArgs);
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
      // A free-function module has no observable state at all, so `observe()`
      // is `{}` for every op and the whole comparison would rest on the return
      // value. That is not enough: `set.js`'s `add`, `subtract`, `intersect`
      // and `disjunct` return `undefined` and do all their work by mutating
      // their FIRST ARGUMENT, and `sort/*.js` sorts in place. So the decoded
      // arguments are re-encoded after the call and compared too. Generic
      // rather than per-function: the oracle holds no module knowledge, and a
      // list of which parameters are out-parameters would be exactly that.
      const args = request.args.map(decode);

      try {
        result = encode(instance[request.name](...args));
      } catch (error) {
        result = {$throw: String(error && error.message ? error.message : error)};
      }

      if (functionsModule) {
        result = {$return: result, $args: args.map(encode)};
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

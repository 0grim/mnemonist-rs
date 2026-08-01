//! The parameterisation point: what a module has to declare to be fuzzable,
//! and the generic op-by-op comparison built on top of it.

use std::fmt;

use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;
use serde_json::{json, Value};

/// One operation in a generated program.
///
/// `name` is `&'static str` on purpose — the op alphabet is fixed per module,
/// so there is nothing for proptest to shrink here and nothing to allocate.
/// Everything shrinkable lives in `args`, which the module's strategy builds
/// out of ordinary shrinkable primitives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    pub name: &'static str,
    pub args: Vec<Value>,
}

impl Op {
    pub fn new(name: &'static str, args: Vec<Value>) -> Self {
        Self { name, args }
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let args: Vec<String> = self.args.iter().map(ToString::to_string).collect();

        write!(f, "{}({})", self.name, args.join(", "))
    }
}

/// The `$forEach` op: walk the collection, and mutate it from inside the walk.
///
/// # Why this op exists
///
/// B-31 was reachable only through a `forEach` whose callback mutated the
/// collection, and no module's alphabet had a `forEach` at all — so 2.94M
/// generated operations could not express the program that breaks it. An
/// alphabet that omits a method omits every bug reachable only through it, and
/// a clean campaign then reads as coverage it never had.
///
/// # What it does and does not reach
///
/// It compares the **loop shape**: which bound is live and which is frozen,
/// and what the walk sees after the collection moves under it. Upstream is
/// inconsistent about this on purpose — `SparseSet.forEach` re-reads
/// `this.size` every iteration while `SparseQueueSet.forEach` captures it —
/// and getting one of them wrong is exactly the kind of thing a hand-written
/// loop does.
///
/// It does **not** reach B-31 itself. The differential fuzzer compares
/// `mnemonist-core` against upstream JS; the napi bridge, where the hoisted
/// read lived, is not in that loop at all. Those specs are
/// `tests/boundary/reentrancy.js`, which needs the real addon and a real JS
/// callback. Stated here rather than left to be assumed, because "we added a
/// forEach op" would otherwise read as "B-31 is now fuzz-covered".
#[derive(Debug, Clone, Copy)]
pub struct ForEach<'a> {
    /// Method the callback calls back into, or `None` for a plain walk.
    pub method: Option<&'a str>,
    /// How that method's arguments are built from the callback's own.
    pub rule: &'a str,
    /// How many times the mutation may fire, counted from the first step.
    pub limit: usize,
}

/// Stand-in for "every step", small enough to stay a plain JSON integer.
pub const FOR_EACH_MANY: u64 = 1_000_000;

/// Read a `$forEach` op's arguments.
pub fn for_each(op: &Op) -> ForEach<'_> {
    ForEach {
        method: op.args[0].as_str(),
        rule: op.args[1].as_str().unwrap_or("none"),
        limit: op.args[2].as_u64().unwrap_or(0) as usize,
    }
}

/// The mutating call's arguments, or `None` when the op must not fire.
///
/// `None` covers two cases that are deliberately identical: no method at all,
/// and a rule that selected an `undefined`.
///
/// # The one narrowing, and why
///
/// A callback argument can be `undefined` — `dense[i]` past the end of a
/// corrupted `SparseSet`, `items[i]` after a `pop`. Passing it on to the
/// mutating method is legal JavaScript and does something specific and awful:
/// `this.sparse[undefined]` is `undefined`, `undefined >= size` is false, and
/// upstream falls through into a swap indexed by `undefined`, which on a typed
/// array is a silently discarded expando write plus a `size--`. `usize` cannot
/// express that and `mnemonist-core` does not model it, so the mutation is
/// **skipped on both sides** rather than guessed at. Disclosed in
/// `fuzz/log.txt`; the plain ops still generate every out-of-range member.
pub fn for_each_args<'v>(spec: &ForEach<'_>, received: &'v [Value]) -> Option<Vec<&'v Value>> {
    // A plain walk has no method, and therefore nothing to fire.
    spec.method?;

    let selected: Vec<&Value> = match spec.rule {
        "none" => Vec::new(),
        "arg0" | "arg0+1" => vec![&received[0]],
        "arg1" => vec![&received[1]],
        "arg1,arg0" => vec![&received[1], &received[0]],
        other => panic!("`{other}` is not a $forEach argument rule"),
    };

    if selected.iter().any(|value| is_undefined(value)) {
        return None;
    }

    Some(selected)
}

/// `{"$undefined": true}`, which is how the oracle spells a value JSON has no
/// word for.
pub fn is_undefined(value: &Value) -> bool {
    value
        .get("$undefined")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// A `$forEach` argument as the non-negative integer the core methods take.
///
/// `+1` is applied here rather than by the caller so the Rust side and
/// `fuzz/oracle.js` implement the rule in exactly one place each.
pub fn for_each_index(spec: &ForEach<'_>, value: &Value) -> usize {
    let number = value.as_u64().expect("a $forEach index is a JSON integer") as usize;

    match spec.rule {
        "arg0+1" => number + 1,
        _ => number,
    }
}

/// The `$forEach` alphabet for one module.
///
/// `mutations` is a table of `(method, rule, many)`: the method the callback
/// calls, how its arguments come out of the callback's own, and the "fires on
/// every step" limit for that mutation. The limit is per row because a
/// mutation that *grows* the collection has to be bounded — for a module whose
/// `forEach` bound is live, firing a growth on every step does not terminate,
/// upstream included.
///
/// A plain walk is always generated alongside them, because a `forEach` that
/// mutates nothing still compares the callback arguments and their order.
pub fn for_each_strategy(
    mutations: &'static [(&'static str, &'static str, u64)],
) -> BoxedStrategy<Op> {
    (0..=mutations.len(), 0usize..2)
        .prop_map(move |(choice, repeat)| {
            if choice == mutations.len() {
                return Op::new("$forEach", vec![Value::Null, json!("none"), json!(0)]);
            }

            let (method, rule, many) = mutations[choice];
            // Both limits matter and neither subsumes the other: "mutate once,
            // then keep walking" is the classic re-entrancy shape, and "mutate
            // on every step" is the one that races the loop bound.
            let limit = if repeat == 0 { 1 } else { many };

            Op::new("$forEach", vec![json!(method), json!(rule), json!(limit)])
        })
        .boxed()
}

/// A complete generated test case: how to build the instance, then what to do
/// to it. This is the value proptest generates and, on failure, shrinks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub ctor: Vec<Value>,
    pub ops: Vec<Op>,
}

impl Program {
    /// Render as pasteable JS. The whole value of shrinking is that the result
    /// is small enough to drop straight into an upstream issue, so it is worth
    /// emitting something that runs rather than a `Debug` dump.
    pub fn render(&self, module: &str) -> String {
        // Module keys are kebab-case upstream filenames; the exported
        // constructor is their PascalCase form.
        let constructor: String = module
            .split('-')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                }
            })
            .collect();

        // `{"$global": "Uint8Array"}` is how a constructor argument travels
        // over JSON (see `fuzz/oracle.js`). Rendering it literally would give a
        // repro that constructs a plain object where upstream wants
        // `Uint8Array`, so it is unwrapped back to the identifier it stands for.
        let ctor: Vec<String> = self
            .ctor
            .iter()
            .map(|arg| match arg.get("$global").and_then(Value::as_str) {
                Some(name) => name.to_owned(),
                None => arg.to_string(),
            })
            .collect();
        let mut out = format!("var s = new {constructor}({});\n", ctor.join(", "));

        for op in &self.ops {
            // The `$` ops are protocol, not methods (see `fuzz/oracle.js`), so
            // rendering them as `s.$next()` would produce a repro that throws
            // instead of reproducing. Since the whole value of shrinking is a
            // case small enough to paste into an upstream issue, they are
            // rendered as the JS they stand for.
            match op.name {
                "$iter" => {
                    let factory = op.args.first().and_then(Value::as_str).unwrap_or("values");

                    out.push_str(&format!("var it = s.{factory}();\n"));
                }
                "$next" => out.push_str("it.next();\n"),
                "$spread" => out.push_str("Array.from(s);\n"),
                "$forEach" => out.push_str(&render_for_each(op)),
                _ => out.push_str(&format!("s.{op};\n")),
            }
        }

        out
    }
}

/// A `$forEach` op as the JavaScript it stands for.
///
/// Rendered rather than dumped for the same reason the other `$` ops are: the
/// point of shrinking is a case small enough to paste into an upstream issue,
/// and `s.$forEach("delete", "arg0", 1)` would not run.
fn render_for_each(op: &Op) -> String {
    let spec = for_each(op);
    let Some(method) = spec.method else {
        return String::from("s.forEach(function (a, b) {});\n");
    };

    let call = match spec.rule {
        "none" => format!("s.{method}()"),
        "arg0" => format!("s.{method}(a)"),
        "arg0+1" => format!("s.{method}(a + 1)"),
        "arg1" => format!("s.{method}(b)"),
        "arg1,arg0" => format!("s.{method}(b, a)"),
        other => panic!("`{other}` is not a $forEach argument rule"),
    };
    // The guard mirrors `for_each_args`' skip, over exactly the arguments the
    // rule selects — see the narrowing note there.
    let guard = match spec.rule {
        "none" => "",
        "arg0" | "arg0+1" => "if (a === undefined) return; ",
        "arg1" => "if (b === undefined) return; ",
        _ => "if (a === undefined || b === undefined) return; ",
    };

    format!(
        "var fired = 0;\ns.forEach(function (a, b) {{ \
         {guard}if (fired++ < {}) {call}; }});\n",
        spec.limit
    )
}

/// Everything the generic driver needs to fuzz one module.
///
/// Implementations are pure glue: build the Rust instance, apply an op to it,
/// and render the same observable state the oracle renders. Encoding rules
/// live in `fuzz/oracle.js`; whatever it emits for a value, [`ModuleSpec`] must
/// emit for the equivalent Rust value, or every run is a false divergence.
pub trait ModuleSpec {
    /// The Rust instance under test.
    type Instance;

    /// Upstream file stem under `bench/upstream/`, and the key used in
    /// `fuzz/log.txt` and `bench/results.json`.
    fn module(&self) -> &'static str;

    /// Properties and nullary methods that together define observable state.
    ///
    /// Sent verbatim to the oracle, so the two sides cannot drift apart.
    fn observations(&self) -> &'static [&'static str];

    /// Generates constructor arguments.
    fn ctor_strategy(&self) -> BoxedStrategy<Vec<Value>>;

    /// Generates an operation valid for an instance built with `ctor`.
    fn op_strategy(&self, ctor: &[Value]) -> BoxedStrategy<Op>;

    /// How many operations a generated program holds.
    ///
    /// Long enough that ops interact, short enough that the round trip per op
    /// still buys a useful case rate; proptest shrinks towards the low end.
    fn program_len(&self) -> std::ops::Range<usize> {
        1..200
    }

    /// Build the Rust instance. Called after `ctor_strategy` produced `args`.
    fn construct(&self, args: &[Value]) -> Self::Instance;

    /// Apply one op, returning its encoded result value.
    fn apply(&self, instance: &mut Self::Instance, op: &Op) -> Value;

    /// Render the declared observable state.
    ///
    /// Takes `&mut` because several upstream "read" methods mutate — `mapping`
    /// and `compile` both drive path compression — and reproducing that
    /// faithfully is part of the point.
    fn observe(&self, instance: &mut Self::Instance) -> Value;
}

/// A concrete disagreement between the port and upstream.
#[derive(Debug, Clone)]
pub struct Divergence {
    /// Module key, so the rendered repro names a real constructor.
    pub module: &'static str,
    /// Index into `program.ops`, or `None` for a disagreement present at
    /// construction time, before any operation ran.
    pub after: Option<usize>,
    /// What disagreed: an op's return value, or the observable state.
    pub kind: DivergenceKind,
    pub program: Program,
    pub port: Value,
    pub upstream: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceKind {
    Result,
    State,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            DivergenceKind::Result => "return value",
            DivergenceKind::State => "observable state",
        };

        match self.after {
            None => writeln!(f, "divergence in {what} at construction")?,
            Some(index) => writeln!(
                f,
                "divergence in {what} after op #{index}: {}",
                self.program.ops[index]
            )?,
        }

        for (label, port, upstream) in narrow(&self.port, &self.upstream) {
            writeln!(f, "  {label}")?;
            writeln!(f, "    port:     {}", elide(&port))?;
            writeln!(f, "    upstream: {}", elide(&upstream))?;
        }

        writeln!(f, "minimal repro:")?;
        write!(f, "{}", self.program.render(self.module))
    }
}

/// Longest value rendered in full before eliding the middle.
///
/// A `compile()` on a 348-element set is ~4 KB of JSON, and printing two of
/// them buries the one number that differs. Measured on a real sabotage run:
/// the untruncated message was 8 KB and the actual difference was the string
/// `Uint8Array` vs `Uint16Array`.
const MAX_RENDERED: usize = 240;

/// Reduce a whole-state disagreement to the observations that actually differ.
///
/// Both sides are the same shape by construction, so when they are objects the
/// interesting part is the differing keys. Falls back to the whole value for
/// scalars and for the return-value case.
fn narrow(port: &Value, upstream: &Value) -> Vec<(String, String, String)> {
    match (port.as_object(), upstream.as_object()) {
        (Some(left), Some(right)) => {
            let mut differing: Vec<(String, String, String)> = left
                .iter()
                .filter(|(key, value)| right.get(*key) != Some(*value))
                .map(|(key, value)| {
                    let other = right
                        .get(key)
                        .map_or_else(|| "<absent>".into(), Value::to_string);

                    (format!("{key}:"), value.to_string(), other)
                })
                .collect();

            // Keys present upstream but not in the port would otherwise vanish.
            differing.extend(
                right
                    .iter()
                    .filter(|(key, _)| !left.contains_key(*key))
                    .map(|(key, value)| (format!("{key}:"), "<absent>".into(), value.to_string())),
            );

            differing
        }
        _ => vec![(
            String::from("value:"),
            port.to_string(),
            upstream.to_string(),
        )],
    }
}

/// Keep both ends, drop the middle. The ends are where a length change or a
/// type tag shows up; the middle of a 1,000-element array almost never is.
fn elide(rendered: &str) -> String {
    if rendered.len() <= MAX_RENDERED {
        return rendered.to_owned();
    }

    let keep = MAX_RENDERED / 2;
    let head: String = rendered.chars().take(keep).collect();
    let tail: String = rendered
        .chars()
        .rev()
        .take(keep)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();

    format!(
        "{head} … [{} chars elided] … {tail}",
        rendered.len() - head.len() - tail.len()
    )
}

/// Run one program against both implementations, comparing after every op.
///
/// Returns `Ok(op_count)` when the two agree throughout. `Err` distinguishes a
/// genuine divergence from apparatus failure so the campaign never books one
/// as the other.
pub fn check_program<S: ModuleSpec>(
    spec: &S,
    oracle: &mut crate::Oracle,
    program: &Program,
) -> Result<u64, CheckFailure> {
    let upstream_state = oracle.init(spec.module(), &program.ctor, spec.observations())?;

    let mut instance = spec.construct(&program.ctor);
    let port_state = spec.observe(&mut instance);

    if port_state != upstream_state {
        return Err(CheckFailure::Diverged(Box::new(Divergence {
            module: spec.module(),
            after: None,
            kind: DivergenceKind::State,
            program: program.clone(),
            port: port_state,
            upstream: upstream_state,
        })));
    }

    for (index, op) in program.ops.iter().enumerate() {
        let port_result = spec.apply(&mut instance, op);
        let port_state = spec.observe(&mut instance);

        let upstream = oracle.apply(op.name, &op.args)?;

        // Order matters for the write-up: a wrong return value with the right
        // state is a different class of bug from the reverse.
        if port_result != upstream.result {
            return Err(CheckFailure::Diverged(Box::new(Divergence {
                module: spec.module(),
                after: Some(index),
                kind: DivergenceKind::Result,
                program: program.clone(),
                port: port_result,
                upstream: upstream.result,
            })));
        }

        if port_state != upstream.state {
            return Err(CheckFailure::Diverged(Box::new(Divergence {
                module: spec.module(),
                after: Some(index),
                kind: DivergenceKind::State,
                program: program.clone(),
                port: port_state,
                upstream: upstream.state,
            })));
        }
    }

    Ok(program.ops.len() as u64)
}

/// Why a single program did not come back clean.
#[derive(Debug)]
pub enum CheckFailure {
    /// The port and upstream disagree. This is the finding.
    Diverged(Box<Divergence>),
    /// The harness broke. Never a finding.
    ///
    /// # Trap for the next module
    ///
    /// An exception *thrown by an operation* on the JS side currently arrives
    /// here, as [`crate::OracleError::Js`], and is therefore classified as
    /// apparatus failure rather than as a divergence. That is correct for
    /// `static-disjoint-set`, whose op alphabet cannot throw — every generated
    /// index is in range and every op name is a real method.
    ///
    /// It stops being correct for any module where throwing is legitimate
    /// behaviour, because "upstream threw and the port did not" is exactly the
    /// kind of divergence this crate exists to catch, and routing it here would
    /// abort the campaign instead of reporting it. The fix, when that module
    /// arrives: make throwing part of the compared result — encode it as
    /// `{"$throw": "<message>"}` on both sides — rather than an out-of-band
    /// error. Left undone deliberately; guessing the shape before there is a
    /// module to check it against is how it gets guessed wrong.
    Oracle(crate::OracleError),
}

impl fmt::Display for CheckFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Diverged(divergence) => write!(f, "{divergence}"),
            Self::Oracle(error) => write!(f, "{error}"),
        }
    }
}

impl From<crate::OracleError> for CheckFailure {
    fn from(error: crate::OracleError) -> Self {
        Self::Oracle(error)
    }
}

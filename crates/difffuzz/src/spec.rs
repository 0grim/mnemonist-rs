//! The parameterisation point: what a module has to declare to be fuzzable,
//! and the generic op-by-op comparison built on top of it.

use std::fmt;

use proptest::strategy::BoxedStrategy;
use serde_json::Value;

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

        let ctor: Vec<String> = self.ctor.iter().map(ToString::to_string).collect();
        let mut out = format!("var s = new {constructor}({});\n", ctor.join(", "));

        for op in &self.ops {
            out.push_str(&format!("s.{op};\n"));
        }

        out
    }
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

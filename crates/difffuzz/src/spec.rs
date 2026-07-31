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

        writeln!(f, "  port:     {}", self.port)?;
        writeln!(f, "  upstream: {}", self.upstream)?;
        writeln!(f, "minimal repro:")?;
        write!(f, "{}", self.program.render(self.module))
    }
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

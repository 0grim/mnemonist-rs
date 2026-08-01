//! The JS side of the comparison: one long-lived `node` process, addressed
//! over line-delimited JSON.
//!
//! The whole reason this type exists is the performance rule in DESIGN.md 4 —
//! spawn Node **once**. A fresh `node` costs ~30 ms to boot, so paying it per
//! operation would cap a fuzz campaign at ~30 ops/second and make the 60-second
//! target in gate 9 take the better part of a day. Reusing one process brings
//! the per-op cost down to a pipe round trip.
//!
//! The oracle is deliberately dumb: it holds no module knowledge. The module
//! name, constructor arguments and observable-state list all travel in the
//! `init` request, which is what lets one script serve every
//! [`ModuleSpec`](crate::ModuleSpec).

use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

/// Anything that can go wrong talking to the oracle.
///
/// None of these are divergences — they mean the measurement apparatus broke,
/// which must never be reported as "the port and upstream disagree".
#[derive(Debug)]
pub enum OracleError {
    /// The pipe broke, or `node` is not on `PATH`.
    Io(io::Error),
    /// The oracle wrote something that is not a JSON object.
    Protocol(String),
    /// The oracle answered `{"ok": false, ...}`.
    Js(String),
}

impl fmt::Display for OracleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "oracle I/O: {error}"),
            Self::Protocol(message) => write!(f, "oracle protocol: {message}"),
            Self::Js(message) => write!(f, "oracle threw: {message}"),
        }
    }
}

impl std::error::Error for OracleError {}

impl From<io::Error> for OracleError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// What the oracle reports back after applying one operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// The operation's own return value, encoded (see `fuzz/oracle.js`).
    pub result: Value,
    /// The declared observable state, after the operation.
    pub state: Value,
}

/// A live `node fuzz/oracle.js` subprocess.
///
/// Dropping the oracle closes its stdin, which ends the readline stream and
/// lets the child exit on its own; [`Oracle::shutdown`] does the same thing but
/// waits, and is what you want when the process count matters.
pub struct Oracle {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    line: String,
}

impl Oracle {
    /// Locate `fuzz/oracle.js` relative to this crate, so callers do not have
    /// to care what the current directory is.
    pub fn default_script() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/oracle.js")
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from("fuzz/oracle.js"))
    }

    /// Spawn the oracle and confirm it answers before returning.
    pub fn spawn(script: &Path) -> Result<Self, OracleError> {
        let mut child = Command::new("node")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| OracleError::Protocol("child has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OracleError::Protocol("child has no stdout".into()))?;

        let mut oracle = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            line: String::new(),
        };

        // Fail here rather than inside a property, where a dead oracle would
        // read as a divergence.
        oracle.request(&json!({"cmd": "ping"}))?;

        Ok(oracle)
    }

    /// Discard any current instance and build a fresh one.
    ///
    /// Called once per generated program, which is what keeps a whole campaign
    /// inside a single process.
    pub fn init(
        &mut self,
        module: &str,
        ctor: &[Value],
        observe: &[&'static str],
    ) -> Result<Value, OracleError> {
        let mut response = self.request(&json!({
            "cmd": "init",
            "module": module,
            "ctor": ctor,
            "observe": observe,
        }))?;

        Self::take(&mut response, "state")
    }

    /// As [`Oracle::init`], for a module that is a set of free functions
    /// rather than a constructor.
    ///
    /// `files` are upstream file stems under `bench/upstream/`; their exports
    /// are merged into one object on the JS side. A list rather than a name
    /// because a unit can span several files — `test/sort.js`'s
    /// require-closure is three of them, and the unit's key, `sort`, is not a
    /// file. There is no `ctor`, because there is nothing to construct.
    pub fn init_functions(
        &mut self,
        module: &str,
        files: &[&'static str],
        observe: &[&'static str],
    ) -> Result<Value, OracleError> {
        let mut response = self.request(&json!({
            "cmd": "init",
            "module": module,
            "ctor": [],
            "observe": observe,
            "functions": files,
        }))?;

        Self::take(&mut response, "state")
    }

    /// Apply one operation and read back its result plus the observable state.
    pub fn apply(&mut self, name: &str, args: &[Value]) -> Result<Observation, OracleError> {
        let mut response = self.request(&json!({
            "cmd": "op",
            "name": name,
            "args": args,
        }))?;

        let result = Self::take(&mut response, "result")?;
        let state = Self::take(&mut response, "state")?;

        Ok(Observation { result, state })
    }

    /// Ask the child to exit and wait for it.
    pub fn shutdown(mut self) -> Result<(), OracleError> {
        // A write failure here means the child is already gone, which is the
        // outcome we wanted anyway.
        let _ = writeln!(self.stdin, "{}", json!({"cmd": "quit"}));
        let _ = self.stdin.flush();
        self.child.wait()?;

        Ok(())
    }

    fn request(&mut self, request: &Value) -> Result<Value, OracleError> {
        writeln!(self.stdin, "{request}")?;
        self.stdin.flush()?;

        self.line.clear();

        if self.stdout.read_line(&mut self.line)? == 0 {
            return Err(OracleError::Protocol("oracle closed its stdout".into()));
        }

        let response: Value = serde_json::from_str(self.line.trim_end())
            .map_err(|error| OracleError::Protocol(format!("{error}: {}", self.line.trim_end())))?;

        if response.get("ok").and_then(Value::as_bool) != Some(true) {
            let message = response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unspecified")
                .to_owned();

            return Err(OracleError::Js(message));
        }

        Ok(response)
    }

    fn take(response: &mut Value, field: &str) -> Result<Value, OracleError> {
        response
            .get_mut(field)
            .map(Value::take)
            .ok_or_else(|| OracleError::Protocol(format!("response has no `{field}`")))
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        // Best effort: killing is fine, the child holds no state we need.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

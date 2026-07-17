//! Per-file command loop: fresh store + spectest registration per JSON
//! file, then every command is judged to exactly one verdict.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value as J;
use wasm_core::{decode, validate, InstError, InstanceId, InvokeError, Store, Value};

use crate::convert::{match_expected, parse_value, str_field};
use crate::spectest::spectest_module;

pub struct Row {
    pub line: u64,
    pub cmd_type: String,
    pub verdict: &'static str,
    pub reason: String,
}

pub struct FileResult {
    pub file: String,
    pub total: u64,
    pub pass: u64,
    pub fail: u64,
    pub unsupported: u64,
    pub rows: Vec<Row>,
}

enum Verdict {
    Pass,
    Fail(String),
    Unsupported(String),
}

struct Ctx {
    store: Store,
    /// Registered import namespaces: module name -> instance.
    registry: HashMap<String, InstanceId>,
    /// Named module instances ($id from the wast source).
    named: HashMap<String, InstanceId>,
    current: Option<InstanceId>,
    dir: std::path::PathBuf,
}

impl Ctx {
    fn resolve_instance(&self, cmd: &J) -> Result<InstanceId, String> {
        match cmd.get("module") {
            None => self.current.ok_or_else(|| "no current module".to_string()),
            Some(J::String(name)) => self
                .named
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown module name {name}")),
            Some(other) => Err(format!("bad module field: {other}")),
        }
    }
}

pub fn run_file(path: &Path) -> Result<FileResult, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let doc: J = serde_json::from_str(&text).map_err(|e| format!("parse: {e}"))?;
    let commands = doc
        .get("commands")
        .and_then(|c| c.as_array())
        .ok_or("missing commands array")?;

    let mut store = Store::new();
    let spectest = store.add_host_module(spectest_module());
    let mut ctx = Ctx {
        store,
        registry: HashMap::from([("spectest".to_string(), spectest)]),
        named: HashMap::new(),
        current: None,
        dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
    };

    let mut res = FileResult {
        file: path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        total: 0,
        pass: 0,
        fail: 0,
        unsupported: 0,
        rows: Vec::new(),
    };

    for cmd in commands {
        let line = cmd.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
        let cmd_type = cmd
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("<missing>")
            .to_string();
        let verdict = run_command(&mut ctx, &cmd_type, cmd);
        res.total += 1;
        let (v, reason) = match verdict {
            Verdict::Pass => {
                res.pass += 1;
                ("PASS", String::new())
            }
            Verdict::Fail(r) => {
                res.fail += 1;
                ("FAIL", r)
            }
            Verdict::Unsupported(r) => {
                res.unsupported += 1;
                ("UNSUPPORTED", r)
            }
        };
        res.rows.push(Row {
            line,
            cmd_type,
            verdict: v,
            reason,
        });
    }
    Ok(res)
}

fn run_command(ctx: &mut Ctx, cmd_type: &str, cmd: &J) -> Verdict {
    match cmd_type {
        "module" => cmd_module(ctx, cmd),
        "register" => cmd_register(ctx, cmd),
        "action" => match perform_action(ctx, cmd) {
            Ok(_) => Verdict::Pass,
            Err(e) => Verdict::Fail(format!("action failed: {e}")),
        },
        "assert_return" => cmd_assert_return(ctx, cmd),
        "assert_trap" => cmd_assert_trap(ctx, cmd),
        "assert_exhaustion" => cmd_assert_trap(ctx, cmd),
        "assert_malformed" => cmd_assert_malformed(ctx, cmd),
        "assert_invalid" => cmd_assert_invalid(ctx, cmd),
        "assert_unlinkable" => cmd_assert_unlinkable(ctx, cmd),
        "assert_uninstantiable" => cmd_assert_uninstantiable(ctx, cmd),
        other => Verdict::Fail(format!("unknown command type {other:?} (fail-closed)")),
    }
}

fn read_module_bytes(ctx: &Ctx, cmd: &J) -> Result<Vec<u8>, String> {
    let filename = str_field(cmd, "filename")?;
    std::fs::read(ctx.dir.join(filename)).map_err(|e| format!("read {filename}: {e}"))
}

/// Decode + validate + instantiate, resolving imports from the registry.
/// Import resolution works on a pre-built snapshot of the registered
/// instances' export maps, so the store is only borrowed once.
fn build_instance(ctx: &mut Ctx, bytes: &[u8]) -> Result<InstanceId, InstOutcome> {
    let module = decode(bytes).map_err(|e| InstOutcome::Malformed(e.to_string()))?;
    validate(&module).map_err(|e| InstOutcome::Invalid(e.to_string()))?;
    let mut snapshot: HashMap<(String, String), wasm_core::ExternVal> = HashMap::new();
    for (mname, inst) in &ctx.registry {
        for (ename, ev) in ctx.store.exports(*inst) {
            snapshot.insert((mname.clone(), ename.clone()), *ev);
        }
    }
    let mut resolve = |m: &str, n: &str| snapshot.get(&(m.to_string(), n.to_string())).copied();
    match ctx.store.instantiate(&module, &mut resolve) {
        Ok(id) => Ok(id),
        Err(InstError::Link(m)) => Err(InstOutcome::Unlinkable(m)),
        Err(InstError::Trap(t)) => Err(InstOutcome::Uninstantiable(t.msg)),
    }
}

enum InstOutcome {
    Malformed(String),
    Invalid(String),
    Unlinkable(String),
    Uninstantiable(String),
}

impl InstOutcome {
    fn describe(&self) -> String {
        match self {
            InstOutcome::Malformed(m) => format!("malformed: {m}"),
            InstOutcome::Invalid(m) => format!("invalid: {m}"),
            InstOutcome::Unlinkable(m) => format!("unlinkable: {m}"),
            InstOutcome::Uninstantiable(m) => format!("uninstantiable: {m}"),
        }
    }
}

fn cmd_module(ctx: &mut Ctx, cmd: &J) -> Verdict {
    let bytes = match read_module_bytes(ctx, cmd) {
        Ok(b) => b,
        Err(e) => return Verdict::Fail(e),
    };
    match build_instance(ctx, &bytes) {
        Ok(id) => {
            ctx.current = Some(id);
            if let Some(J::String(name)) = cmd.get("name") {
                ctx.named.insert(name.clone(), id);
            }
            Verdict::Pass
        }
        Err(o) => {
            // Judging invariant: a failed module must invalidate the
            // current instance so later actions cannot silently hit the
            // previous module and produce cascading false verdicts.
            ctx.current = None;
            Verdict::Fail(format!("module failed: {}", o.describe()))
        }
    }
}

fn cmd_register(ctx: &mut Ctx, cmd: &J) -> Verdict {
    let as_name = match str_field(cmd, "as") {
        Ok(s) => s.to_string(),
        Err(e) => return Verdict::Fail(e),
    };
    match ctx.resolve_instance(cmd) {
        Ok(inst) => {
            ctx.registry.insert(as_name, inst);
            Verdict::Pass
        }
        Err(e) => Verdict::Fail(format!("register: {e}")),
    }
}

fn perform_action(ctx: &mut Ctx, cmd: &J) -> Result<Vec<Value>, String> {
    let action = cmd.get("action").ok_or("missing action")?;
    let ty = str_field(action, "type")?;
    // Fail-closed ordering: reject unknown action shapes before consulting
    // any instance state, so they can never be masked by other errors.
    if !matches!(ty, "invoke" | "get") {
        return Err(format!("unknown action type {ty:?} (fail-closed)"));
    }
    let field = str_field(action, "field")?;
    let mut args = Vec::new();
    if let Some(arr) = action.get("args").and_then(|a| a.as_array()) {
        for a in arr {
            args.push(parse_value(a)?);
        }
    }
    let inst = ctx.resolve_instance(action)?;
    match ty {
        "invoke" => ctx
            .store
            .invoke_export(inst, field, &args)
            .map_err(|e| invoke_err(&e)),
        _ => ctx
            .store
            .get_global_export(inst, field)
            .map(|v| vec![v])
            .map_err(|e| invoke_err(&e)),
    }
}

fn invoke_err(e: &InvokeError) -> String {
    match e {
        InvokeError::Trap(t) => format!("trap: {}", t.msg),
        other => other.to_string(),
    }
}

fn cmd_assert_return(ctx: &mut Ctx, cmd: &J) -> Verdict {
    let expected = match cmd.get("expected").and_then(|e| e.as_array()) {
        Some(e) => e,
        None => return Verdict::Fail("missing expected array".into()),
    };
    let actual = match perform_action(ctx, cmd) {
        Ok(v) => v,
        Err(e) => return Verdict::Fail(format!("action failed: {e}")),
    };
    if actual.len() != expected.len() {
        return Verdict::Fail(format!(
            "result arity {} != expected {}",
            actual.len(),
            expected.len()
        ));
    }
    for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
        match match_expected(a, e) {
            Ok(true) => {}
            Ok(false) => {
                return Verdict::Fail(format!("result[{i}] mismatch: actual {a:?}, expected {e}"))
            }
            Err(msg) => return Verdict::Fail(format!("result[{i}]: {msg}")),
        }
    }
    Verdict::Pass
}

/// Trap-message matching: spec-canonical messages; PASS iff a trap occurred
/// and one message is a prefix of the other (covers "uninitialized element 7"
/// vs "uninitialized element").
fn trap_matches(actual: &str, expected: &str) -> bool {
    actual.starts_with(expected) || expected.starts_with(actual)
}

fn cmd_assert_trap(ctx: &mut Ctx, cmd: &J) -> Verdict {
    let expected_text = match str_field(cmd, "text") {
        Ok(t) => t.to_string(),
        Err(e) => return Verdict::Fail(e),
    };
    match perform_action(ctx, cmd) {
        Ok(v) => Verdict::Fail(format!(
            "expected trap {expected_text:?}, got results {v:?}"
        )),
        Err(e) => {
            if let Some(msg) = e.strip_prefix("trap: ") {
                if trap_matches(msg, &expected_text) {
                    Verdict::Pass
                } else {
                    Verdict::Fail(format!(
                        "trap message mismatch: actual {msg:?}, expected {expected_text:?}"
                    ))
                }
            } else {
                Verdict::Fail(format!("expected trap, got non-trap error: {e}"))
            }
        }
    }
}

fn cmd_assert_malformed(ctx: &mut Ctx, cmd: &J) -> Verdict {
    match cmd.get("module_type").and_then(|t| t.as_str()) {
        Some("text") => {
            // Frozen boundary: text-format parse-error tests target the text
            // frontend (wast2json), not the binary engine under test.
            Verdict::Unsupported("text-format module (method boundary)".into())
        }
        Some("binary") => {
            let bytes = match read_module_bytes(ctx, cmd) {
                Ok(b) => b,
                Err(e) => return Verdict::Fail(e),
            };
            match decode(&bytes) {
                Err(_) => Verdict::Pass,
                Ok(_) => Verdict::Fail("malformed module was decoded successfully".into()),
            }
        }
        other => Verdict::Fail(format!("unknown module_type {other:?} (fail-closed)")),
    }
}

fn cmd_assert_invalid(ctx: &mut Ctx, cmd: &J) -> Verdict {
    let bytes = match read_module_bytes(ctx, cmd) {
        Ok(b) => b,
        Err(e) => return Verdict::Fail(e),
    };
    match decode(&bytes) {
        Err(e) => Verdict::Fail(format!(
            "invalid-module test failed at decode stage (must decode, then fail validation): {e}"
        )),
        Ok(m) => match validate(&m) {
            Err(_) => Verdict::Pass,
            Ok(()) => Verdict::Fail("invalid module passed validation".into()),
        },
    }
}

fn cmd_assert_unlinkable(ctx: &mut Ctx, cmd: &J) -> Verdict {
    let bytes = match read_module_bytes(ctx, cmd) {
        Ok(b) => b,
        Err(e) => return Verdict::Fail(e),
    };
    match build_instance(ctx, &bytes) {
        Err(InstOutcome::Unlinkable(_)) => Verdict::Pass,
        Err(o) => Verdict::Fail(format!("expected link error, got {}", o.describe())),
        Ok(_) => Verdict::Fail("expected link error, module instantiated".into()),
    }
}

fn cmd_assert_uninstantiable(ctx: &mut Ctx, cmd: &J) -> Verdict {
    let bytes = match read_module_bytes(ctx, cmd) {
        Ok(b) => b,
        Err(e) => return Verdict::Fail(e),
    };
    match build_instance(ctx, &bytes) {
        Err(InstOutcome::Uninstantiable(_)) => Verdict::Pass,
        Err(o) => Verdict::Fail(format!("expected instantiation trap, got {}", o.describe())),
        Ok(_) => Verdict::Fail("expected instantiation trap, module instantiated".into()),
    }
}

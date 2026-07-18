//! spec-runner: drives the wast2json-converted official test/core corpus
//! against wasm-core and produces a fail-closed ledger.
//!
//! Accounting rules (frozen in goal-runs/wasm-zero-oneshot/contract.md):
//! - every command gets exactly one verdict in {PASS, FAIL, UNSUPPORTED};
//! - UNSUPPORTED is allowed only for `module_type == "text"` commands
//!   (text-format method boundary); everything else that cannot be driven
//!   or produces an unexpected outcome is FAIL;
//! - unknown command types, unknown action types, unknown value shapes are
//!   FAIL (fail-closed), never skipped.

mod convert;
mod runfile;
mod spectest;

use std::path::PathBuf;
use std::process::ExitCode;

use runfile::{run_file, FileResult};

struct Args {
    dir: PathBuf,
    ledger_out: Option<PathBuf>,
    only: Vec<String>,
    expect_total: Option<u64>,
    expect_unsupported: Option<u64>,
    verbose_fail: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        dir: PathBuf::from("build/wast-json"),
        ledger_out: None,
        only: Vec::new(),
        expect_total: None,
        expect_unsupported: None,
        verbose_fail: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dir" => args.dir = PathBuf::from(it.next().ok_or("--dir needs a value")?),
            "--ledger-out" => {
                args.ledger_out = Some(PathBuf::from(
                    it.next().ok_or("--ledger-out needs a value")?,
                ))
            }
            "--only" => args.only.push(it.next().ok_or("--only needs a value")?),
            "--expect-total" => {
                args.expect_total = Some(
                    it.next()
                        .ok_or("--expect-total needs a value")?
                        .parse()
                        .map_err(|e| format!("bad --expect-total: {e}"))?,
                )
            }
            "--expect-unsupported" => {
                args.expect_unsupported = Some(
                    it.next()
                        .ok_or("--expect-unsupported needs a value")?
                        .parse()
                        .map_err(|e| format!("bad --expect-unsupported: {e}"))?,
                )
            }
            "--verbose-fail" => args.verbose_fail = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let mut files: Vec<PathBuf> = match std::fs::read_dir(&args.dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect(),
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", args.dir.display());
            return ExitCode::from(2);
        }
    };
    files.sort();
    if files.is_empty() {
        // An empty corpus must never look like a green sweep.
        eprintln!(
            "error: no corpus JSON files found under {}",
            args.dir.display()
        );
        return ExitCode::from(2);
    }
    if !args.only.is_empty() {
        files.retain(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            args.only
                .iter()
                .any(|o| name.as_ref() == o || name.trim_end_matches(".json") == o)
        });
        if files.is_empty() {
            eprintln!("error: --only matched no files");
            return ExitCode::from(2);
        }
    }

    let mut results: Vec<FileResult> = Vec::new();
    for f in &files {
        match run_file(f) {
            Ok(r) => results.push(r),
            Err(e) => {
                // A file-level harness failure must not be silently absorbed.
                eprintln!("harness error in {}: {e}", f.display());
                return ExitCode::from(2);
            }
        }
    }

    let (mut pass, mut fail, mut unsupported, mut total) = (0u64, 0u64, 0u64, 0u64);
    for r in &results {
        pass += r.pass;
        fail += r.fail;
        unsupported += r.unsupported;
        total += r.total;
        if r.fail > 0 {
            println!(
                "FILE {:40} pass={:5} fail={:4} unsupported={:4}",
                r.file, r.pass, r.fail, r.unsupported
            );
            if args.verbose_fail {
                for row in &r.rows {
                    if row.verdict == "FAIL" {
                        println!(
                            "  FAIL {}:{} [{}] {}",
                            r.file, row.line, row.cmd_type, row.reason
                        );
                    }
                }
            }
        }
    }

    // no-silent-skip accounting: verdicts must exactly cover all commands.
    let sum = pass + fail + unsupported;
    let mut accounting_ok = sum == total;

    if let Some(exp) = args.expect_total {
        if total != exp {
            eprintln!("ACCOUNTING MISMATCH: total commands {total} != expected {exp}");
            accounting_ok = false;
        }
    }
    if let Some(exp) = args.expect_unsupported {
        if unsupported != exp {
            eprintln!("ACCOUNTING MISMATCH: unsupported {unsupported} != expected {exp}");
            accounting_ok = false;
        }
    }

    println!(
        "TOTAL files={} commands={} PASS={} FAIL={} UNSUPPORTED={} accounting={}",
        results.len(),
        total,
        pass,
        fail,
        unsupported,
        if accounting_ok { "ok" } else { "MISMATCH" }
    );

    if let Some(out) = &args.ledger_out {
        let ledger = build_ledger_json(&results, pass, fail, unsupported, total, accounting_ok);
        if let Some(parent) = out.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(out, ledger) {
            eprintln!("error: cannot write ledger {}: {e}", out.display());
            return ExitCode::from(2);
        }
        println!("ledger written: {}", out.display());
    }

    if fail == 0 && accounting_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn build_ledger_json(
    results: &[FileResult],
    pass: u64,
    fail: u64,
    unsupported: u64,
    total: u64,
    accounting_ok: bool,
) -> String {
    use serde_json::{json, Value};
    let files: Vec<Value> = results
        .iter()
        .map(|r| {
            // PASS rows are summarized by count; FAIL/UNSUPPORTED rows are
            // itemized so every non-pass verdict is attributable.
            let rows: Vec<Value> = r
                .rows
                .iter()
                .filter(|row| row.verdict != "PASS")
                .map(|row| {
                    json!({
                        "line": row.line,
                        "type": row.cmd_type,
                        "verdict": row.verdict,
                        "reason": row.reason,
                    })
                })
                .collect();
            json!({
                "file": r.file,
                "total": r.total,
                "pass": r.pass,
                "fail": r.fail,
                "unsupported": r.unsupported,
                "non_pass_rows": rows,
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({
        "corpus": "WebAssembly/spec test/core @ wg-2.0 (fffc6e12), via wast2json (WABT 1.0.41)",
        "totals": {
            "files": results.len(),
            "commands": total,
            "pass": pass,
            "fail": fail,
            "unsupported": unsupported,
            "accounting_ok": accounting_ok,
        },
        "files": files,
    }))
    .expect("ledger serialization cannot fail")
}
